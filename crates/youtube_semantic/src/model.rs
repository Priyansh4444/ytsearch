use crate::{Error, Result, domain::*};
use ort::{session::Session, value::Tensor};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
use tokenizers::Tokenizer;
use tokio::sync::{mpsc, oneshot};

/// Searches that may wait for the one inference thread (~30-70 ms each) before
/// new ones are told the service is busy.
pub(crate) const QUEUE_DEPTH: usize = 8;

pub(crate) struct Gemma {
    session: Session,
    tokenizer: Tokenizer,
}
impl Gemma {
    pub(crate) fn load(dir: &Path) -> Result<Self> {
        // Hash in bounded buffers, then let ORT map external weights from local files.
        for (name, expected) in ASSETS {
            let mut file = std::fs::File::open(dir.join(name)).map_err(|_| Error::Model)?;
            let mut hash = Sha256::new();
            let mut buffer = [0; 64 * 1024];
            loop {
                let count = file.read(&mut buffer).map_err(|_| Error::Model)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            if format!("{:x}", hash.finalize()) != expected {
                return Err(Error::Input("model asset checksum mismatch"));
            }
        }
        let mut tokenizer =
            Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|_| Error::Model)?;
        tokenizer.with_truncation(None).map_err(|_| Error::Model)?;
        tokenizer.with_padding(None);
        let runtime = std::env::var_os("ORT_DYLIB_PATH").ok_or(Error::Model)?;
        let environment = ort::init_from(runtime)
            .map_err(|_| Error::Model)?
            .with_telemetry(false)
            .build()
            .map_err(|_| Error::Model)?;
        environment.set_log_level(ort::logging::LogLevel::Fatal);
        let session = Session::builder(&environment)
            .map_err(|_| Error::Model)?
            .with_intra_threads(1)
            .map_err(|_| Error::Model)?
            .with_inter_threads(1)
            .map_err(|_| Error::Model)?
            .commit_from_file(dir.join("onnx/model_q4.onnx"))
            .map_err(|_| Error::Model)?;
        Ok(Self { session, tokenizer })
    }
    pub(crate) fn counter(&self) -> impl Fn(&str) -> Result<usize> + use<> {
        let tokenizer = self.tokenizer.clone();
        move |text| {
            Ok(tokenizer
                .encode(text, true)
                .map_err(|_| Error::Model)?
                .len())
        }
    }
    fn embed(&mut self, text: String, profile: &Profile) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|_| Error::Model)?;
        let length = encoding.len();
        if length == 0 || length > MAX_TOKENS {
            return Err(Error::Input("input exceeds 512 token cap"));
        }
        let ids: Vec<i64> = encoding.get_ids().iter().map(|id| i64::from(*id)).collect();
        let mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|id| i64::from(*id))
            .collect();
        let ids = Tensor::from_array(([1usize, length], ids)).map_err(|_| Error::Model)?;
        let mask = Tensor::from_array(([1usize, length], mask)).map_err(|_| Error::Model)?;
        let output = self
            .session
            .run(ort::inputs!["input_ids" => ids, "attention_mask" => mask])
            .map_err(|_| Error::Model)?;
        let (shape, values) = output["sentence_embedding"]
            .try_extract_tensor::<f32>()
            .map_err(|_| Error::Model)?;
        if shape.as_ref() != [1, 768] {
            return Err(Error::Model);
        }
        normalize(values.to_vec(), profile.dimensions)
    }
    pub(crate) fn worker(mut self, profile: Profile) -> Inference {
        let (send, mut recv) = mpsc::channel::<Work>(QUEUE_DEPTH);
        // Exactly one long-lived inference thread, rather than per-request blocking tasks.
        std::thread::spawn(move || {
            while let Some(work) = recv.blocking_recv() {
                if !work.reply.is_closed() {
                    let _ = work.reply.send(self.embed(work.text, &profile));
                }
            }
        });
        Inference(send)
    }
}
struct Work {
    text: String,
    reply: oneshot::Sender<Result<Vec<f32>>>,
}
#[derive(Clone)]
pub(crate) struct Inference(mpsc::Sender<Work>);
impl Inference {
    pub(crate) fn ready(&self) -> bool {
        !self.0.is_closed()
    }
    #[cfg(test)]
    pub(crate) fn fake() -> Self {
        let (send, mut recv) = mpsc::channel::<Work>(QUEUE_DEPTH);
        tokio::spawn(async move {
            while let Some(work) = recv.recv().await {
                // Serial and slow enough that concurrent callers must queue.
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                let _ = work.reply.send(Ok(vec![1. / 128f32.sqrt(); 128]));
            }
        });
        Self(send)
    }
    pub(crate) async fn embed(&self, text: String) -> Result<Vec<f32>> {
        let (reply, receiver) = oneshot::channel();
        // Queue behind the single inference thread; the caller's admission
        // semaphore bounds how many can wait, and the timeout bounds how long.
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            self.0
                .send(Work { text, reply })
                .await
                .map_err(|_| Error::Model)?;
            receiver.await.map_err(|_| Error::Model)?
        })
        .await
        .map_err(|_| Error::Busy)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn concurrent_searches_queue_instead_of_failing() {
        let inference = Inference::fake();
        let searches = (0..QUEUE_DEPTH).map(|i| {
            let inference = inference.clone();
            tokio::spawn(async move { inference.embed(format!("query {i}")).await })
        });
        for search in searches.collect::<Vec<_>>() {
            assert_eq!(search.await.unwrap().unwrap().len(), 128);
        }
    }
}
