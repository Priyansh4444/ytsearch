use blake3::Hash;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, backup::Backup,
    params,
};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};
use youtube_index_store::{
    ChannelDeletionScope, ChannelDocument, IndexOperation, IndexOperationState, IndexStoreError,
    VideoReplacement,
};
use youtube_ingest::TranscriptChunk;

const BACKEND: &str = "raw";
const MUTATION_SCHEMA_VERSION: i64 = 1;
const BACKUP_PAGES_PER_STEP: i32 = 256;

pub struct MutationStore {
    connection: Mutex<Connection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationSnapshotExport {
    pub(crate) through_sequence: u64,
    pub(crate) bytes: u64,
    pub(crate) blake3: String,
}

#[derive(Clone, Debug)]
pub struct StoredVideoMutation {
    pub(crate) video_id: String,
    pub(crate) chunks: Option<Vec<TranscriptChunk>>,
}

#[derive(Clone, Debug)]
pub struct StoredChannelMutation {
    pub(crate) channel_id: String,
    pub(crate) document: Option<ChannelDocument>,
    pub(crate) deletion_scope: ChannelDeletionScope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoMutationSummary {
    pub(crate) video_id: String,
    pub(crate) youtuber_id: Option<String>,
    pub(crate) chunk_count: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct MutationSnapshot {
    pub(crate) through_sequence: u64,
    pub(crate) videos: Vec<StoredVideoMutation>,
    pub(crate) channels: Vec<StoredChannelMutation>,
}

impl MutationStore {
    pub fn open(path: &Path) -> Result<Self, IndexStoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| IndexStoreError::internal(BACKEND, error))?;
        }
        let connection = Connection::open(path).map_err(map_sqlite)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=FULL;
                 PRAGMA foreign_keys=ON;
                 PRAGMA busy_timeout=5000;
                 CREATE TABLE IF NOT EXISTS operations (
                   sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                   id TEXT NOT NULL UNIQUE,
                   kind TEXT NOT NULL,
                   target TEXT NOT NULL,
                   payload_hash BLOB NOT NULL,
                   state INTEGER NOT NULL,
                   error TEXT
                 );
                 CREATE TABLE IF NOT EXISTS video_mutations (
                   video_id TEXT PRIMARY KEY,
                   sequence INTEGER NOT NULL REFERENCES operations(sequence),
                   chunks_json BLOB,
                   youtuber_id TEXT,
                   chunk_count INTEGER
                 );
                 CREATE TABLE IF NOT EXISTS channel_mutations (
                   channel_id TEXT PRIMARY KEY,
                   sequence INTEGER NOT NULL REFERENCES operations(sequence),
                   document_json BLOB,
                   cascade INTEGER NOT NULL DEFAULT 0
                 );
                 PRAGMA user_version=1;",
            )
            .map_err(map_sqlite)?;
        ensure_video_summary_columns(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn open_snapshot_read_only(path: &Path) -> Result<Self, IndexStoreError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(map_sqlite)?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON;")
            .map_err(map_sqlite)?;
        let schema_version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(map_sqlite)?;
        if schema_version != MUTATION_SCHEMA_VERSION {
            return Err(IndexStoreError::InvalidInput(format!(
                "unsupported mutation snapshot schema {schema_version}; expected {MUTATION_SCHEMA_VERSION}"
            )));
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub(crate) fn replace_video(
        &self,
        replacement: VideoReplacement<'_>,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("video_id", replacement.video_id())?;
        validate_idempotency_key(idempotency_key)?;
        let chunks_json = serde_json::to_vec(replacement.chunks()).map_err(map_json)?;
        let youtuber_id = replacement
            .chunks()
            .first()
            .map(|chunk| chunk.youtuber_id().to_string());
        let chunk_count = i64::try_from(replacement.chunks().len())
            .map_err(|error| IndexStoreError::internal(BACKEND, error))?;
        let hash = payload_hash("replace_video", replacement.video_id(), &chunks_json);
        self.write_operation(
            idempotency_key,
            "replace_video",
            replacement.video_id(),
            hash,
            |transaction, sequence| {
                transaction.execute(
                    "INSERT INTO video_mutations(
                       video_id, sequence, chunks_json, youtuber_id, chunk_count
                     ) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(video_id) DO UPDATE SET
                       sequence=excluded.sequence, chunks_json=excluded.chunks_json,
                       youtuber_id=excluded.youtuber_id, chunk_count=excluded.chunk_count",
                    params![
                        replacement.video_id(),
                        sequence,
                        chunks_json,
                        youtuber_id,
                        chunk_count
                    ],
                )?;
                Ok(())
            },
        )
    }

    pub(crate) fn delete_video(
        &self,
        video_id: &str,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("video_id", video_id)?;
        validate_idempotency_key(idempotency_key)?;
        let hash = payload_hash("delete_video", video_id, &[]);
        self.write_operation(
            idempotency_key,
            "delete_video",
            video_id,
            hash,
            |transaction, sequence| {
                transaction.execute(
                    "INSERT INTO video_mutations(
                       video_id, sequence, chunks_json, youtuber_id, chunk_count
                     ) VALUES (?1, ?2, NULL, NULL, NULL)
                     ON CONFLICT(video_id) DO UPDATE SET
                       sequence=excluded.sequence, chunks_json=NULL,
                       youtuber_id=NULL, chunk_count=NULL",
                    params![video_id, sequence],
                )?;
                Ok(())
            },
        )
    }

