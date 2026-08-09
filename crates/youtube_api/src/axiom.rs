use crate::SearchLogEvent;
use std::{env, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle, time::timeout};

const AXIOM_QUEUE_CAPACITY: usize = 256;
const AXIOM_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub(crate) struct AxiomLogger {
    sender: mpsc::Sender<SearchLogEvent>,
}

pub(crate) struct AxiomWorker {
    task: JoinHandle<()>,
}

struct AxiomSink {
    client: reqwest::Client,
    ingest_url: reqwest::Url,
}

impl AxiomLogger {
    pub(crate) fn from_env() -> Option<(Self, AxiomWorker)> {
        // Ingest only. Never use AXIOM_API_QUERY here — that is the operator query token.
        let token = env::var("AXIOM_API_KEY")
            .or_else(|_| env::var("AXIOM_TOKEN"))
            .ok()?;
        let dataset = env::var("AXIOM_DATASET").ok()?;
        let (base_url, edge) = if let Ok(edge_url) = env::var("AXIOM_EDGE_URL") {
            (edge_url, true)
        } else if let Ok(edge) = env::var("AXIOM_EDGE") {
            (
                format!("https://{}", edge.trim().trim_end_matches('/')),
                true,
            )
        } else if let Ok(url) = env::var("AXIOM_URL") {
            (url, false)
        } else if let Ok(domain) = env::var("AXIOM_DOMAIN") {
            (
                format!(
                    "https://{}",
                    domain
                        .trim()
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .trim_end_matches('/')
                ),
                false,
            )
        } else {
            ("https://api.axiom.co".to_string(), false)
        };
        let ingest_url = match axiom_ingest_url(&base_url, &dataset, edge) {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(%error, "failed to build Axiom ingest URL");
                return None;
            }
        };
        let mut headers = reqwest::header::HeaderMap::new();
        let mut authorization =
            match reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(%error, "failed to initialize Axiom authorization");
                    return None;
                }
            };
        authorization.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        let client = match reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "failed to initialize Axiom logger");
                return None;
            }
        };

        let (sender, receiver) = mpsc::channel(AXIOM_QUEUE_CAPACITY);
        let sink = AxiomSink { client, ingest_url };
        let task = tokio::spawn(sink.run(receiver));
        Some((Self { sender }, AxiomWorker { task }))
    }

    pub(crate) fn log_search(&self, event: SearchLogEvent) {
        match self.sender.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    capacity = AXIOM_QUEUE_CAPACITY,
                    "dropping Axiom search log because the bounded queue is full"
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("dropping Axiom search log because the worker has stopped");
            }
        }
    }
}

impl AxiomWorker {
    pub(crate) async fn shutdown(mut self) {
        if timeout(AXIOM_SHUTDOWN_TIMEOUT, &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = self.task.await;
            tracing::warn!("timed out while draining the Axiom search log queue");
        }
    }
}

impl AxiomSink {
    async fn run(self, mut receiver: mpsc::Receiver<SearchLogEvent>) {
        while let Some(event) = receiver.recv().await {
            if let Err(error) = self.ingest(event).await {
                tracing::warn!(%error, "failed to ingest search query log to Axiom");
            }
        }
    }

    async fn ingest(&self, event: SearchLogEvent) -> Result<(), reqwest::Error> {
        self.client
            .post(self.ingest_url.clone())
            .json(&[event])
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map(|_| ())
    }
}

pub(crate) fn axiom_ingest_url(
    base_url: &str,
    dataset: &str,
    edge: bool,
) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url).map_err(|error| error.to_string())?;
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| "Axiom URL cannot be used as a base URL".to_string())?;
    segments.pop_if_empty();
    if edge {
        segments.extend(["v1", "ingest", dataset]);
    } else {
        segments.extend(["v1", "datasets", dataset, "ingest"]);
    }
    drop(segments);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_urls_preserve_base_paths_and_encode_dataset_names() {
        assert_eq!(
            axiom_ingest_url("https://api.axiom.co", "search events", false)
                .unwrap()
                .as_str(),
            "https://api.axiom.co/v1/datasets/search%20events/ingest"
        );
        assert_eq!(
            axiom_ingest_url("https://edge.example.com/custom/", "search/events", true)
                .unwrap()
                .as_str(),
            "https://edge.example.com/custom/v1/ingest/search%2Fevents"
        );
    }
}
