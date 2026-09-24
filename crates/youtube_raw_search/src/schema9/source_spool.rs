//! Disk-backed source ordering before dense schema-9 ID assignment.
//!
//! This private stage does not choose per-video metadata or write an index.
//! Its two files are temporary and must never be included in a generation.

use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    ops::Range,
    path::Path,
};

use rusqlite::{Connection, params};

use super::source::{MAX_SOURCE_LINE_BYTES, SourceDocument, SourceError};

#[derive(Debug, thiserror::Error)]
pub(crate) enum SpoolError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error("duplicate external document ID: {0}")]
    DuplicateId(String),
    #[error("source spool offset or dense ID exceeds its format limit")]
    Size,
    #[error("source spool row no longer matches its source bytes")]
    Mismatch,
    #[error("source spool was rejected and cannot be finalized")]
    Rejected,
    #[error("video {video_id} has conflicting {field} values")]
    VideoConflict {
        video_id: String,
        field: &'static str,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VideoSource {
    pub(crate) external_video_id: String,
    pub(crate) creator_id: String,
    pub(crate) title: String,
    pub(crate) duration_ms: Option<u32>,
    pub(crate) published_at_s: Option<i64>,
    pub(crate) views: Option<u64>,
    pub(crate) likes: Option<u64>,
    pub(crate) comments: Option<u64>,
    pub(crate) documents: Range<u32>,
}

impl VideoSource {
    fn first(dense_id: u32, source: SourceDocument) -> Result<Self, SpoolError> {
        Ok(Self {
            external_video_id: source.external_video_id,
            creator_id: source.creator_id,
            title: source.title,
            duration_ms: source.duration_ms,
            published_at_s: source.published_at_s,
            views: source.views,
            likes: source.likes,
            comments: source.comments,
            documents: dense_id..dense_id.checked_add(1).ok_or(SpoolError::Size)?,
        })
    }

    fn absorb(&mut self, dense_id: u32, source: SourceDocument) -> Result<(), SpoolError> {
        if self.documents.end != dense_id || self.external_video_id != source.external_video_id {
            return Err(SpoolError::Mismatch);
        }
        if self.creator_id != source.creator_id {
            return Err(self.conflict("creator"));
        }
        if self.title != source.title {
            return Err(self.conflict("title"));
        }
        merge_known(
            &mut self.duration_ms,
            source.duration_ms,
            &self.external_video_id,
            "duration",
        )?;
        merge_known(
            &mut self.published_at_s,
            source.published_at_s,
            &self.external_video_id,
            "publication time",
        )?;
        self.views = self.views.max(source.views);
        self.likes = self.likes.max(source.likes);
        self.comments = self.comments.max(source.comments);
        self.documents.end = dense_id.checked_add(1).ok_or(SpoolError::Size)?;
        Ok(())
    }

    fn conflict(&self, field: &'static str) -> SpoolError {
        SpoolError::VideoConflict {
            video_id: self.external_video_id.clone(),
            field,
        }
    }
}

