use crate::{
    GenerationCompatibility, MutationStore, Publisher, ReloadableEngine,
    cache_io::{BackgroundIoGuard, run_background_work},
};
use axum::body::Body;
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::io::AsyncWriteExt;
use youtube_index_store::{
    IndexStoreError, RawGenerationFile, RawGenerationManifest, RawGenerationUpload,
    RawPublicationJob, RawPublicationStatus as RemoteJobStatus,
};

pub(crate) use youtube_index_store::RAW_GENERATION_FILES;
pub(crate) type GenerationFile = RawGenerationFile;
pub(crate) type GenerationManifest = RawGenerationManifest;
pub(crate) type GenerationUploadRequest = RawGenerationUpload;
pub(crate) type RemoteJobResponse = RawPublicationJob;

const BACKEND: &str = "raw";
const STATE_VERSION: u8 = 1;
const GENERATION_SCHEMA_VERSION: u8 = 7;
const STATE_FILE: &str = "job.json";
const SNAPSHOT_FILE: &str = "mutations.sqlite3";
const GENERATION_DIRECTORY: &str = "generation";
const GENERATION_MANIFEST: &str = "manifest.json";
const PREPARE_SNAPSHOT: &str = ".prepare-snapshot.sqlite3";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_GENERATION_FILE_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const MAX_GENERATION_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MIN_FREE_AFTER_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const UPLOAD_BUFFER_BYTES: usize = 256 * 1024;
const UPLOAD_RECLAIM_BYTES: u64 = 64 * 1024 * 1024;
const LEGACY_SCHEMA_SIX_FILES: [&str; 9] = [
    "channels.json",
    "videos.json",
    "lexicon.bin",
    "postings.bin",
    "positions.bin",
    "positions.full.bin",
    "documents.blocks",
    "documents.meta",
    "fuzzy.bin",
];
// Jobs persisted by a reader that advertised a schema-8 format stay readable,
// so a restart can retire them instead of failing to load its state.
const LEGACY_SCHEMA_EIGHT_FILES: [&str; 12] = [
    "channels.json",
    "videos.json",
    "lexicon.bin",
    "postings.bin",
    "positions.bin",
    "positions.full.bin",
    "documents.blocks",
    "documents.meta",
    "fuzzy.bin",
    "bigrams.text.bin",
    "bigrams.title.bin",
    "broad-single.bin",
];
const LEGACY_SCHEMA_SEVEN_V1_FORMAT: &str = "raw-index-schema-7-v1";

#[derive(Clone)]
pub(crate) struct RemotePublication {
    engine: Arc<ReloadableEngine>,
    store: Arc<MutationStore>,
    publisher: Arc<Publisher>,
    compatibility: GenerationCompatibility,
    root: PathBuf,
    operation: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedJob {
    state_version: u8,
    job_id: String,
    target_sequence: u64,
    snapshot_bytes: u64,
    snapshot_blake3: String,
    compatibility: GenerationCompatibility,
    status: RemoteJobStatus,
    #[serde(default)]
    manifest: Option<GenerationManifest>,
    #[serde(default)]
    completed_files: BTreeSet<String>,
}

pub(crate) struct SnapshotDownload {
    pub file: tokio::fs::File,
    pub bytes: u64,
    pub blake3: String,
}

impl RemotePublication {
    pub(crate) fn new(
        engine: Arc<ReloadableEngine>,
        store: Arc<MutationStore>,
        publisher: Arc<Publisher>,
    ) -> Result<Self, IndexStoreError> {
        let root = publisher.state_directory().join("work/remote-publication");
        fs::create_dir_all(root.join("jobs")).map_err(map_io)?;
        // Resolve immutable corpus identity before the listener is bound so
        // job preparation never scans the base archives on a serving reader.
        let compatibility = publisher.generation_compatibility()?;
        let publication = Self {
            engine,
            store,
            publisher,
            compatibility,
            root,
            operation: Arc::new(tokio::sync::Mutex::new(())),
        };
        if publication.state_path().exists() {
            let job = publication.load_job()?;
            verify_snapshot(
                &publication.job_directory(&job.job_id)?.join(SNAPSHOT_FILE),
                &job,
            )?;
            if job.compatibility != publication.compatibility {
                // A prepared generation is derived state: the mutation WAL is
                // still authoritative. Retire jobs from an older index format
                // at startup so prepare cannot return an unbuildable job and a
                // legacy accepted manifest cannot prevent the reader starting.
                publication.remove_job_directory(&job)?;
                fs::remove_file(publication.state_path()).map_err(map_io)?;
                sync_directory(&publication.root.join("jobs"))?;
                sync_directory(&publication.root)?;
            }
        }
        Ok(publication)
    }