    pub(crate) fn upsert_channel(
        &self,
        channel: &ChannelDocument,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("channel id", &channel.id)?;
        validate_idempotency_key(idempotency_key)?;
        let document_json = serde_json::to_vec(channel).map_err(map_json)?;
        let hash = payload_hash("upsert_channel", &channel.id, &document_json);
        self.write_operation(
            idempotency_key,
            "upsert_channel",
            &channel.id,
            hash,
            |transaction, sequence| {
                transaction.execute(
                    "INSERT INTO channel_mutations(channel_id, sequence, document_json, cascade)
                     VALUES (?1, ?2, ?3, 0)
                     ON CONFLICT(channel_id) DO UPDATE SET
                       sequence=excluded.sequence, document_json=excluded.document_json, cascade=0",
                    params![channel.id, sequence, document_json],
                )?;
                Ok(())
            },
        )
    }

    pub(crate) fn delete_channel(
        &self,
        channel_id: &str,
        scope: ChannelDeletionScope,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("channel id", channel_id)?;
        validate_idempotency_key(idempotency_key)?;
        let cascade = scope.cascades_videos();
        let hash = payload_hash("delete_channel", channel_id, &[u8::from(cascade)]);
        self.write_operation(
            idempotency_key,
            "delete_channel",
            channel_id,
            hash,
            |transaction, sequence| {
                transaction.execute(
                    "INSERT INTO channel_mutations(channel_id, sequence, document_json, cascade)
                     VALUES (?1, ?2, NULL, ?3)
                     ON CONFLICT(channel_id) DO UPDATE SET
                       sequence=excluded.sequence, document_json=NULL, cascade=excluded.cascade",
                    params![channel_id, sequence, cascade],
                )?;
                Ok(())
            },
        )
    }

    pub(crate) fn operation(&self, operation_id: &str) -> Result<IndexOperation, IndexStoreError> {
        validate_idempotency_key(operation_id)?;
        self.connection()?
            .query_row(
                "SELECT sequence, state, error FROM operations WHERE id=?1",
                [operation_id],
                |row| operation_from_row(operation_id, row),
            )
            .optional()
            .map_err(map_sqlite)?
            .ok_or_else(|| IndexStoreError::InvalidInput("unknown operation id".to_string()))
    }