fn merge_known<T: Copy + Eq>(
    current: &mut Option<T>,
    incoming: Option<T>,
    video_id: &str,
    field: &'static str,
) -> Result<(), SpoolError> {
    match (*current, incoming) {
        (Some(left), Some(right)) if left != right => Err(SpoolError::VideoConflict {
            video_id: video_id.to_owned(),
            field,
        }),
        (None, Some(value)) => {
            *current = Some(value);
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(crate) struct SourceSpool {
    database: Connection,
    output: BufWriter<File>,
    next_offset: u64,
    rejected: bool,
}

pub(crate) struct SortedSource {
    database: Connection,
    input: BufReader<File>,
}

/// Only this state may feed dense-ID assignment and structural writing.
pub(crate) struct AuditedSource {
    sorted: SortedSource,
    videos: Vec<VideoSource>,
}

impl SourceSpool {
    /// `root` is a new, caller-owned scratch directory. Existing scratch
    /// files cause failure so a partially built run cannot be reused.
    pub(crate) fn create(root: &Path) -> Result<Self, SpoolError> {
        let output = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.join("source.ndjson"))?;
        let database_path = root.join("source.sqlite");
        if database_path.exists() {
            return Err(SpoolError::Mismatch);
        }
        let database = Connection::open(database_path)?;
        database.execute_batch(
            "CREATE TABLE documents (
                id TEXT PRIMARY KEY NOT NULL,
                video_id TEXT NOT NULL,
                t_start_ms INTEGER NOT NULL,
                source_offset INTEGER NOT NULL,
                source_length INTEGER NOT NULL
            ) WITHOUT ROWID;
            BEGIN IMMEDIATE;",
        )?;
        Ok(Self {
            database,
            output: BufWriter::new(output),
            next_offset: 0,
            rejected: false,
        })
    }

    pub(crate) fn append_line(&mut self, line: &str) -> Result<(), SpoolError> {
        if self.rejected {
            return Err(SpoolError::Rejected);
        }
        let result = self.append_line_inner(line);
        if result.is_err() {
            self.rejected = true;
        }
        result
    }

    fn append_line_inner(&mut self, line: &str) -> Result<(), SpoolError> {
        let source = SourceDocument::parse_json_line(line)?;
        let offset = i64::try_from(self.next_offset).map_err(|_| SpoolError::Size)?;
        let length = i64::try_from(line.len()).map_err(|_| SpoolError::Size)?;
        let next = self
            .next_offset
            .checked_add(line.len() as u64)
            .and_then(|value| value.checked_add(1))
            .ok_or(SpoolError::Size)?;
        if next > i64::MAX as u64 {
            return Err(SpoolError::Size);
        }
        self.output.write_all(line.as_bytes())?;
        self.output.write_all(b"\n")?;
        self.next_offset = next;
        match self.database.execute(
            "INSERT INTO documents (id, video_id, t_start_ms, source_offset, source_length)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                source.external_id,
                source.external_video_id,
                source.t_start_ms,
                offset,
                length
            ],
        ) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(SpoolError::DuplicateId(source.external_id))
            }
            Err(error) => Err(SpoolError::Sql(error)),
        }
    }

    pub(crate) fn finish(mut self) -> Result<SortedSource, SpoolError> {
        if self.rejected {
            return Err(SpoolError::Rejected);
        }
        self.output.flush()?;
        self.output.get_ref().sync_all()?;
        self.database.execute_batch(
            "COMMIT;
             CREATE INDEX documents_order
             ON documents (video_id COLLATE BINARY, t_start_ms, id COLLATE BINARY);",
        )?;
        // Retain the exact spool inode written above. A path-based reopen
        // could silently select a replacement file in the scratch directory.
        let input = BufReader::new(self.output.get_ref().try_clone()?);
        Ok(SortedSource {
            database: self.database,
            input,
        })
    }
}

impl SortedSource {
    /// Internal scan used to establish the video audit before exposing rows.
    fn visit_sorted(
        &mut self,
        mut visit: impl FnMut(u32, SourceDocument) -> Result<(), SpoolError>,
    ) -> Result<u64, SpoolError> {
        let mut statement = self.database.prepare(
            "SELECT id, video_id, t_start_ms, source_offset, source_length
             FROM documents
             ORDER BY video_id COLLATE BINARY, t_start_ms, id COLLATE BINARY",
        )?;
        let mut rows = statement.query([])?;
        let mut count = 0_u64;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let video_id: String = row.get(1)?;
            let t_start_ms: u32 = row.get(2)?;
            let offset = u64::try_from(row.get::<_, i64>(3)?).map_err(|_| SpoolError::Mismatch)?;
            let length =
                usize::try_from(row.get::<_, i64>(4)?).map_err(|_| SpoolError::Mismatch)?;
            if length > MAX_SOURCE_LINE_BYTES {
                return Err(SpoolError::Mismatch);
            }
            if count >= u64::from(u32::MAX) {
                return Err(SpoolError::Size);
            }
            let dense_id = count as u32;
            self.input.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; length];
            self.input.read_exact(&mut bytes)?;
            let line = std::str::from_utf8(&bytes).map_err(|_| SpoolError::Mismatch)?;
            let source = SourceDocument::parse_json_line(line)?;
            if source.external_id != id
                || source.external_video_id != video_id
                || source.t_start_ms != t_start_ms
            {
                return Err(SpoolError::Mismatch);
            }
            visit(dense_id, source)?;
            count += 1;
        }
        Ok(count)
    }

    /// Audit one compact row per video after the global source sort. A missing
    /// value may be filled by another chunk; two distinct known values fail.
    /// Engagement counters use the highest known observation, matching the
    /// current Raw aggregation while retaining the all-missing state.
    pub(crate) fn audit_videos(mut self) -> Result<AuditedSource, SpoolError> {
        let mut videos = Vec::new();
        let mut active: Option<VideoSource> = None;
        self.visit_sorted(|dense_id, source| {
            if active
                .as_ref()
                .is_some_and(|video| video.external_video_id != source.external_video_id)
            {
                videos.push(active.take().ok_or(SpoolError::Mismatch)?);
            }
            if let Some(video) = &mut active {
                video.absorb(dense_id, source)?;
            } else {
                active = Some(VideoSource::first(dense_id, source)?);
            }
            Ok(())
        })?;
        if let Some(video) = active {
            videos.push(video);
        }
        Ok(AuditedSource {
            sorted: self,
            videos,
        })
    }
}

