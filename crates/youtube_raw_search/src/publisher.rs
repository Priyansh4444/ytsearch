use crate::{
    MutationStore, RawSearchEngine,
    cache_io::ColdReader,
    engine::{WarmEngine, build_publication_candidate},
    mutable::{MutationSnapshot, VideoChunkStream},
};
use flate2::read::MultiGzDecoder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};
use youtube_index_store::{ChannelDocument, IndexStoreError, RawGenerationCompatibility};

const BACKEND: &str = "raw";
const ACTIVE_MANIFEST: &str = "active-generation.json";
const GENERATION_MANIFEST: &str = "manifest.json";
const REMOTE_PUBLICATION_WORK: &str = "work/remote-publication";
// Publication builds schema 9 (schema 7 plus accelerators). The reader still
// opens schema-7 generations, so an active schema-7 generation keeps serving
// until the first schema-9 publication replaces it.
pub const RAW_GENERATION_FORMAT: &str = "raw-index-schema-9-v1";
const CORPUS_IDENTITY_DOMAIN: &[u8] = b"ytsearch raw corpus identity v1\0";

#[derive(Clone, Debug)]
pub struct PublisherPaths {
    pub base_chunks_gzip: PathBuf,
    pub base_channels_gzip: PathBuf,
    pub base_index: PathBuf,
    pub state_directory: PathBuf,
}

pub type GenerationCompatibility = RawGenerationCompatibility;

#[derive(Clone, Debug)]
pub struct GenerationBuilder {
    base_chunks_gzip: PathBuf,
    base_channels_gzip: PathBuf,
}

#[derive(Clone, Debug)]
pub struct DetachedGeneration {
    pub sequence: u64,
    pub path: PathBuf,
}

pub struct Publisher {
    paths: PublisherPaths,
    compatibility: OnceLock<GenerationCompatibility>,
}

#[derive(Deserialize, Serialize)]
struct ActiveGeneration {
    sequence: u64,
    directory: String,
}

#[derive(Deserialize)]
struct ChunkKey {
    video_id: String,
    youtuber_id: String,
}

impl GenerationBuilder {
    pub fn new(base_chunks_gzip: PathBuf, base_channels_gzip: PathBuf) -> Self {
        Self {
            base_chunks_gzip,
            base_channels_gzip,
        }
    }

    pub fn compatibility(&self) -> Result<GenerationCompatibility, IndexStoreError> {
        validate_source(&self.base_chunks_gzip)?;
        validate_source(&self.base_channels_gzip)?;
        let chunks = hash_source_archive(&self.base_chunks_gzip)?;
        let channels = hash_source_archive(&self.base_channels_gzip)?;
        let mut corpus = blake3::Hasher::new();
        corpus.update(CORPUS_IDENTITY_DOMAIN);
        update_identity_component(
            &mut corpus,
            b"generation-format",
            RAW_GENERATION_FORMAT.as_bytes(),
        );
        update_identity_component(&mut corpus, b"base-chunks", chunks.as_bytes());
        update_identity_component(&mut corpus, b"base-channels", channels.as_bytes());
        Ok(GenerationCompatibility {
            generation_format: RAW_GENERATION_FORMAT.to_string(),
            corpus_identity: corpus.finalize().to_hex().to_string(),
            base_chunks_blake3: chunks.to_hex().to_string(),
            base_channels_blake3: channels.to_hex().to_string(),
        })
    }

    /// Builds a verified immutable generation from a captured mutation store.
    ///
    /// The source archives must remain immutable from compatibility validation
    /// through the end of the build. Production satisfies that contract by
    /// shipping them in a read-only image layer. This method never activates
    /// the output and never changes operation state in `snapshot_store`.
    pub fn build(
        &self,
        snapshot_store: &MutationStore,
        expected: &GenerationCompatibility,
        output_path: &Path,
    ) -> Result<DetachedGeneration, IndexStoreError> {
        let compatibility = self.compatibility()?;
        if &compatibility != expected {
            return Err(IndexStoreError::InvalidInput(format!(
                "raw generation compatibility mismatch: expected format={} corpus={}, got format={} corpus={}",
                expected.generation_format,
                expected.corpus_identity,
                compatibility.generation_format,
                compatibility.corpus_identity
            )));
        }
        let sequence = snapshot_store.with_snapshot(|snapshot, video_chunks| {
            self.build_snapshot(snapshot, video_chunks, output_path)?;
            Ok(snapshot.through_sequence)
        })?;
        sync_directory_tree(output_path)?;
        // Detached output crosses a process and storage boundary, so it must
        // pass full manifest hashes.
        drop(RawSearchEngine::open(output_path).map_err(map_raw)?);
        Ok(DetachedGeneration {
            sequence,
            path: output_path.to_path_buf(),
        })
    }