    pub(crate) fn snapshot(&self) -> Result<MutationSnapshot, IndexStoreError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(map_sqlite)?;
        let through_sequence_sql = snapshot_sequence(&transaction)?;
        let snapshot = snapshot_at_sequence(&transaction, through_sequence_sql)?;
        transaction.commit().map_err(map_sqlite)?;
        Ok(snapshot)
    }

    pub(crate) fn export_snapshot(
        &self,
        destination: &Path,
    ) -> Result<MutationSnapshotExport, IndexStoreError> {
        let parent = destination.parent().ok_or_else(|| {
            IndexStoreError::InvalidInput("mutation snapshot path has no parent".to_string())
        })?;
        fs::create_dir_all(parent).map_err(map_io)?;
        if destination.exists() {
            return Err(IndexStoreError::Conflict(format!(
                "mutation snapshot already exists: {}",
                destination.display()
            )));
        }
        let temporary = temporary_snapshot_path(destination);
        if temporary.exists() {
            fs::remove_file(&temporary).map_err(map_io)?;
        }

        let through_sequence = {
            let mut connection = self.connection()?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(map_sqlite)?;
            let through_sequence_sql = snapshot_sequence(&transaction)?;
            let mut output = Connection::open(&temporary).map_err(map_sqlite)?;
            {
                let backup = Backup::new(&transaction, &mut output).map_err(map_sqlite)?;
                backup
                    .run_to_completion(BACKUP_PAGES_PER_STEP, Duration::from_millis(1), None)
                    .map_err(map_sqlite)?;
            }
            drop(output);
            transaction.commit().map_err(map_sqlite)?;
            u64::try_from(through_sequence_sql)
                .map_err(|error| IndexStoreError::internal(BACKEND, error))?
        };

        File::open(&temporary)
            .and_then(|file| file.sync_all())
            .map_err(map_io)?;
        let exported = Self::open_snapshot_read_only(&temporary)?;
        let exported_sequence = exported.last_sequence()?;
        if exported_sequence != through_sequence {
            return Err(IndexStoreError::internal(
                BACKEND,
                format!(
                    "mutation snapshot sequence changed during backup: expected {through_sequence}, got {exported_sequence}"
                ),
            ));
        }
        drop(exported);
        let bytes = fs::metadata(&temporary).map_err(map_io)?.len();
        let blake3 = hash_file(&temporary)?;
        fs::rename(&temporary, destination).map_err(map_io)?;
        sync_directory(parent)?;

        Ok(MutationSnapshotExport {
            through_sequence,
            bytes,
            blake3,
        })
    }

    pub(crate) fn video_mutation_summary(
        &self,
        video_id: &str,
    ) -> Result<Option<VideoMutationSummary>, IndexStoreError> {
        validate_identifier("video_id", video_id)?;
        self.connection()?
            .query_row(
                "SELECT video_id, youtuber_id, chunk_count
                 FROM video_mutations WHERE video_id=?1",
                [video_id],
                video_summary_from_row,
            )
            .optional()
            .map_err(map_sqlite)
    }

    pub(crate) fn video_mutation_summaries(
        &self,
    ) -> Result<Vec<VideoMutationSummary>, IndexStoreError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT video_id, youtuber_id, chunk_count
                 FROM video_mutations ORDER BY video_id",
            )
            .map_err(map_sqlite)?;
        statement
            .query_map([], video_summary_from_row)
            .map_err(map_sqlite)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sqlite)
    }

    pub(crate) fn channel_mutation(
        &self,
        channel_id: &str,
    ) -> Result<Option<StoredChannelMutation>, IndexStoreError> {
        validate_identifier("channel id", channel_id)?;
        self.connection()?
            .query_row(
                "SELECT channel_id, document_json, cascade
                 FROM channel_mutations WHERE channel_id=?1",
                [channel_id],
                |row| {
                    let document_json = row.get::<_, Option<Vec<u8>>>(1)?;
                    let document = document_json
                        .map(|bytes| serde_json::from_slice::<ChannelDocument>(&bytes))
                        .transpose()
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Blob,
                                Box::new(error),
                            )
                        })?;
                    Ok(StoredChannelMutation {
                        channel_id: row.get(0)?,
                        document,
                        deletion_scope: ChannelDeletionScope::from_cascade(row.get(2)?),
                    })
                },
            )
            .optional()
            .map_err(map_sqlite)
    }

    pub(crate) fn mark_published(&self, through_sequence: u64) -> Result<(), IndexStoreError> {
        let through_sequence = i64::try_from(through_sequence)
            .map_err(|error| IndexStoreError::internal(BACKEND, error))?;
        self.connection()?
            .execute(
                "UPDATE operations SET state=?1, error=NULL
                 WHERE sequence <= ?2 AND state < ?1",
                params![
                    state_number(IndexOperationState::Published),
                    through_sequence
                ],
            )
            .map_err(map_sqlite)?;
        Ok(())
    }

    pub fn last_sequence(&self) -> Result<u64, IndexStoreError> {
        let sequence = self
            .connection()?
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM operations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite)?;
        u64::try_from(sequence).map_err(|error| IndexStoreError::internal(BACKEND, error))
    }

    fn write_operation<F>(
        &self,
        id: &str,
        kind: &str,
        target: &str,
        hash: Hash,
        write_mutation: F,
    ) -> Result<IndexOperation, IndexStoreError>
    where
        F: FnOnce(&Transaction<'_>, i64) -> rusqlite::Result<()>,
    {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        if let Some(existing) = existing_operation(&transaction, id)? {
            if existing.kind != kind
                || existing.target != target
                || existing.hash != hash.as_bytes()
            {
                return Err(IndexStoreError::Conflict(format!(
                    "idempotency key {id} was already used for a different mutation"
                )));
            }
            return Ok(existing.operation);
        }
        transaction
            .execute(
                "INSERT INTO operations(id, kind, target, payload_hash, state)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    kind,
                    target,
                    hash.as_bytes().as_slice(),
                    state_number(IndexOperationState::Durable)
                ],
            )
            .map_err(map_sqlite)?;
        let sequence_sql = transaction.last_insert_rowid();
        let sequence = u64::try_from(sequence_sql)
            .map_err(|error| IndexStoreError::internal(BACKEND, error))?;
        write_mutation(&transaction, sequence_sql).map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        Ok(operation(id, sequence, IndexOperationState::Durable, None))
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, IndexStoreError> {
        self.connection
            .lock()
            .map_err(|_| IndexStoreError::internal(BACKEND, "mutation store lock poisoned"))
    }
}

