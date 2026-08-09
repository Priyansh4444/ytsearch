use blake3::Hash;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::{
    fs,
    path::Path,
    sync::{Mutex, MutexGuard},
};
use youtube_index_store::{
    ChannelDocument, IndexOperation, IndexOperationState, IndexStoreError, VideoReplacement,
};
use youtube_ingest::TranscriptChunk;

const BACKEND: &str = "raw";

pub struct MutationStore {
    connection: Mutex<Connection>,
}

#[derive(Clone, Debug)]
pub struct StoredVideoMutation {
    pub sequence: u64,
    pub video_id: String,
    pub chunks: Option<Vec<TranscriptChunk>>,
}

#[derive(Clone, Debug)]
pub struct StoredChannelMutation {
    pub sequence: u64,
    pub channel_id: String,
    pub document: Option<ChannelDocument>,
    pub cascade: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MutationSnapshot {
    pub through_sequence: u64,
    pub videos: Vec<StoredVideoMutation>,
    pub channels: Vec<StoredChannelMutation>,
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
                   chunks_json BLOB
                 );
                 CREATE TABLE IF NOT EXISTS channel_mutations (
                   channel_id TEXT PRIMARY KEY,
                   sequence INTEGER NOT NULL REFERENCES operations(sequence),
                   document_json BLOB,
                   cascade INTEGER NOT NULL DEFAULT 0
                 );",
            )
            .map_err(map_sqlite)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn replace_video(
        &self,
        replacement: VideoReplacement<'_>,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("video_id", replacement.video_id())?;
        validate_idempotency_key(idempotency_key)?;
        let chunks_json = serde_json::to_vec(replacement.chunks()).map_err(map_json)?;
        let hash = payload_hash("replace_video", replacement.video_id(), &chunks_json);
        self.write_operation(
            idempotency_key,
            "replace_video",
            replacement.video_id(),
            hash,
            |transaction, sequence| {
                transaction.execute(
                    "INSERT INTO video_mutations(video_id, sequence, chunks_json)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(video_id) DO UPDATE SET
                       sequence=excluded.sequence, chunks_json=excluded.chunks_json",
                    params![replacement.video_id(), sequence, chunks_json],
                )?;
                Ok(())
            },
        )
    }

    pub fn delete_video(
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
                    "INSERT INTO video_mutations(video_id, sequence, chunks_json)
                     VALUES (?1, ?2, NULL)
                     ON CONFLICT(video_id) DO UPDATE SET
                       sequence=excluded.sequence, chunks_json=NULL",
                    params![video_id, sequence],
                )?;
                Ok(())
            },
        )
    }

    pub fn upsert_channel(
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

    pub fn delete_channel(
        &self,
        channel_id: &str,
        cascade: bool,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("channel id", channel_id)?;
        validate_idempotency_key(idempotency_key)?;
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

    pub fn operation(&self, operation_id: &str) -> Result<IndexOperation, IndexStoreError> {
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

    pub fn snapshot(&self) -> Result<MutationSnapshot, IndexStoreError> {
        let connection = self.connection()?;
        let through_sequence_sql = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM operations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite)?;
        let through_sequence = u64::try_from(through_sequence_sql)
            .map_err(|error| IndexStoreError::internal(BACKEND, error))?;
        let mut video_statement = connection
            .prepare(
                "SELECT sequence, video_id, chunks_json
                 FROM video_mutations WHERE sequence <= ?1 ORDER BY video_id",
            )
            .map_err(map_sqlite)?;
        let videos = video_statement
            .query_map([through_sequence_sql], |row| {
                let chunks_json = row.get::<_, Option<Vec<u8>>>(2)?;
                let chunks = chunks_json
                    .map(|bytes| serde_json::from_slice::<Vec<TranscriptChunk>>(&bytes))
                    .transpose()
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Blob,
                            Box::new(error),
                        )
                    })?;
                Ok(StoredVideoMutation {
                    sequence: row_u64(row, 0)?,
                    video_id: row.get(1)?,
                    chunks,
                })
            })
            .map_err(map_sqlite)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sqlite)?;
        drop(video_statement);

        let mut channel_statement = connection
            .prepare(
                "SELECT sequence, channel_id, document_json, cascade
                 FROM channel_mutations WHERE sequence <= ?1 ORDER BY channel_id",
            )
            .map_err(map_sqlite)?;
        let channels = channel_statement
            .query_map([through_sequence_sql], |row| {
                let document_json = row.get::<_, Option<Vec<u8>>>(2)?;
                let document = document_json
                    .map(|bytes| serde_json::from_slice::<ChannelDocument>(&bytes))
                    .transpose()
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Blob,
                            Box::new(error),
                        )
                    })?;
                Ok(StoredChannelMutation {
                    sequence: row_u64(row, 0)?,
                    channel_id: row.get(1)?,
                    document,
                    cascade: row.get(3)?,
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

    pub fn mark_published(&self, through_sequence: u64) -> Result<(), IndexStoreError> {
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

fn map_json(error: serde_json::Error) -> IndexStoreError {
    IndexStoreError::internal(BACKEND, error)
}

#[cfg(test)]
mod tests;