    fn build_snapshot(
        &self,
        snapshot: &MutationSnapshot,
        video_chunks: &mut VideoChunkStream<'_>,
        candidate_path: &Path,
    ) -> Result<(), IndexStoreError> {
        validate_source(&self.base_chunks_gzip)?;
        validate_source(&self.base_channels_gzip)?;
        let video_mutations = &snapshot.video_ids;
        let cascaded_channels = snapshot
            .channels
            .iter()
            .filter(|mutation| {
                mutation.document.is_none() && mutation.deletion_scope.cascades_videos()
            })
            .map(|mutation| mutation.channel_id.as_str())
            .collect::<HashSet<_>>();

        let base_documents = gzip_lines(&self.base_chunks_gzip)?.filter_map(|line| {
            let line = match line {
                Ok(line) => line,
                Err(error) => return Some(Err(crate::Error::Io(error))),
            };
            let key = match serde_json::from_str::<ChunkKey>(&line) {
                Ok(key) => key,
                Err(error) => return Some(Err(crate::Error::Json(error))),
            };
            if video_mutations.contains(key.video_id.as_str())
                || cascaded_channels.contains(key.youtuber_id.as_str())
            {
                None
            } else {
                Some(Ok(line))
            }
        });
        // One video's chunks are decoded at a time, in the same video-id order
        // the materialized snapshot used, so the builder input is unchanged.
        let replacement_documents = video_chunks.flat_map(|chunks| {
            let chunks = match chunks {
                Ok(chunks) => chunks.unwrap_or_default(),
                Err(error) => {
                    return vec![Err(crate::Error::Io(std::io::Error::other(error)))];
                }
            };
            if chunks
                .first()
                .is_some_and(|chunk| cascaded_channels.contains(chunk.youtuber_id()))
            {
                return Vec::new();
            }
            chunks
                .iter()
                .map(|chunk| serde_json::to_string(chunk).map_err(crate::Error::Json))
                .collect()
        });

        let mut channels = BTreeMap::<String, ChannelDocument>::new();
        for line in gzip_lines(&self.base_channels_gzip)? {
            let channel: ChannelDocument =
                serde_json::from_str(&line.map_err(map_io)?).map_err(map_json)?;
            channels.insert(channel.id.clone(), channel);
        }
        for mutation in &snapshot.channels {
            match &mutation.document {
                Some(document) => {
                    channels.insert(mutation.channel_id.clone(), document.clone());
                }
                None => {
                    channels.remove(&mutation.channel_id);
                }
            }
        }
        build_publication_candidate(
            base_documents.chain(replacement_documents),
            channels
                .into_values()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_json)?,
            candidate_path,
        )
        .map_err(map_raw)
    }
}

impl Publisher {
    pub fn new(paths: PublisherPaths) -> Self {
        Self {
            paths,
            compatibility: OnceLock::new(),
        }
    }

    fn active_index_path(&self) -> Result<PathBuf, IndexStoreError> {
        let manifest_path = self.paths.state_directory.join(ACTIVE_MANIFEST);
        if !manifest_path.exists() {
            return Ok(self.paths.base_index.clone());
        }
        let manifest: ActiveGeneration =
            serde_json::from_reader(File::open(&manifest_path).map_err(map_io)?)
                .map_err(map_json)?;
        let path = self.paths.state_directory.join(&manifest.directory);
        ensure_generation_path(&self.paths.state_directory, &path)?;
        Ok(path)
    }

    fn active_sequence(&self) -> Result<u64, IndexStoreError> {
        let manifest_path = self.paths.state_directory.join(ACTIVE_MANIFEST);
        if !manifest_path.exists() {
            return Ok(0);
        }
        serde_json::from_reader::<_, ActiveGeneration>(File::open(manifest_path).map_err(map_io)?)
            .map(|manifest| manifest.sequence)
            .map_err(map_json)
    }

