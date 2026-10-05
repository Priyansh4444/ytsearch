use crate::CliError;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use youtube_index_store::{
    RAW_GENERATION_FILES, RawGenerationCompatibility, RawGenerationManifest, RawPublicationJob,
};
use youtube_raw_search::{GenerationBuilder, MutationStore, RawSearchEngine};

const RECEIPT: &str = "verified-build.json";
const INPUTS: &str = "build-inputs.json";

pub(super) fn reconcile_cleanup(
    root: &Path,
    confirmation: Result<RawPublicationJob, youtube_index_store::IndexStoreError>,
) -> Result<(), CliError> {
    match confirmation {
        Ok(job) => cleanup_activated_build(root, &job),
        Err(_) => {
            eprintln!("Raw publication cleanup unconfirmed; preserving local build and continuing");
            Ok(())
        }
    }
}

pub(super) enum Workspace {
    Build,
    Resume,
}

pub(super) fn prepare_workspace(
    root: &Path,
    job: &RawPublicationJob,
) -> Result<Workspace, CliError> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(root)?,
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return Err(CliError::Message(
                "publication workspace is not a real directory".into(),
            ));
        }
    }
    if root.join(RECEIPT).try_exists()? {
        return Ok(Workspace::Resume);
    }
    let input_path = root.join(INPUTS);
    let expected = Receipt::for_job(job, String::new());
    if input_path.try_exists()? {
        if read_receipt(&input_path, "invalid publication inputs")? != expected {
            return Err(CliError::Message(
                "publication workspace belongs to different inputs".into(),
            ));
        }
        // Only this protocol's incomplete artifacts are eligible for retry cleanup.
        let entries = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
        for entry in &entries {
            let name = entry.file_name();
            let allowed = match name.to_str() {
                Some("generation") => entry.file_type()?.is_dir(),
                Some(
                    "build-inputs.json"
                    | "mutations.sqlite3"
                    | "mutations.download"
                    | "verified-build.next",
                ) => entry.file_type()?.is_file(),
                _ => false,
            };
            if !allowed {
                return Err(CliError::Message(format!(
                    "unrecognized incomplete publication artifact: {}",
                    entry.path().display()
                )));
            }
        }
        for entry in entries {
            if entry.file_name() == INPUTS {
                continue;
            }
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
        fs::File::open(root)?.sync_all()?;
    } else {
        // A crash while initializing can leave only an unfinished input record.
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.file_name() != "build-inputs.next" || !entry.file_type()?.is_file() {
                return Err(CliError::Message(format!(
                    "publication workspace {} lacks input provenance; preserved for recovery",
                    root.display()
                )));
            }
            fs::remove_file(entry.path())?;
        }
        write_receipt(&input_path, &root.join("build-inputs.next"), &expected)?;
        fs::File::open(
            root.parent()
                .ok_or_else(|| CliError::Message("publication workspace has no parent".into()))?,
        )?
        .sync_all()?;
    }
    Ok(Workspace::Build)
}

pub(super) fn recover_cleanup_and_list_completed_jobs(
    root: &Path,
) -> Result<Vec<String>, CliError> {
    let mut jobs = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if entry.file_type()?.is_dir()
            && name
                .to_str()
                .and_then(|name| name.strip_prefix(".reclaim-"))
                .is_some_and(crate::is_publication_job_id)
        {
            fs::remove_dir_all(entry.path())?;
            fs::File::open(root)?.sync_all()?;
            continue;
        }
        if !entry.file_type()?.is_dir() || !name.to_str().is_some_and(crate::is_publication_job_id)
        {
            continue;
        }
        if entry.path().join(RECEIPT).try_exists()? {
            jobs.push(name.to_string_lossy().into_owned());
        }
    }
    Ok(jobs)
}