    pub(crate) async fn prepare(&self) -> Result<RemoteJobResponse, IndexStoreError> {
        let _operation = self.operation.lock().await;
        if self.state_path().exists() {
            let existing = self.load_job()?;
            if existing.status != RemoteJobStatus::Activated
                || self.store.last_sequence()? <= self.engine.sequence()
            {
                return Ok(job_response(&existing));
            }
            self.remove_job_directory(&existing)?;
            fs::remove_file(self.state_path()).map_err(map_io)?;
            sync_directory(&self.root)?;
        }
        if self.store.last_sequence()? <= self.engine.sequence() {
            return Err(IndexStoreError::Conflict(
                "raw publication has no durable mutations beyond the active generation".to_string(),
            ));
        }

        let temporary_snapshot = self.root.join(PREPARE_SNAPSHOT);
        if temporary_snapshot.exists() {
            fs::remove_file(&temporary_snapshot).map_err(map_io)?;
        }
        let store = Arc::clone(&self.store);
        let compatibility = self.compatibility.clone();
        let snapshot_path = temporary_snapshot.clone();
        let prepared = run_background_work("raw-publication-prepare", move || {
            let _background_io = BackgroundIoGuard::enter();
            let snapshot = store.export_snapshot(&snapshot_path)?;
            Ok::<_, IndexStoreError>((compatibility, snapshot))
        })
        .await
        .map_err(map_io)??;
        let (compatibility, snapshot) = prepared;
        if snapshot.through_sequence <= self.engine.sequence() {
            fs::remove_file(&temporary_snapshot).map_err(map_io)?;
            return Err(IndexStoreError::Conflict(
                "raw publication snapshot is not newer than the active generation".to_string(),
            ));
        }

        let job_id = job_id(snapshot.through_sequence, &snapshot.blake3, &compatibility);
        let directory = self.job_directory(&job_id)?;
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(map_io)?;
        }
        fs::create_dir_all(directory.join(GENERATION_DIRECTORY)).map_err(map_io)?;
        fs::rename(&temporary_snapshot, directory.join(SNAPSHOT_FILE)).map_err(map_io)?;
        sync_directory(&directory)?;
        sync_directory(&self.root.join("jobs"))?;

        let job = PersistedJob {
            state_version: STATE_VERSION,
            job_id,
            target_sequence: snapshot.through_sequence,
            snapshot_bytes: snapshot.bytes,
            snapshot_blake3: snapshot.blake3,
            compatibility,
            status: RemoteJobStatus::Prepared,
            manifest: None,
            completed_files: BTreeSet::new(),
        };
        self.persist_job(&job)?;
        Ok(job_response(&job))
    }

    pub(crate) async fn open_snapshot(
        &self,
        job_id: &str,
    ) -> Result<SnapshotDownload, IndexStoreError> {
        let _operation = self.operation.lock().await;
        let job = self.load_matching_job(job_id)?;
        let file = tokio::fs::File::open(self.job_directory(job_id)?.join(SNAPSHOT_FILE))
            .await
            .map_err(map_io)?;
        Ok(SnapshotDownload {
            file,
            bytes: job.snapshot_bytes,
            blake3: job.snapshot_blake3,
        })
    }

    pub(crate) async fn status(&self, job_id: &str) -> Result<RemoteJobResponse, IndexStoreError> {
        let _operation = self.operation.lock().await;
        self.load_matching_job(job_id).map(|job| job_response(&job))
    }