    /// Opens the durable active generation and reconciles its sequence with
    /// the mutation WAL. Activation persists the generation pointer before it
    /// acknowledges operations, so this closes the crash window between those
    /// two steps without rebuilding or replaying an already-active snapshot.
    pub fn open_active_generation(
        &self,
        store: &MutationStore,
    ) -> Result<(RawSearchEngine, u64), IndexStoreError> {
        let sequence = self.active_sequence()?;
        let engine = RawSearchEngine::open(&self.active_index_path()?).map_err(map_raw)?;
        store.mark_published(sequence)?;
        Ok((engine, sequence))
    }

    pub(crate) fn state_directory(&self) -> &Path {
        &self.paths.state_directory
    }

    pub(crate) fn generation_compatibility(
        &self,
    ) -> Result<GenerationCompatibility, IndexStoreError> {
        if let Some(compatibility) = self.compatibility.get() {
            return Ok(compatibility.clone());
        }
        let compatibility = GenerationBuilder::new(
            self.paths.base_chunks_gzip.clone(),
            self.paths.base_channels_gzip.clone(),
        )
        .compatibility()?;
        let _ = self.compatibility.set(compatibility.clone());
        Ok(self.compatibility.get().cloned().unwrap_or(compatibility))
    }

    /// Installs a detached generation without acknowledging its mutation WAL.
    ///
    /// The caller must swap the returned engine into `ReloadableEngine` before
    /// marking the corresponding mutations published. `expected_manifest`
    /// makes an activation retry exact even when the first attempt already
    /// moved the incoming directory into its durable generation path.
    pub(crate) fn install_verified_generation(
        &self,
        sequence: u64,
        incoming_path: &Path,
        expected_manifest: &[u8],
    ) -> Result<WarmEngine, IndexStoreError> {
        if sequence == 0 {
            return Err(IndexStoreError::InvalidInput(
                "raw generation sequence must be positive".to_string(),
            ));
        }
        let generation_name = format!("generations/{sequence:020}");
        let final_path = self.paths.state_directory.join(&generation_name);
        ensure_generation_path(&self.paths.state_directory, &final_path)?;
        let active_sequence = self.active_sequence()?;
        if sequence < active_sequence {
            return Err(IndexStoreError::Conflict(format!(
                "raw generation {sequence} is older than active generation {active_sequence}"
            )));
        }
        if sequence == active_sequence {
            return open_exact_generation(sequence, final_path, expected_manifest);
        }

        fs::create_dir_all(self.paths.state_directory.join("generations")).map_err(map_io)?;
        if final_path.exists() {
            let existing = open_exact_generation(sequence, final_path, expected_manifest)?;
            self.activate(sequence, &generation_name)?;
            self.remove_old_generations(sequence)?;
            return Ok(existing);
        }

        validate_remote_incoming_path(&self.paths.state_directory, incoming_path)?;
        let actual_manifest = fs::read(incoming_path.join(GENERATION_MANIFEST)).map_err(map_io)?;
        if actual_manifest != expected_manifest {
            return Err(IndexStoreError::Conflict(
                "detached generation manifest does not match the accepted manifest".to_string(),
            ));
        }
        // The authenticated receiver already length-checks, BLAKE3-verifies,
        // and fsyncs every streamed file. Avoid rereading the full generation
        // while the old mmap is serving queries. Startup and recovery continue
        // to use the full-checksum `RawSearchEngine::open` path.
        let engine = RawSearchEngine::open_verified_transfer(incoming_path)
            .map_err(map_raw)?
            .warmed_for_activation();

        fs::rename(incoming_path, &final_path).map_err(map_io)?;
        sync_directory(&self.paths.state_directory.join("generations"))?;
        self.activate(sequence, &generation_name)?;
        self.remove_old_generations(sequence)?;
        Ok(engine)
    }

    fn activate(&self, sequence: u64, directory: &str) -> Result<(), IndexStoreError> {
        let manifest = ActiveGeneration {
            sequence,
            directory: directory.to_string(),
        };
        let temporary = self
            .paths
            .state_directory
            .join(format!(".{ACTIVE_MANIFEST}.new"));
        let final_path = self.paths.state_directory.join(ACTIVE_MANIFEST);
        {
            let mut output = BufWriter::new(File::create(&temporary).map_err(map_io)?);
            serde_json::to_writer(&mut output, &manifest).map_err(map_json)?;
            output.write_all(b"\n").map_err(map_io)?;
            output.flush().map_err(map_io)?;
            output.get_ref().sync_all().map_err(map_io)?;
        }
        fs::rename(&temporary, final_path).map_err(map_io)?;
        sync_directory(&self.paths.state_directory)
    }

