use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use super::DiscoveredVideo;

const MANIFEST_SCHEMA_VERSION: u8 = 1;
const RUN_ID_ENV: &str = "RECONCILIATION_RUN_ID";
static RUN_ID: OnceLock<String> = OnceLock::new();

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DiscoveryStatus {
    Complete,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct DiscoveryManifest {
    schema_version: u8,
    status: DiscoveryStatus,
    run_id: String,
    creator_id: String,
    handle: String,
    video_ids: Vec<String>,
}

pub(super) fn write_complete_discovery(
    transcript_cache_directory: &Path,
    creator_id: &str,
    handle: &str,
    videos: &[DiscoveredVideo],
) -> std::io::Result<PathBuf> {
    let directory = transcript_cache_directory.join(".reconciliation");
    fs::create_dir_all(&directory)?;

    let mut video_ids = videos
        .iter()
        .map(|video| video.video_id.clone())
        .collect::<Vec<_>>();
    video_ids.sort_unstable();
    video_ids.dedup();
    if video_ids.len() != videos.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("discovery for {handle} contains duplicate video IDs"),
        ));
    }

    let manifest = DiscoveryManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        status: DiscoveryStatus::Complete,
        run_id: reconciliation_run_id()?,
        creator_id: creator_id.to_string(),
        handle: handle.to_string(),
        video_ids,
    };
    let encoded = serde_json::to_vec_pretty(&manifest)?;
    let key = blake3::hash(creator_id.as_bytes()).to_hex();
    let path = directory.join(format!("{key}.json"));
    write_atomic(&directory, &path, &encoded)?;
    Ok(path)
}

pub(super) fn begin_catalog_run(transcript_cache_directory: &Path) -> std::io::Result<String> {
    let run_id = reconciliation_run_id()?;
    let directory = transcript_cache_directory.join(".reconciliation");
    fs::create_dir_all(&directory)?;
    remove_prior_creator_manifests(&directory)?;
    write_atomic(
        &directory,
        &directory.join("active-run-id"),
        run_id.as_bytes(),
    )?;
    Ok(run_id)
}