    pub(crate) async fn accept_manifest(
        &self,
        job_id: &str,
        request: GenerationUploadRequest,
    ) -> Result<RemoteJobResponse, IndexStoreError> {
        let _operation = self.operation.lock().await;
        let mut job = self.load_matching_job(job_id)?;
        if job.status == RemoteJobStatus::Activated {
            return Err(IndexStoreError::Conflict(
                "raw publication job is already activated".to_string(),
            ));
        }
        if request.target_sequence != job.target_sequence {
            return Err(IndexStoreError::Conflict(format!(
                "generation sequence {} does not match prepared snapshot {}",
                request.target_sequence, job.target_sequence
            )));
        }
        if request.compatibility != job.compatibility {
            return Err(IndexStoreError::Conflict(
                "generation compatibility does not match the prepared reader".to_string(),
            ));
        }
        validate_manifest(&request.manifest)?;
        if let Some(existing) = &job.manifest {
            if existing != &request.manifest {
                return Err(IndexStoreError::Conflict(
                    "publication job already accepted a different manifest".to_string(),
                ));
            }
            return Ok(job_response(&job));
        }

        let manifest_bytes = manifest_bytes(&request.manifest)?;
        let generation = self.job_directory(job_id)?.join(GENERATION_DIRECTORY);
        let required = required_upload_space(&request.manifest)?;
        let available = available_space(&generation)?;
        if available < required {
            return Err(IndexStoreError::Conflict(format!(
                "raw reader has {available} free bytes but generation upload requires {required}"
            )));
        }
        write_atomic(&generation.join(GENERATION_MANIFEST), &manifest_bytes)?;
        sync_directory(&generation)?;
        job.manifest = Some(request.manifest);
        job.status = RemoteJobStatus::ManifestAccepted;
        self.persist_job(&job)?;
        Ok(job_response(&job))
    }

