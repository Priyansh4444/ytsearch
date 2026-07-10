use async_trait::async_trait;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
};
use youtube_ingest::{ChunkSink, IngestError, TranscriptChunk};

#[derive(Debug, Error)]
pub enum JsonlError {
    #[error("jsonl io failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("json encode failed: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("json decode failed on line {line}: {source}")]
    Decode {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}

pub type Result<T> = std::result::Result<T, JsonlError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonlWriteMode {
    Append,
    Truncate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonlSink {
    path: PathBuf,
    mode: JsonlWriteMode,
}

impl JsonlSink {
    pub fn append(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mode: JsonlWriteMode::Append,
        }
    }

    pub fn overwrite(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mode: JsonlWriteMode::Truncate,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn mode(&self) -> JsonlWriteMode {
        self.mode
    }

    async fn write(&self, chunks: &[TranscriptChunk]) -> Result<()> {
        match self.mode {
            JsonlWriteMode::Append => append_chunks(&self.path, chunks).await,
            JsonlWriteMode::Truncate => write_chunks(&self.path, chunks).await,
        }
    }
}

#[async_trait]
impl ChunkSink for JsonlSink {
    async fn upsert_chunks(&self, chunks: &[TranscriptChunk]) -> youtube_ingest::Result<()> {
        self.write(chunks)
            .await
            .map_err(|error| IngestError::Sink(error.to_string()))
    }
}

pub async fn write_chunks(path: impl AsRef<Path>, chunks: &[TranscriptChunk]) -> Result<()> {
    let path = path.as_ref();
    ensure_parent(path).await?;

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .await?;

    write_chunk_lines(&mut file, chunks).await
}

pub async fn append_chunks(path: impl AsRef<Path>, chunks: &[TranscriptChunk]) -> Result<()> {
    let path = path.as_ref();
    ensure_parent(path).await?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;

    write_chunk_lines(&mut file, chunks).await
}

pub async fn read_chunks(path: impl AsRef<Path>) -> Result<Vec<TranscriptChunk>> {
    let file = File::open(path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut chunks = Vec::new();
    let mut line_number = 0;

    while let Some(line) = lines.next_line().await? {
        line_number += 1;
        if line.trim().is_empty() {
            continue;
        }

        let chunk = serde_json::from_str::<TranscriptChunk>(&line).map_err(|source| {
            JsonlError::Decode {
                line: line_number,
                source,
            }
        })?;
        chunks.push(chunk);
    }

    Ok(chunks)
}

async fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).await?;
    }

    Ok(())
}

async fn write_chunk_lines(file: &mut File, chunks: &[TranscriptChunk]) -> Result<()> {
    let mut line = Vec::with_capacity(512);
    for chunk in chunks {
        line.clear();
        serde_json::to_writer(&mut line, chunk).map_err(JsonlError::Encode)?;
        line.push(b'\n');
        file.write_all(&line).await?;
    }

    file.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn chunk(id: &str) -> TranscriptChunk {
        TranscriptChunk {
            id: id.to_string(),
            youtuber_id: "t3dotgg".to_string(),
            youtuber_handle: "@t3dotgg".to_string(),
            youtuber_name: "Theo - t3.gg".to_string(),
            channel_id: "channel-1".to_string(),
            video_id: "video-1".to_string(),
            title: "A useful video".to_string(),
            text: "hello world".to_string(),
            prev_text: None,
            next_text: None,
            sequence: 0,
            chapter: None,
            chapter_index: None,
            duration: Some(5_000),
            t_start_ms: 0,
            t_end_ms: 5_000,
            thumbnail: None,
            upload_date: Some("20260101".to_string()),
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            published_at_timestamp: Some(1_767_225_600),
            uploader_id: "@t3dotgg".to_string(),
            embedding: None,
            extra: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn writes_and_reads_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chunks.jsonl");

        write_chunks(&path, &[chunk("video-1_0"), chunk("video-1_1")])
            .await
            .unwrap();

        let chunks = read_chunks(&path).await.unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].id, "video-1_0");
        assert_eq!(chunks[1].id, "video-1_1");
    }
}