fn remove_prior_creator_manifests(directory: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(stem) = file_name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn reconciliation_run_id() -> std::io::Result<String> {
    if let Some(run_id) = RUN_ID.get() {
        return Ok(run_id.clone());
    }
    let run_id = std::env::var(RUN_ID_ENV).unwrap_or_else(|_| {
        format!(
            "catalog-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    });
    let valid = (8..=128).contains(&run_id.len())
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !valid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{RUN_ID_ENV} must be 8-128 ASCII identifier characters"),
        ));
    }
    let _ = RUN_ID.set(run_id.clone());
    Ok(run_id)
}

const BLOCKED_CREATOR_FILE: &str = "blocked-creator";

/// Orders a catalog pass to resume after the creator where the previous pass
/// was stopped by YouTube (bot challenge or rate limit). Without this, a pass
/// blocked at the same creator every time never reaches the creators listed
/// after it. The blocked creator moves to the end of the order.
pub(super) fn resume_order(
    transcript_cache_directory: &Path,
    handles: Vec<String>,
) -> std::io::Result<Vec<String>> {
    let path = transcript_cache_directory
        .join(".reconciliation")
        .join(BLOCKED_CREATOR_FILE);
    let blocked = match fs::read_to_string(&path) {
        Ok(value) => value.trim().to_string(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(handles),
        Err(error) => return Err(error),
    };
    Ok(rotate_after(handles, &blocked))
}

fn rotate_after(mut handles: Vec<String>, blocked: &str) -> Vec<String> {
    if let Some(index) = handles
        .iter()
        .position(|handle| handle.eq_ignore_ascii_case(blocked))
    {
        handles.rotate_left(index + 1);
    }
    handles
}

/// Records where a catalog pass was blocked, or clears it after a pass that
/// attempted every creator.
pub(super) fn remember_blocked_creator(
    transcript_cache_directory: &Path,
    handle: Option<&str>,
) -> std::io::Result<()> {
    let directory = transcript_cache_directory.join(".reconciliation");
    let path = directory.join(BLOCKED_CREATOR_FILE);
    match handle {
        Some(handle) => {
            fs::create_dir_all(&directory)?;
            write_atomic(&directory, &path, handle.as_bytes())
        }
        None => match fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

fn write_atomic(directory: &Path, path: &Path, body: &[u8]) -> std::io::Result<()> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(
        ".{}.tmp.{}.{nonce}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("reconciliation"),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(body)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    File::open(directory)?.sync_all()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_blocked_pass_resumes_after_the_blocked_creator() {
        let root = tempfile::TempDir::new().expect("temp dir");
        let handles = ["@a", "@b", "@c", "@d"].map(String::from).to_vec();
        assert_eq!(
            super::resume_order(root.path(), handles.clone()).expect("order"),
            handles
        );
        super::remember_blocked_creator(root.path(), Some("@B")).expect("remember");
        assert_eq!(
            super::resume_order(root.path(), handles.clone()).expect("order"),
            ["@c", "@d", "@a", "@b"]
        );
        super::remember_blocked_creator(root.path(), None).expect("clear");
        assert_eq!(
            super::resume_order(root.path(), handles.clone()).expect("order"),
            handles
        );
        super::remember_blocked_creator(root.path(), Some("@gone")).expect("remember");
        assert_eq!(
            super::resume_order(root.path(), handles.clone()).expect("order"),
            handles,
            "a creator no longer listed leaves the order unchanged"
        );
    }

    use super::*;

    fn temporary_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ytsearch-reconciliation-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    #[test]
    fn complete_discovery_is_sorted_and_atomically_replaced() {
        let root = temporary_directory("replace");
        let run_id = begin_catalog_run(&root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".reconciliation/active-run-id"))
                .unwrap()
                .trim(),
            run_id
        );
        let first = vec![
            DiscoveredVideo {
                video_id: "video-b".to_string(),
                title: "B".to_string(),
            },
            DiscoveredVideo {
                video_id: "video-a".to_string(),
                title: "A".to_string(),
            },
        ];
        let path = write_complete_discovery(&root, "creator", "@Creator", &first).unwrap();
        let manifest: DiscoveryManifest =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(manifest.status, DiscoveryStatus::Complete);
        assert_eq!(manifest.run_id, run_id);
        assert_eq!(manifest.video_ids, ["video-a", "video-b"]);

        let second = vec![DiscoveredVideo {
            video_id: "video-c".to_string(),
            title: "C".to_string(),
        }];
        assert_eq!(
            write_complete_discovery(&root, "creator", "@Creator", &second).unwrap(),
            path
        );
        let manifest: DiscoveryManifest =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(manifest.video_ids, ["video-c"]);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_discovered_ids_fail_before_replacing_evidence() {
        let root = temporary_directory("duplicates");
        let duplicate = vec![
            DiscoveredVideo {
                video_id: "same".to_string(),
                title: "first".to_string(),
            },
            DiscoveredVideo {
                video_id: "same".to_string(),
                title: "second".to_string(),
            },
        ];
        assert!(write_complete_discovery(&root, "creator", "@creator", &duplicate).is_err());
        assert!(
            !root
                .join(".reconciliation")
                .read_dir()
                .unwrap()
                .any(|_| true)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_run_removes_only_prior_creator_manifests() {
        let root = temporary_directory("catalog-cleanup");
        let directory = root.join(".reconciliation");
        fs::create_dir_all(&directory).unwrap();
        let stale_manifest = directory.join(format!("{}.json", "a".repeat(64)));
        let operator_file = directory.join("operator-notes.json");
        let unrelated_directory = directory.join(format!("{}.json", "b".repeat(64)));
        fs::write(&stale_manifest, b"stale").unwrap();
        fs::write(&operator_file, b"preserve").unwrap();
        fs::create_dir(&unrelated_directory).unwrap();

        begin_catalog_run(&root).unwrap();

        assert!(!stale_manifest.exists());
        assert_eq!(fs::read(&operator_file).unwrap(), b"preserve");
        assert!(unrelated_directory.is_dir());
        assert!(directory.join("active-run-id").is_file());
        let _ = fs::remove_dir_all(root);
    }
}
