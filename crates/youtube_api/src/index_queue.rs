use crate::{ApiError, epoch_seconds};
use axum::http::{HeaderMap, header};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

static REQUEST_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub(crate) struct IndexQueue {
    pub(crate) path: PathBuf,
    token: Option<String>,
    max_items: usize,
    mutation_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct IndexQueueItem {
    pub(crate) id: String,
    pub(crate) handle: String,
    pub(crate) note: Option<String>,
    pub(crate) requested_at: u64,
    pub(crate) source_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target_backend: Option<String>,
    pub(crate) status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct IndexActivity {
    pub(crate) handle: String,
    pub(crate) started_at: u64,
    pub(crate) target_backend: String,
}

impl IndexQueue {
    pub(crate) fn new(path: PathBuf, token: Option<String>, max_items: usize) -> Self {
        Self {
            path,
            token,
            max_items: max_items.max(1),
            mutation_lock: Arc::new(Mutex::new(())),
        }
    }

    fn activity_path(&self) -> PathBuf {
        self.path.with_extension("activity.json")
    }

    pub(crate) async fn activity(&self) -> Result<Option<IndexActivity>, ApiError> {
        let path = self.activity_path();
        let activity = match tokio::fs::read_to_string(path).await {
            Ok(body) => serde_json::from_str::<IndexActivity>(&body).ok(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(ApiError::Server(error)),
        };
        Ok(activity.filter(|activity| epoch_seconds().saturating_sub(activity.started_at) <= 900))
    }

    pub(crate) async fn set_activity(
        &self,
        activity: Option<IndexActivity>,
    ) -> Result<(), ApiError> {
        let path = self.activity_path();
        match activity {
            Some(activity) => {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                let body = serde_json::to_string(&activity)
                    .map_err(|error| ApiError::Queue(error.to_string()))?;
                tokio::fs::write(path, body).await?;
            }
            None => match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ApiError::Server(error)),
            },
        }
        Ok(())
    }

    pub(crate) async fn enqueue(
        &self,
        request: IndexQueueItem,
    ) -> Result<IndexQueueItem, ApiError> {
        self.mutate(|requests| {
            if let Some(existing) = requests.iter().find(|queued| {
                queued.handle == request.handle
                    && matches!(queued.status.as_str(), "pending" | "queued" | "running")
                    && queued.target_backend == request.target_backend
            }) {
                return Ok(existing.clone());
            }
            if requests.len() >= self.max_items {
                return Err(ApiError::QueueFull);
            }
            requests.push(request.clone());
            Ok(request)
        })
        .await
    }

    pub(crate) async fn list(&self, limit: usize) -> Result<Vec<IndexQueueItem>, ApiError> {
        let body = match tokio::fs::read_to_string(&self.path).await {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(ApiError::Server(error)),
        };
        Ok(body
            .lines()
            .filter_map(|line| serde_json::from_str::<IndexQueueItem>(line).ok())
            .take(limit)
            .collect())
    }

    pub(crate) async fn ack(&self, id: &str) -> Result<(), ApiError> {
        self.mutate(|requests| {
            let original_len = requests.len();
            requests.retain(|request| request.id != id);
            if requests.len() == original_len {
                return Err(ApiError::BadRequest("unknown index request".to_string()));
            }
            Ok(())
        })
        .await
    }

    pub(crate) async fn set_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<IndexQueueItem, ApiError> {
        self.mutate(|requests| {
            let request = find_queue_item(requests, id)?;
            request.status = status.to_string();
            Ok(request.clone())
        })
        .await
    }

    pub(crate) async fn set_status_from(
        &self,
        id: &str,
        status: &str,
        allowed_statuses: &[&str],
    ) -> Result<IndexQueueItem, ApiError> {
        self.mutate(|requests| {
            let request = find_queue_item(requests, id)?;
            if request.status != status {
                if !allowed_statuses.contains(&request.status.as_str()) {
                    return Err(ApiError::BadRequest(format!(
                        "cannot change index request from {} to {status}",
                        request.status
                    )));
                }
                request.status = status.to_string();
            }
            Ok(request.clone())
        })
        .await
    }

    async fn mutate<T>(
        &self,
        update: impl FnOnce(&mut Vec<IndexQueueItem>) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let _guard = self.mutation_lock.lock().await;
        let mut requests = self.list(usize::MAX).await?;
        let result = update(&mut requests)?;
        self.write(&requests).await?;
        Ok(result)
    }

    async fn write(&self, requests: &[IndexQueueItem]) -> Result<(), ApiError> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut body = requests
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ApiError::Queue(error.to_string()))?
            .join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        let temporary = self
            .path
            .with_extension(format!("tmp.{}", std::process::id()));
        tokio::fs::write(&temporary, body).await?;
        tokio::fs::rename(temporary, &self.path).await?;
        Ok(())
    }

    pub(crate) fn authorize(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        let Some(expected) = &self.token else {
            return Err(ApiError::Unauthorized);
        };
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        let explicit = headers
            .get("x-index-queue-token")
            .and_then(|value| value.to_str().ok());
        if bearer == Some(expected.as_str()) || explicit == Some(expected.as_str()) {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        }
    }
}