    pub(crate) async fn upload_file(
        &self,
        job_id: &str,
        file_name: &str,
        content_length: Option<u64>,
        mut body: Body,
    ) -> Result<RemoteJobResponse, IndexStoreError> {
        let allowlisted_name = allowlisted_file(file_name)?;
        let _operation = self.operation.lock().await;
        let mut job = self.load_matching_job(job_id)?;
        if job.status == RemoteJobStatus::Activated {
            return Err(IndexStoreError::Conflict(
                "raw publication job is already activated".to_string(),
            ));
        }
        let expected = job
            .manifest
            .as_ref()
            .ok_or_else(|| {
                IndexStoreError::Conflict(
                    "generation manifest must be accepted before file upload".to_string(),
                )
            })?
            .files
            .get(allowlisted_name)
            .expect("validated manifest contains every allowlisted file")
            .clone();
        if content_length.is_some_and(|bytes| bytes != expected.bytes) {
            return Err(IndexStoreError::InvalidInput(format!(
                "{allowlisted_name} Content-Length does not match manifest"
            )));
        }
        let generation = self.job_directory(job_id)?.join(GENERATION_DIRECTORY);
        let final_path = generation.join(allowlisted_name);
        if final_path.exists() {
            if verify_file(&final_path, &expected).is_ok() {
                job.completed_files.insert(allowlisted_name.to_string());
                self.persist_job(&job)?;
                return Ok(job_response(&job));
            }
            fs::remove_file(&final_path).map_err(map_io)?;
            job.completed_files.remove(allowlisted_name);
            self.persist_job(&job)?;
        }

        let part_path = generation.join(format!(".{allowlisted_name}.part"));
        if part_path.exists() {
            fs::remove_file(&part_path).map_err(map_io)?;
        }
        let output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part_path)
            .await
            .map_err(map_io)?;
        let mut output = tokio::io::BufWriter::with_capacity(UPLOAD_BUFFER_BYTES, output);
        let mut received = 0_u64;
        let mut reclaimed_through = 0_u64;
        let mut hasher = blake3::Hasher::new();
        let result = async {
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|error| IndexStoreError::unavailable(BACKEND, error))?;
                let Ok(chunk) = frame.into_data() else {
                    continue;
                };
                received = received
                    .checked_add(chunk.len() as u64)
                    .ok_or_else(|| invalid_file(allowlisted_name, "length overflow"))?;
                if received > expected.bytes {
                    return Err(invalid_file(
                        allowlisted_name,
                        "body exceeds manifest length",
                    ));
                }
                hasher.update(&chunk);
                output.write_all(&chunk).await.map_err(map_io)?;
                if received.saturating_sub(reclaimed_through) >= UPLOAD_RECLAIM_BYTES {
                    output.flush().await.map_err(map_io)?;
                    output.get_ref().sync_data().await.map_err(map_io)?;
                    advise_uploaded_range(
                        output.get_ref(),
                        reclaimed_through,
                        received - reclaimed_through,
                    );
                    reclaimed_through = received;
                }
            }
            if received != expected.bytes {
                return Err(invalid_file(
                    allowlisted_name,
                    "body is shorter than manifest length",
                ));
            }
            if hasher.finalize().to_hex().as_str() != expected.blake3 {
                return Err(invalid_file(
                    allowlisted_name,
                    "BLAKE3 does not match manifest",
                ));
            }
            output.flush().await.map_err(map_io)?;
            output.get_ref().sync_all().await.map_err(map_io)?;
            advise_uploaded_range(
                output.get_ref(),
                reclaimed_through,
                received - reclaimed_through,
            );
            Ok::<(), IndexStoreError>(())
        }
        .await;
        drop(output);
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(error);
        }
        tokio::fs::rename(&part_path, &final_path)
            .await
            .map_err(map_io)?;
        sync_directory(&generation)?;
        job.completed_files.insert(allowlisted_name.to_string());
        self.persist_job(&job)?;
        Ok(job_response(&job))
    }

    pub(crate) async fn activate(
        &self,
        job_id: &str,
    ) -> Result<RemoteJobResponse, IndexStoreError> {
        let _operation = self.operation.lock().await;
        let mut job = self.load_matching_job(job_id)?;
        let manifest = job.manifest.as_ref().ok_or_else(|| {
            IndexStoreError::Conflict(
                "generation manifest must be accepted before activation".to_string(),
            )
        })?;
        let expected_completed = RAW_GENERATION_FILES
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        if job.completed_files != expected_completed {
            return Err(IndexStoreError::Conflict(
                "every generation file must complete a verified upload before activation"
                    .to_string(),
            ));
        }
        let generation = self.job_directory(job_id)?.join(GENERATION_DIRECTORY);
        for name in RAW_GENERATION_FILES {
            let expected = manifest
                .files
                .get(name)
                .expect("validated manifest contains every allowlisted file");
            let metadata = fs::metadata(generation.join(name)).map_err(|error| {
                IndexStoreError::Conflict(format!(
                    "generation file {name} is incomplete or missing: {error}"
                ))
            })?;
            if !metadata.is_file() || metadata.len() != expected.bytes {
                return Err(IndexStoreError::Conflict(format!(
                    "generation file {name} is incomplete"
                )));
            }
        }
        let manifest_bytes = manifest_bytes(manifest)?;
        let publisher = Arc::clone(&self.publisher);
        let generation_path = generation.clone();
        let expected_manifest = manifest_bytes.clone();
        let sequence = job.target_sequence;
        let installed = run_background_work("raw-publication-activate", move || {
            let _background_io = BackgroundIoGuard::enter();
            publisher.install_verified_generation(sequence, &generation_path, &expected_manifest)
        })
        .await
        .map_err(map_io)??;
        self.engine
            .replace(installed.engine, sequence)
            .map_err(|error| IndexStoreError::internal(BACKEND, format!("{error:?}")))?;
        self.store.mark_published(sequence)?;
        job.status = RemoteJobStatus::Activated;
        job.completed_files = expected_completed;
        self.persist_job(&job)?;
        Ok(job_response(&job))
    }

    fn state_path(&self) -> PathBuf {
        self.root.join(STATE_FILE)
    }

    fn job_directory(&self, job_id: &str) -> Result<PathBuf, IndexStoreError> {
        validate_job_id(job_id)?;
        Ok(self.root.join("jobs").join(job_id))
    }

    fn load_matching_job(&self, job_id: &str) -> Result<PersistedJob, IndexStoreError> {
        validate_job_id(job_id)?;
        let job = self.load_job()?;
        if job.job_id != job_id {
            return Err(IndexStoreError::InvalidInput(
                "unknown remote publication job".to_string(),
            ));
        }
        Ok(job)
    }

    fn load_job(&self) -> Result<PersistedJob, IndexStoreError> {
        let job = self.read_job_state()?;
        let snapshot = self.job_directory(&job.job_id)?.join(SNAPSHOT_FILE);
        let metadata = fs::metadata(snapshot).map_err(map_io)?;
        if !metadata.is_file() || metadata.len() != job.snapshot_bytes {
            return Err(IndexStoreError::InvalidInput(
                "remote publication snapshot is missing or has the wrong length".to_string(),
            ));
        }
        Ok(job)
    }

    fn read_job_state(&self) -> Result<PersistedJob, IndexStoreError> {
        let job: PersistedJob =
            serde_json::from_reader(File::open(self.state_path()).map_err(map_io)?)
                .map_err(map_json)?;
        if job.state_version != STATE_VERSION {
            return Err(IndexStoreError::InvalidInput(format!(
                "unsupported remote publication state version {}",
                job.state_version
            )));
        }
        validate_job_id(&job.job_id)?;
        let expected_id = job_id(
            job.target_sequence,
            &job.snapshot_blake3,
            &job.compatibility,
        );
        if job.job_id != expected_id {
            return Err(IndexStoreError::InvalidInput(
                "remote publication job id does not match its persisted identity".to_string(),
            ));
        }
        validate_persisted_job(&job)?;
        Ok(job)
    }

    fn persist_job(&self, job: &PersistedJob) -> Result<(), IndexStoreError> {
        let bytes = serde_json::to_vec_pretty(job).map_err(map_json)?;
        write_atomic(&self.state_path(), &bytes)?;
        sync_directory(&self.root)
    }

    fn remove_job_directory(&self, job: &PersistedJob) -> Result<(), IndexStoreError> {
        let path = self.job_directory(&job.job_id)?;
        if path.exists() {
            fs::remove_dir_all(path).map_err(map_io)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn available_space(path: &Path) -> Result<u64, IndexStoreError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        IndexStoreError::InvalidInput("publication path contains a NUL byte".to_string())
    })?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stats` points to writable storage.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(map_io(std::io::Error::last_os_error()));
    }
    // SAFETY: a successful `statvfs` initialized the entire structure.
    let stats = unsafe { stats.assume_init() };
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn available_space(_path: &Path) -> Result<u64, IndexStoreError> {
    Ok(u64::MAX)
}

#[cfg(target_os = "linux")]
fn advise_uploaded_range(file: &tokio::fs::File, offset: u64, length: u64) {
    use std::os::fd::AsRawFd;

    if length == 0 {
        return;
    }
    // This is a cache-residency hint after durability, never a correctness
    // boundary. Failure leaves the bytes cached and is safe.
    let result = unsafe {
        libc::posix_fadvise(
            file.as_raw_fd(),
            offset.min(i64::MAX as u64) as i64,
            length.min(i64::MAX as u64) as i64,
            libc::POSIX_FADV_DONTNEED,
        )
    };
    if result != 0 {
        tracing::warn!(
            result,
            offset,
            length,
            "raw publication cache advice failed"
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn advise_uploaded_range(_file: &tokio::fs::File, _offset: u64, _length: u64) {}

fn validate_manifest(manifest: &GenerationManifest) -> Result<(), IndexStoreError> {
    validate_manifest_contract(manifest, GENERATION_SCHEMA_VERSION, &RAW_GENERATION_FILES)
}

fn validate_manifest_contract(
    manifest: &GenerationManifest,
    schema_version: u8,
    generation_files: &[&str],
) -> Result<(), IndexStoreError> {
    if manifest.schema_version != schema_version {
        return Err(IndexStoreError::InvalidInput(format!(
            "unsupported raw generation schema {}; expected {schema_version}",
            manifest.schema_version
        )));
    }
    if !manifest.average_document_tokens.is_finite()
        || manifest.average_document_tokens.is_sign_negative()
    {
        return Err(IndexStoreError::InvalidInput(
            "average_document_tokens must be finite and non-negative".to_string(),
        ));
    }
    let expected = generation_files.iter().copied().collect::<BTreeSet<_>>();
    let actual = manifest
        .files
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(IndexStoreError::InvalidInput(format!(
            "generation manifest must contain exactly the {} allowlisted files",
            generation_files.len()
        )));
    }
    let mut total = 0_u64;
    for (name, file) in &manifest.files {
        if file.bytes == 0 || file.bytes > MAX_GENERATION_FILE_BYTES {
            return Err(IndexStoreError::InvalidInput(format!(
                "generation file {name} has an invalid length"
            )));
        }
        total = total.checked_add(file.bytes).ok_or_else(|| {
            IndexStoreError::InvalidInput("generation byte length overflows".to_string())
        })?;
        if total > MAX_GENERATION_BYTES {
            return Err(IndexStoreError::InvalidInput(
                "generation exceeds the maximum accepted byte length".to_string(),
            ));
        }
        let parsed = blake3::Hash::from_hex(&file.blake3).map_err(|_| {
            IndexStoreError::InvalidInput(format!(
                "generation file {name} has an invalid BLAKE3 digest"
            ))
        })?;
        if parsed.to_hex().as_str() != file.blake3 {
            return Err(IndexStoreError::InvalidInput(format!(
                "generation file {name} BLAKE3 digest is not canonical lowercase hex"
            )));
        }
    }
    if manifest_bytes(manifest)?.len() > MAX_MANIFEST_BYTES {
        return Err(IndexStoreError::InvalidInput(
            "generation manifest exceeds the maximum accepted length".to_string(),
        ));
    }
    Ok(())
}

fn validate_persisted_job(job: &PersistedJob) -> Result<(), IndexStoreError> {
    if job.target_sequence == 0 || job.snapshot_bytes == 0 {
        return Err(IndexStoreError::InvalidInput(
            "remote publication job has an invalid snapshot identity".to_string(),
        ));
    }
    validate_canonical_blake3(&job.snapshot_blake3, "snapshot")?;
    validate_canonical_blake3(&job.compatibility.corpus_identity, "corpus identity")?;
    validate_canonical_blake3(
        &job.compatibility.base_chunks_blake3,
        "base chunks identity",
    )?;
    validate_canonical_blake3(
        &job.compatibility.base_channels_blake3,
        "base channels identity",
    )?;

    let (schema_version, generation_files): (u8, &[&str]) =
        match job.compatibility.generation_format.as_str() {
            crate::RAW_GENERATION_FORMAT => (GENERATION_SCHEMA_VERSION, &RAW_GENERATION_FILES),
            "raw-index-schema-8-v1" | "raw-index-schema-8-v2" => (8, &LEGACY_SCHEMA_EIGHT_FILES),
            LEGACY_SCHEMA_SEVEN_V1_FORMAT => (7, &RAW_GENERATION_FILES),
            "raw-index-schema-6-v1" => (6, &LEGACY_SCHEMA_SIX_FILES),
            _ => {
                return Err(IndexStoreError::InvalidInput(
                    "unsupported persisted raw generation format".to_string(),
                ));
            }
        };
    let allowlisted = generation_files.iter().copied().collect::<BTreeSet<_>>();
    if !job
        .completed_files
        .iter()
        .all(|name| allowlisted.contains(name.as_str()))
    {
        return Err(IndexStoreError::InvalidInput(
            "remote publication state contains an unknown completed file".to_string(),
        ));
    }
    if let Some(manifest) = &job.manifest {
        validate_manifest_contract(manifest, schema_version, generation_files)?;
    }
    match job.status {
        RemoteJobStatus::Prepared if job.manifest.is_none() && job.completed_files.is_empty() => {}
        RemoteJobStatus::ManifestAccepted if job.manifest.is_some() => {}
        RemoteJobStatus::Activated
            if job.manifest.is_some()
                && job.completed_files
                    == allowlisted
                        .into_iter()
                        .map(str::to_string)
                        .collect::<BTreeSet<_>>() => {}
        _ => {
            return Err(IndexStoreError::InvalidInput(
                "remote publication status does not match its persisted artifacts".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_canonical_blake3(value: &str, label: &str) -> Result<(), IndexStoreError> {
    let parsed = blake3::Hash::from_hex(value).map_err(|_| {
        IndexStoreError::InvalidInput(format!("remote publication {label} is not a BLAKE3 digest"))
    })?;
    if parsed.to_hex().as_str() != value {
        return Err(IndexStoreError::InvalidInput(format!(
            "remote publication {label} is not canonical lowercase hex"
        )));
    }
    Ok(())
}

fn required_upload_space(manifest: &GenerationManifest) -> Result<u64, IndexStoreError> {
    manifest
        .files
        .values()
        .try_fold(MIN_FREE_AFTER_UPLOAD_BYTES, |total, file| {
            total.checked_add(file.bytes)
        })
        .ok_or_else(|| {
            IndexStoreError::InvalidInput("generation byte length overflows".to_string())
        })
}

fn allowlisted_file(file_name: &str) -> Result<&'static str, IndexStoreError> {
    RAW_GENERATION_FILES
        .into_iter()
        .find(|candidate| *candidate == file_name)
        .ok_or_else(|| {
            IndexStoreError::InvalidInput(
                "generation file name is not in the fixed allowlist".to_string(),
            )
        })
}

fn validate_job_id(job_id: &str) -> Result<(), IndexStoreError> {
    let valid = job_id.len() == 37
        && job_id.as_bytes()[20] == b'-'
        && job_id[..20].bytes().all(|byte| byte.is_ascii_digit())
        && job_id[21..].bytes().all(|byte| byte.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(IndexStoreError::InvalidInput(
            "invalid remote publication job id".to_string(),
        ))
    }
}

fn job_id(sequence: u64, snapshot_hash: &str, compatibility: &GenerationCompatibility) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"ytsearch remote publication job v1\0");
    hasher.update(&sequence.to_le_bytes());
    hasher.update(snapshot_hash.as_bytes());
    hasher.update(compatibility.generation_format.as_bytes());
    hasher.update(compatibility.corpus_identity.as_bytes());
    let digest = hasher.finalize().to_hex();
    format!("{sequence:020}-{}", &digest.as_str()[..16])
}

fn manifest_bytes(manifest: &GenerationManifest) -> Result<Vec<u8>, IndexStoreError> {
    serde_json::to_vec_pretty(manifest).map_err(map_json)
}

fn job_response(job: &PersistedJob) -> RemoteJobResponse {
    RemoteJobResponse {
        job_id: job.job_id.clone(),
        target_sequence: job.target_sequence,
        snapshot_bytes: job.snapshot_bytes,
        snapshot_blake3: job.snapshot_blake3.clone(),
        compatibility: job.compatibility.clone(),
        status: job.status.clone(),
        completed_files: job.completed_files.clone(),
    }
}

fn verify_file(path: &Path, expected: &GenerationFile) -> Result<(), IndexStoreError> {
    let metadata = fs::metadata(path).map_err(map_io)?;
    if !metadata.is_file() || metadata.len() != expected.bytes {
        return Err(IndexStoreError::Conflict(format!(
            "uploaded generation file {} has the wrong length",
            path.display()
        )));
    }
    let mut input = File::open(path).map_err(map_io)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut input).map_err(map_io)?;
    if hasher.finalize().to_hex().as_str() != expected.blake3 {
        return Err(IndexStoreError::Conflict(format!(
            "uploaded generation file {} has the wrong BLAKE3 digest",
            path.display()
        )));
    }
    Ok(())
}

fn verify_snapshot(path: &Path, job: &PersistedJob) -> Result<(), IndexStoreError> {
    let metadata = fs::metadata(path).map_err(map_io)?;
    if !metadata.is_file() || metadata.len() != job.snapshot_bytes {
        return Err(IndexStoreError::InvalidInput(
            "remote publication snapshot is missing or has the wrong length".to_string(),
        ));
    }
    let mut input = File::open(path).map_err(map_io)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut input).map_err(map_io)?;
    if hasher.finalize().to_hex().as_str() != job.snapshot_blake3 {
        return Err(IndexStoreError::InvalidInput(
            "remote publication snapshot has the wrong BLAKE3 digest".to_string(),
        ));
    }
    Ok(())
}

fn invalid_file(file_name: &str, message: &str) -> IndexStoreError {
    IndexStoreError::InvalidInput(format!("generation file {file_name} {message}"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), IndexStoreError> {
    let parent = path.parent().ok_or_else(|| {
        IndexStoreError::InvalidInput("remote publication path has no parent".to_string())
    })?;
    fs::create_dir_all(parent).map_err(map_io)?;
    let temporary = path.with_extension("new");
    {
        let mut output = BufWriter::new(File::create(&temporary).map_err(map_io)?);
        output.write_all(bytes).map_err(map_io)?;
        output.flush().map_err(map_io)?;
        output.get_ref().sync_all().map_err(map_io)?;
    }
    fs::rename(temporary, path).map_err(map_io)?;
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<(), IndexStoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(map_io)
}

fn map_io(error: std::io::Error) -> IndexStoreError {
    IndexStoreError::internal(BACKEND, error)
}

fn map_json(error: serde_json::Error) -> IndexStoreError {
    IndexStoreError::InvalidInput(format!("invalid remote publication JSON: {error}"))
}

#[cfg(test)]
mod tests;