fn snapshot_sequence(transaction: &Transaction<'_>) -> Result<i64, IndexStoreError> {
    transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM operations",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(map_sqlite)
}

fn snapshot_at_sequence(
    transaction: &Transaction<'_>,
    through_sequence_sql: i64,
) -> Result<MutationSnapshot, IndexStoreError> {
    let through_sequence = u64::try_from(through_sequence_sql)
        .map_err(|error| IndexStoreError::internal(BACKEND, error))?;
    let mut video_statement = transaction
        .prepare(
            "SELECT video_id, chunks_json
                 FROM video_mutations WHERE sequence <= ?1 ORDER BY video_id",
        )
        .map_err(map_sqlite)?;
    let videos = video_statement
        .query_map([through_sequence_sql], |row| {
            let chunks_json = row.get::<_, Option<Vec<u8>>>(1)?;
            let chunks = chunks_json
                .map(|bytes| serde_json::from_slice::<Vec<TranscriptChunk>>(&bytes))
                .transpose()
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Blob,
                        Box::new(error),
                    )
                })?;
            Ok(StoredVideoMutation {
                video_id: row.get(0)?,
                chunks,
            })
        })
        .map_err(map_sqlite)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite)?;
    drop(video_statement);

    let mut channel_statement = transaction
        .prepare(
            "SELECT channel_id, document_json, cascade
                 FROM channel_mutations WHERE sequence <= ?1 ORDER BY channel_id",
        )
        .map_err(map_sqlite)?;
    let channels = channel_statement
        .query_map([through_sequence_sql], |row| {
            let document_json = row.get::<_, Option<Vec<u8>>>(1)?;
            let document = document_json
                .map(|bytes| serde_json::from_slice::<ChannelDocument>(&bytes))
                .transpose()
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Blob,
                        Box::new(error),
                    )
                })?;
            Ok(StoredChannelMutation {
                channel_id: row.get(0)?,
                document,
                deletion_scope: ChannelDeletionScope::from_cascade(row.get(2)?),
            })
        })
        .map_err(map_sqlite)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite)?;

    Ok(MutationSnapshot {
        through_sequence,
        videos,
        channels,
    })
}

fn temporary_snapshot_path(destination: &Path) -> PathBuf {
    let mut value = destination.as_os_str().to_os_string();
    value.push(".new");
    PathBuf::from(value)
}

fn hash_file(path: &Path) -> Result<String, IndexStoreError> {
    let mut input = File::open(path).map_err(map_io)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut input).map_err(map_io)?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn sync_directory(path: &Path) -> Result<(), IndexStoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(map_io)
}

fn ensure_video_summary_columns(connection: &Connection) -> Result<(), IndexStoreError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(video_mutations)")
        .map_err(map_sqlite)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(map_sqlite)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite)?;
    drop(statement);
    if !columns.iter().any(|column| column == "youtuber_id") {
        connection
            .execute(
                "ALTER TABLE video_mutations ADD COLUMN youtuber_id TEXT",
                [],
            )
            .map_err(map_sqlite)?;
    }
    if !columns.iter().any(|column| column == "chunk_count") {
        connection
            .execute(
                "ALTER TABLE video_mutations ADD COLUMN chunk_count INTEGER",
                [],
            )
            .map_err(map_sqlite)?;
    }
    let mut statement = connection
        .prepare(
            "SELECT video_id, chunks_json FROM video_mutations
             WHERE chunks_json IS NOT NULL
               AND (youtuber_id IS NULL OR chunk_count IS NULL)",
        )
        .map_err(map_sqlite)?;
    let missing = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(map_sqlite)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite)?;
    drop(statement);
    for (video_id, chunks_json) in missing {
        let chunks =
            serde_json::from_slice::<Vec<TranscriptChunk>>(&chunks_json).map_err(map_json)?;
        let youtuber_id = chunks.first().map(TranscriptChunk::youtuber_id);
        let chunk_count = i64::try_from(chunks.len())
            .map_err(|error| IndexStoreError::internal(BACKEND, error))?;
        connection
            .execute(
                "UPDATE video_mutations SET youtuber_id=?1, chunk_count=?2 WHERE video_id=?3",
                params![youtuber_id, chunk_count, video_id],
            )
            .map_err(map_sqlite)?;
    }
    Ok(())
}