pub(crate) fn new_index_request_id(target_backend: &str, handle: &str) -> String {
    let epoch_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = REQUEST_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "{epoch_nanos:x}-{:x}-{sequence:x}-{target_backend}-{}",
        std::process::id(),
        handle.trim_start_matches('@')
    )
}

fn find_queue_item<'a>(
    requests: &'a mut [IndexQueueItem],
    id: &str,
) -> Result<&'a mut IndexQueueItem, ApiError> {
    requests
        .iter_mut()
        .find(|request| request.id == id)
        .ok_or_else(|| ApiError::BadRequest("unknown index request".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_queue(label: &str, max_items: usize) -> IndexQueue {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        IndexQueue::new(
            std::env::temp_dir().join(format!(
                "ytsearch-{label}-{}-{nonce}.jsonl",
                std::process::id()
            )),
            None,
            max_items,
        )
    }

    fn test_request(id: &str, handle: &str) -> IndexQueueItem {
        IndexQueueItem {
            id: id.to_string(),
            handle: handle.to_string(),
            note: None,
            requested_at: 1,
            source_backend: "raw".to_string(),
            target_backend: Some("raw".to_string()),
            status: "pending".to_string(),
        }
    }

    #[test]
    fn request_ids_preserve_distinct_handles_and_are_unique() {
        let first = new_index_request_id("raw", "@a-b");
        let second = new_index_request_id("raw", "@ab");
        let third = new_index_request_id("raw", "@a-b");

        assert_ne!(first, second);
        assert_ne!(first, third);
        assert!(first.ends_with("raw-a-b"));
        assert!(second.ends_with("raw-ab"));
    }

    #[tokio::test]
    async fn refuses_new_items_at_capacity_but_keeps_deduplication_idempotent() {
        let queue = test_queue("capacity", 1);
        let first = test_request("first", "@first");
        queue.enqueue(first.clone()).await.expect("enqueue first");

        let duplicate = IndexQueueItem {
            id: "duplicate".to_string(),
            ..first.clone()
        };
        assert_eq!(
            queue
                .enqueue(duplicate)
                .await
                .expect("deduplicate at capacity")
                .id,
            first.id
        );
        assert!(matches!(
            queue.enqueue(test_request("second", "@second")).await,
            Err(ApiError::QueueFull)
        ));
        assert_eq!(queue.list(usize::MAX).await.expect("list queue").len(), 1);

        let _ = tokio::fs::remove_file(queue.path).await;
    }

    #[tokio::test]
    async fn pending_requests_require_approval_before_becoming_queueable() {
        let queue = test_queue("approval", 10);
        let request = test_request("approval-request", "@test");
        queue
            .enqueue(request.clone())
            .await
            .expect("enqueue request");

        let approved = queue
            .set_status_from(&request.id, "queued", &["pending", "rejected", "failed"])
            .await
            .expect("approve pending request");
        assert_eq!(approved.status, "queued");

        let approved_again = queue
            .set_status_from(&request.id, "queued", &["pending", "rejected", "failed"])
            .await
            .expect("approval is idempotent");
        assert_eq!(approved_again.status, "queued");

        let rejection = queue
            .set_status_from(&request.id, "rejected", &["pending"])
            .await;
        assert!(rejection.is_err());
        let _ = tokio::fs::remove_file(queue.path).await;
    }
}