pub(super) fn cleanup_activated_build(
    root: &Path,
    confirmed: &RawPublicationJob,
) -> Result<(), CliError> {
    if !crate::is_publication_job_id(&confirmed.job_id)
        || confirmed.job_id[..20].parse::<u64>().ok() != Some(confirmed.target_sequence)
        || confirmed.status != youtube_index_store::RawPublicationStatus::Activated
    {
        return Ok(());
    }
    let path = root.join(&confirmed.job_id);
    if !path.try_exists()? {
        return Ok(());
    }
    if !fs::symlink_metadata(&path)?.file_type().is_dir() {
        return Err(CliError::Message(
            "completed publication workspace is not a real directory".into(),
        ));
    }
    let receipt = read_receipt(&path.join(RECEIPT), "invalid completed publication receipt")?;
    if receipt != Receipt::for_job(confirmed, manifest_hash(&path)?) {
        return Ok(());
    }
    let reclaim = root.join(format!(".reclaim-{}", receipt.job_id));
    if fs::symlink_metadata(&reclaim).is_ok() {
        return Err(CliError::Message(
            "publication cleanup destination already exists; retry cleanup".into(),
        ));
    }
    fs::rename(&path, &reclaim)?;
    fs::File::open(root)?.sync_all()?;
    fs::remove_dir_all(&reclaim)?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u8,
    job_id: String,
    target_sequence: u64,
    snapshot_bytes: u64,
    snapshot_blake3: String,
    compatibility: RawGenerationCompatibility,
    manifest_blake3: String,
}

impl Receipt {
    fn for_job(job: &RawPublicationJob, manifest_blake3: String) -> Self {
        Self {
            version: 1,
            job_id: job.job_id.clone(),
            target_sequence: job.target_sequence,
            snapshot_bytes: job.snapshot_bytes,
            snapshot_blake3: job.snapshot_blake3.clone(),
            compatibility: job.compatibility.clone(),
            manifest_blake3,
        }
    }
}

fn hash_regular_file(path: &Path) -> Result<String, CliError> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(CliError::Message(format!(
            "publication artifact is not a regular file: {}",
            path.display()
        )));
    }
    let mut hash = blake3::Hasher::new();
    hash.update_reader(fs::File::open(path)?)?;
    Ok(hash.finalize().to_hex().to_string())
}

/// Reads a receipt only from a regular file, never through a symlink.
fn read_receipt(path: &Path, invalid: &str) -> Result<Receipt, CliError> {
    hash_regular_file(path)?;
    serde_json::from_reader(fs::File::open(path)?)
        .map_err(|error| CliError::Message(format!("{invalid}: {error}")))
}

fn write_receipt(path: &Path, pending: &Path, receipt: &Receipt) -> Result<(), CliError> {
    let encoded =
        serde_json::to_vec(receipt).map_err(|error| CliError::Message(error.to_string()))?;
    Ok(crate::atomic_file::write(path, pending, &encoded)?)
}

fn manifest_hash(root: &Path) -> Result<String, CliError> {
    hash_regular_file(&root.join("generation/manifest.json"))
}

/// Called only after the detached builder has completed its full verification.
pub(super) fn record_verified_build(root: &Path, job: &RawPublicationJob) -> Result<(), CliError> {
    write_receipt(
        &root.join(RECEIPT),
        &root.join("verified-build.next"),
        &Receipt::for_job(job, manifest_hash(root)?),
    )
}