    fn remove_old_generations(&self, current_sequence: u64) -> Result<(), IndexStoreError> {
        let generations = self.paths.state_directory.join("generations");
        let mut completed = fs::read_dir(&generations)
            .map_err(map_io)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let sequence = name.parse::<u64>().ok()?;
                (sequence != current_sequence).then_some((sequence, entry.path()))
            })
            .collect::<Vec<_>>();
        completed.sort_unstable_by_key(|(sequence, _)| *sequence);
        let remove_count = completed.len().saturating_sub(1);
        for (_, path) in completed.into_iter().take(remove_count) {
            ensure_generation_path(&self.paths.state_directory, &path)?;
            fs::remove_dir_all(path).map_err(map_io)?;
        }
        Ok(())
    }
}

fn open_exact_generation(
    sequence: u64,
    path: PathBuf,
    expected_manifest: &[u8],
) -> Result<WarmEngine, IndexStoreError> {
    let actual_manifest = fs::read(path.join(GENERATION_MANIFEST)).map_err(map_io)?;
    if actual_manifest != expected_manifest {
        return Err(IndexStoreError::Conflict(format!(
            "generation {sequence} exists with a different manifest"
        )));
    }
    Ok(RawSearchEngine::open(&path)
        .map_err(map_raw)?
        .warmed_for_activation())
}

fn validate_remote_incoming_path(root: &Path, path: &Path) -> Result<(), IndexStoreError> {
    let remote_root = root.join(REMOTE_PUBLICATION_WORK);
    ensure_generation_path(root, &remote_root)?;
    if path == remote_root || !path.starts_with(&remote_root) {
        return Err(IndexStoreError::InvalidInput(format!(
            "detached generation is outside the remote publication work directory: {}",
            path.display()
        )));
    }
    let canonical_root = fs::canonicalize(&remote_root).map_err(map_io)?;
    let canonical_path = fs::canonicalize(path).map_err(map_io)?;
    if canonical_path == canonical_root || !canonical_path.starts_with(&canonical_root) {
        return Err(IndexStoreError::InvalidInput(format!(
            "detached generation is outside the remote publication work directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn hash_source_archive(path: &Path) -> Result<blake3::Hash, IndexStoreError> {
    let mut input = ColdReader::open(path).map_err(map_io)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut input).map_err(map_io)?;
    Ok(hasher.finalize())
}

fn update_identity_component(hasher: &mut blake3::Hasher, name: &[u8], value: &[u8]) {
    hasher.update(&(name.len() as u64).to_le_bytes());
    hasher.update(name);
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn gzip_lines(
    path: &Path,
) -> Result<impl Iterator<Item = std::io::Result<String>>, IndexStoreError> {
    let input = ColdReader::open(path).map_err(map_io)?;
    Ok(BufReader::new(MultiGzDecoder::new(input)).lines())
}

fn validate_source(path: &Path) -> Result<(), IndexStoreError> {
    if !path.exists() {
        return Err(IndexStoreError::Unavailable {
            backend: BACKEND,
            message: format!("required publisher source is missing: {}", path.display()),
        });
    }
    Ok(())
}

fn sync_directory_tree(path: &Path) -> Result<(), IndexStoreError> {
    for entry in fs::read_dir(path).map_err(map_io)? {
        let entry = entry.map_err(map_io)?;
        let entry_path = entry.path();
        if entry.file_type().map_err(map_io)?.is_dir() {
            sync_directory_tree(&entry_path)?;
        } else {
            File::open(entry_path)
                .and_then(|file| file.sync_all())
                .map_err(map_io)?;
        }
    }
    sync_directory(path)
}

fn sync_directory(path: &Path) -> Result<(), IndexStoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(map_io)
}

fn ensure_generation_path(root: &Path, path: &Path) -> Result<(), IndexStoreError> {
    if path == root || !path.starts_with(root) {
        return Err(IndexStoreError::InvalidInput(format!(
            "publisher path escapes state directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn map_io(error: std::io::Error) -> IndexStoreError {
    IndexStoreError::internal(BACKEND, error)
}

fn map_json(error: serde_json::Error) -> IndexStoreError {
    IndexStoreError::internal(BACKEND, error)
}

fn map_raw(error: crate::Error) -> IndexStoreError {
    IndexStoreError::internal(BACKEND, error)
}

#[cfg(test)]
mod tests;