fn video_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VideoMutationSummary> {
    let chunk_count = row
        .get::<_, Option<i64>>(2)?
        .map(|count| {
            u64::try_from(count).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, count))
        })
        .transpose()?;
    Ok(VideoMutationSummary {
        video_id: row.get(0)?,
        youtuber_id: row.get(1)?,
        chunk_count,
    })
}

struct ExistingOperation {
    kind: String,
    target: String,
    hash: Vec<u8>,
    operation: IndexOperation,
}

fn existing_operation(
    transaction: &Transaction<'_>,
    id: &str,
) -> Result<Option<ExistingOperation>, IndexStoreError> {
    transaction
        .query_row(
            "SELECT sequence, kind, target, payload_hash, state, error
             FROM operations WHERE id=?1",
            [id],
            |row| {
                let sequence = row_u64(row, 0)?;
                Ok(ExistingOperation {
                    kind: row.get(1)?,
                    target: row.get(2)?,
                    hash: row.get(3)?,
                    operation: operation(
                        id,
                        sequence,
                        state_from_number(row.get(4)?)?,
                        row.get(5)?,
                    ),
                })
            },
        )
        .optional()
        .map_err(map_sqlite)
}

fn operation_from_row(id: &str, row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexOperation> {
    Ok(operation(
        id,
        row_u64(row, 0)?,
        state_from_number(row.get(1)?)?,
        row.get(2)?,
    ))
}

fn operation(
    id: &str,
    sequence: u64,
    state: IndexOperationState,
    error: Option<String>,
) -> IndexOperation {
    IndexOperation {
        id: id.to_string(),
        sequence: Some(sequence),
        state,
        acknowledged_readers: u8::from(state == IndexOperationState::Published),
        error,
    }
}

fn payload_hash(kind: &str, target: &str, payload: &[u8]) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(kind.as_bytes());
    hasher.update(&[0]);
    hasher.update(target.as_bytes());
    hasher.update(&[0]);
    hasher.update(payload);
    hasher.finalize()
}

fn validate_identifier(label: &str, value: &str) -> Result<(), IndexStoreError> {
    if value.is_empty() || value.len() > 256 {
        return Err(IndexStoreError::InvalidInput(format!(
            "{label} must contain between 1 and 256 bytes"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'_' | b'-' | b'.'))
    {
        return Err(IndexStoreError::InvalidInput(format!(
            "{label} contains an unsupported character"
        )));
    }
    Ok(())
}

fn validate_idempotency_key(value: &str) -> Result<(), IndexStoreError> {
    if value.is_empty() || value.len() > 256 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(IndexStoreError::InvalidInput(
            "idempotency key must be at most 256 visible ASCII bytes".to_string(),
        ));
    }
    Ok(())
}

fn state_number(state: IndexOperationState) -> i64 {
    state as i64
}

fn state_from_number(value: i64) -> rusqlite::Result<IndexOperationState> {
    match value {
        0 => Ok(IndexOperationState::Accepted),
        1 => Ok(IndexOperationState::Durable),
        2 => Ok(IndexOperationState::Published),
        3 => Ok(IndexOperationState::Replicated),
        4 => Ok(IndexOperationState::Failed),
        _ => Err(rusqlite::Error::IntegralValueOutOfRange(0, value)),
    }
}

fn row_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn map_sqlite(error: rusqlite::Error) -> IndexStoreError {
    IndexStoreError::internal(BACKEND, error)
}

fn map_io(error: io::Error) -> IndexStoreError {
    IndexStoreError::internal(BACKEND, error)
}

fn map_json(error: serde_json::Error) -> IndexStoreError {
    IndexStoreError::internal(BACKEND, error)
}

#[cfg(test)]
mod tests;