pub(super) fn resume_verified_build(
    root: &Path,
    job: &RawPublicationJob,
    builder: &GenerationBuilder,
) -> Result<RawGenerationManifest, CliError> {
    if !fs::symlink_metadata(root)?.file_type().is_dir()
        || !fs::symlink_metadata(root.join("generation"))?
            .file_type()
            .is_dir()
    {
        return Err(CliError::Message(
            "publication workspace must contain real directories".into(),
        ));
    }
    let receipt_path = root.join(RECEIPT);
    if !receipt_path.try_exists()? {
        return Err(CliError::Message(format!(
            "publication workspace {} has no verified build receipt; preserved for recovery, not safe to upload",
            root.display()
        )));
    }
    if read_receipt(&receipt_path, "invalid publication receipt")?
        != Receipt::for_job(job, manifest_hash(root)?)
    {
        return Err(CliError::Message(
            "publication receipt does not match the prepared job and manifest".into(),
        ));
    }
    let snapshot = root.join("mutations.sqlite3");
    if hash_regular_file(&snapshot)? != job.snapshot_blake3
        || fs::metadata(&snapshot)?.len() != job.snapshot_bytes
        || MutationStore::open_snapshot_read_only(&snapshot)?.last_sequence()?
            != job.target_sequence
        || builder.compatibility()? != job.compatibility
    {
        return Err(CliError::Message(
            "publication resume inputs do not match the verified build".into(),
        ));
    }
    let generation = root.join("generation");
    for name in RAW_GENERATION_FILES {
        if !fs::symlink_metadata(generation.join(name))?
            .file_type()
            .is_file()
        {
            return Err(CliError::Message(format!(
                "publication generation file {name} is not regular"
            )));
        }
    }
    drop(RawSearchEngine::open(&generation).map_err(|error| {
        CliError::Message(format!("publication resume verification failed: {error}"))
    })?);
    serde_json::from_reader(fs::File::open(generation.join("manifest.json"))?)
        .map_err(|error| CliError::Message(format!("invalid publication manifest: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::collections::BTreeSet;
    use youtube_index_store::RawPublicationStatus;

    fn fixture() -> (tempfile::TempDir, RawPublicationJob, GenerationBuilder) {
        let root = tempfile::tempdir().unwrap();
        let chunks = root.path().join("chunks.gz");
        let channels = root.path().join("channels.gz");
        for path in [&chunks, &channels] {
            GzEncoder::new(fs::File::create(path).unwrap(), Compression::fast())
                .finish()
                .unwrap();
        }
        let builder = GenerationBuilder::new(chunks, channels);
        let snapshot = root.path().join("mutations.sqlite3");
        drop(MutationStore::open(&snapshot).unwrap());
        let job = RawPublicationJob {
            job_id: "00000000000000000000-0123456789abcdef".into(),
            target_sequence: 0,
            snapshot_bytes: fs::metadata(&snapshot).unwrap().len(),
            snapshot_blake3: hash_regular_file(&snapshot).unwrap(),
            compatibility: builder.compatibility().unwrap(),
            status: RawPublicationStatus::Prepared,
            completed_files: BTreeSet::new(),
        };
        let snapshot = MutationStore::open_snapshot_read_only(&snapshot).unwrap();
        builder
            .build(
                &snapshot,
                &job.compatibility,
                &root.path().join("generation"),
            )
            .unwrap();
        (root, job, builder)
    }

    #[test]
    fn verified_build_resumes_repeatedly_without_rebuilding() {
        let (root, mut job, builder) = fixture();
        record_verified_build(root.path(), &job).unwrap();
        let first = resume_verified_build(root.path(), &job, &builder).unwrap();
        job.status = RawPublicationStatus::ManifestAccepted;
        job.completed_files.insert("documents.bin".into());
        assert_eq!(
            first,
            resume_verified_build(root.path(), &job, &builder).unwrap()
        );
    }

    #[test]
    fn legacy_build_is_preserved_but_not_adopted() {
        let (root, job, builder) = fixture();
        assert!(
            resume_verified_build(root.path(), &job, &builder)
                .unwrap_err()
                .to_string()
                .contains("no verified build receipt")
        );
        assert!(root.path().join("generation/manifest.json").exists());
        assert!(!root.path().join(RECEIPT).exists());
    }

    #[test]
    fn receipt_binds_every_input_and_manifest() {
        let (root, job, builder) = fixture();
        record_verified_build(root.path(), &job).unwrap();
        for field in 0..6 {
            let mut changed = job.clone();
            match field {
                0 => changed.job_id.push('0'),
                1 => changed.target_sequence += 1,
                2 => changed.snapshot_bytes += 1,
                3 => changed.snapshot_blake3.push('0'),
                4 => changed.compatibility.corpus_identity.push('0'),
                _ => changed.compatibility.generation_format.push('0'),
            }
            assert!(resume_verified_build(root.path(), &changed, &builder).is_err());
        }
        fs::write(root.path().join("generation/manifest.json"), b"{}").unwrap();
        assert!(resume_verified_build(root.path(), &job, &builder).is_err());
    }

    #[test]
    fn verified_receipt_does_not_hide_corrupted_output() {
        let (root, job, builder) = fixture();
        record_verified_build(root.path(), &job).unwrap();
        fs::write(root.path().join("generation/postings.bin"), b"corrupt").unwrap();
        assert!(resume_verified_build(root.path(), &job, &builder).is_err());
    }

    #[test]
    fn changed_snapshot_is_rejected_even_with_an_unchanged_receipt() {
        let (root, job, builder) = fixture();
        record_verified_build(root.path(), &job).unwrap();
        fs::write(
            root.path().join("mutations.sqlite3"),
            b"not the captured snapshot",
        )
        .unwrap();
        assert!(resume_verified_build(root.path(), &job, &builder).is_err());
    }

    #[test]
    fn unfinished_receipt_is_not_a_completion_record() {
        let (root, job, builder) = fixture();
        fs::write(root.path().join("verified-build.next"), b"{}").unwrap();
        assert!(resume_verified_build(root.path(), &job, &builder).is_err());
        assert!(root.path().join("generation/manifest.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_generation_is_rejected() {
        let (root, job, builder) = fixture();
        record_verified_build(root.path(), &job).unwrap();
        fs::rename(root.path().join("generation"), root.path().join("original")).unwrap();
        std::os::unix::fs::symlink("original", root.path().join("generation")).unwrap();
        assert!(resume_verified_build(root.path(), &job, &builder).is_err());
    }

    #[test]
    fn interrupted_download_and_build_can_retry_without_touching_other_jobs() {
        let (fixture_root, job, _) = fixture();
        let root = fixture_root.path().join("retry");
        assert!(matches!(
            prepare_workspace(&root, &job).unwrap(),
            Workspace::Build
        ));
        fs::write(root.join("mutations.download"), b"partial download").unwrap();
        assert!(matches!(
            prepare_workspace(&root, &job).unwrap(),
            Workspace::Build
        ));
        assert!(!root.join("mutations.download").exists());
        fs::create_dir(root.join("generation")).unwrap();
        fs::write(root.join("generation/partial"), b"partial build").unwrap();
        assert!(matches!(
            prepare_workspace(&root, &job).unwrap(),
            Workspace::Build
        ));
        assert!(!root.join("generation").exists());
        assert!(
            fixture_root
                .path()
                .join("generation/manifest.json")
                .exists()
        );
    }

    #[test]
    fn retry_preserves_legacy_or_mismatched_work() {
        let (root, job, _) = fixture();
        assert!(prepare_workspace(root.path(), &job).is_err());
        assert!(root.path().join("generation/manifest.json").exists());
        let retry = root.path().join("retry");
        prepare_workspace(&retry, &job).unwrap();
        fs::write(retry.join("mutations.download"), b"keep").unwrap();
        let mut changed = job.clone();
        changed.target_sequence += 1;
        assert!(prepare_workspace(&retry, &changed).is_err());
        assert!(retry.join("mutations.download").exists());
    }

    #[test]
    fn confirmed_activation_reclaims_completed_work_idempotently() {
        let (root, mut job, _) = fixture();
        record_verified_build(root.path(), &job).unwrap();
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join(&job.job_id);
        fs::rename(root.path(), &target).unwrap();
        cleanup_activated_build(parent.path(), &job).unwrap();
        assert!(target.exists());
        job.status = RawPublicationStatus::Activated;
        let mut other_corpus = job.clone();
        other_corpus.compatibility.corpus_identity.push('0');
        cleanup_activated_build(parent.path(), &other_corpus).unwrap();
        assert!(target.exists());
        let mut other_sequence = job.clone();
        other_sequence.target_sequence += 1;
        cleanup_activated_build(parent.path(), &other_sequence).unwrap();
        assert!(target.exists());
        cleanup_activated_build(parent.path(), &job).unwrap();
        assert!(!target.exists());
        cleanup_activated_build(parent.path(), &job).unwrap();
    }

    #[test]
    fn interrupted_reclamation_finishes_even_after_receipt_was_removed() {
        let parent = tempfile::tempdir().unwrap();
        let reclaim = parent
            .path()
            .join(".reclaim-00000000000000000001-0123456789abcdef");
        fs::create_dir(&reclaim).unwrap();
        fs::write(reclaim.join("remaining.bin"), b"remaining output").unwrap();
        recover_cleanup_and_list_completed_jobs(parent.path()).unwrap();
        assert!(!reclaim.exists());
    }

    #[test]
    fn unavailable_old_job_preserves_work_without_blocking_new_build() {
        let (root, job, _) = fixture();
        record_verified_build(root.path(), &job).unwrap();
        let parent = tempfile::tempdir().unwrap();
        let old = parent.path().join(&job.job_id);
        fs::rename(root.path(), &old).unwrap();
        assert_eq!(
            recover_cleanup_and_list_completed_jobs(parent.path()).unwrap(),
            vec![job.job_id.clone()]
        );
        for error in [
            youtube_index_store::IndexStoreError::InvalidInput(
                "unknown remote publication job".into(),
            ),
            youtube_index_store::IndexStoreError::InvalidInput("unauthorized".into()),
            youtube_index_store::IndexStoreError::unavailable("raw", "connection failed"),
        ] {
            reconcile_cleanup(parent.path(), Err(error)).unwrap();
            assert!(old.join(RECEIPT).exists());
        }
        let mut next = job.clone();
        next.job_id = "00000000000000000001-0123456789abcdef".into();
        next.target_sequence = 1;
        assert!(matches!(
            prepare_workspace(&parent.path().join(&next.job_id), &next).unwrap(),
            Workspace::Build
        ));
        assert!(old.exists());
    }
}