impl AuditedSource {
    pub(crate) fn videos(&self) -> &[VideoSource] {
        &self.videos
    }

    /// Visit source in bytewise video ID, start time, exact document ID order.
    pub(crate) fn visit_sorted(
        &mut self,
        visit: impl FnMut(u32, SourceDocument) -> Result<(), SpoolError>,
    ) -> Result<u64, SpoolError> {
        self.sorted.visit_sorted(visit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, video: &str, start: u32) -> String {
        serde_json::json!({
            "id": id, "video_id": video, "youtuber_id": "creator",
            "title": "title", "text": "text", "t_start_ms": start,
            "t_end_ms": start + 1000
        })
        .to_string()
    }

    #[test]
    fn orders_video_time_and_exact_id_without_defaulting_source_fields() {
        let directory = tempfile::tempdir().unwrap();
        let mut spool = SourceSpool::create(directory.path()).unwrap();
        spool.append_line(&line("z", "video-b", 0)).unwrap();
        spool.append_line(&line("b", "video-a", 20)).unwrap();
        spool.append_line(&line("a", "video-a", 20)).unwrap();
        let mut sorted = spool.finish().unwrap().audit_videos().unwrap();
        let mut seen = Vec::new();
        assert_eq!(
            sorted
                .visit_sorted(|dense, source| {
                    seen.push((dense, source.external_id, source.views));
                    Ok(())
                })
                .unwrap(),
            3
        );
        assert_eq!(
            seen,
            [
                (0, "a".into(), None),
                (1, "b".into(), None),
                (2, "z".into(), None)
            ]
        );
    }

    #[test]
    fn duplicate_exact_document_id_fails_before_dense_assignment() {
        let directory = tempfile::tempdir().unwrap();
        let mut spool = SourceSpool::create(directory.path()).unwrap();
        spool.append_line(&line("same", "one", 0)).unwrap();
        assert!(matches!(
            spool.append_line(&line("same", "two", 0)),
            Err(SpoolError::DuplicateId(id)) if id == "same"
        ));
        assert!(matches!(spool.finish(), Err(SpoolError::Rejected)));
    }

    #[test]
    fn audits_video_ranges_known_values_and_missing_counters() {
        let directory = tempfile::tempdir().unwrap();
        let mut spool = SourceSpool::create(directory.path()).unwrap();
        let mut first: serde_json::Value = serde_json::from_str(&line("b", "one", 20)).unwrap();
        first["duration"] = 60_001.into();
        first["view_count"] = 0.into();
        spool.append_line(&first.to_string()).unwrap();
        let mut second: serde_json::Value = serde_json::from_str(&line("a", "one", 0)).unwrap();
        second["duration"] = 60_001.into();
        second["view_count"] = 42.into();
        spool.append_line(&second.to_string()).unwrap();
        spool.append_line(&line("c", "two", 0)).unwrap();
        let sorted = spool.finish().unwrap().audit_videos().unwrap();
        let videos = sorted.videos();
        assert_eq!(videos.len(), 2);
        assert_eq!(videos[0].documents, 0..2);
        assert_eq!(videos[0].duration_ms, Some(60_001));
        assert_eq!((videos[0].views, videos[0].likes), (Some(42), None));
        assert_eq!(videos[1].documents, 2..3);
        assert_eq!(videos[1].duration_ms, None);
        assert_eq!(videos[1].views, None);
    }

    #[test]
    fn contradictory_known_video_metadata_stops_audit() {
        let directory = tempfile::tempdir().unwrap();
        let mut spool = SourceSpool::create(directory.path()).unwrap();
        let mut first: serde_json::Value = serde_json::from_str(&line("a", "one", 0)).unwrap();
        first["duration"] = 60_000.into();
        spool.append_line(&first.to_string()).unwrap();
        let mut second: serde_json::Value = serde_json::from_str(&line("b", "one", 10)).unwrap();
        second["duration"] = 60_001.into();
        spool.append_line(&second.to_string()).unwrap();
        let sorted = spool.finish().unwrap();
        assert!(matches!(
            sorted.audit_videos(),
            Err(SpoolError::VideoConflict {
                field: "duration",
                ..
            })
        ));
    }
}
