use crate::{
    BuildReport, MutationSnapshot, MutationStore, RawSearchEngine, cache_io::ColdReader,
    engine::build_index_from_lines,
};
use flate2::read::MultiGzDecoder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};
use youtube_index_store::{ChannelDocument, IndexStoreError};

const BACKEND: &str = "raw";
const ACTIVE_MANIFEST: &str = "active-generation.json";

#[derive(Clone, Debug)]
pub struct PublisherPaths {
    pub base_chunks_gzip: PathBuf,
    pub base_channels_gzip: PathBuf,
    pub base_index: PathBuf,
    pub state_directory: PathBuf,
}

pub struct PublishedGeneration {
    pub sequence: u64,
    pub path: PathBuf,
    pub engine: RawSearchEngine,
    pub report: BuildReport,
}

pub struct Publisher {
    paths: PublisherPaths,
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

impl Publisher {
    pub fn new(paths: PublisherPaths) -> Self {
        Self { paths }
    }

    pub fn active_index_path(&self) -> Result<PathBuf, IndexStoreError> {
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

    pub fn active_sequence(&self) -> Result<u64, IndexStoreError> {
        let manifest_path = self.paths.state_directory.join(ACTIVE_MANIFEST);
        if !manifest_path.exists() {
            return Ok(0);
        }
        serde_json::from_reader::<_, ActiveGeneration>(File::open(manifest_path).map_err(map_io)?)
            .map(|manifest| manifest.sequence)
            .map_err(map_json)
    }

    pub fn publish(
        &self,
        store: &MutationStore,
    ) -> Result<Option<PublishedGeneration>, IndexStoreError> {
        fs::create_dir_all(self.paths.state_directory.join("generations")).map_err(map_io)?;
        fs::create_dir_all(self.paths.state_directory.join("work")).map_err(map_io)?;
        self.remove_uncommitted_artifacts()?;
        let snapshot = store.snapshot()?;
        if snapshot.through_sequence <= self.active_sequence()? {
            return Ok(None);
        }
        validate_source(&self.paths.base_chunks_gzip)?;
        validate_source(&self.paths.base_channels_gzip)?;
        validate_source(&self.paths.base_index)?;
        let generation_name = format!("generations/{:020}", snapshot.through_sequence);
        let final_path = self.paths.state_directory.join(&generation_name);
        ensure_generation_path(&self.paths.state_directory, &final_path)?;
        if final_path.exists() {
            let engine = RawSearchEngine::open(&final_path).map_err(map_raw)?;
            self.activate(snapshot.through_sequence, &generation_name)?;
            store.mark_published(snapshot.through_sequence)?;
            let report = report_for_existing(&engine);
            return Ok(Some(PublishedGeneration {
                sequence: snapshot.through_sequence,
                path: final_path,
                engine,
                report,
            }));
        }

        let candidate_path = final_path.with_extension("candidate");
        reset_private_directory(&self.paths.state_directory, &candidate_path)?;

        let report = self.build_generation(&snapshot, &candidate_path)?;
        let engine = RawSearchEngine::open(&candidate_path).map_err(map_raw)?;
        sync_directory_tree(&candidate_path)?;
        fs::rename(&candidate_path, &final_path).map_err(map_io)?;
        sync_directory(&self.paths.state_directory.join("generations"))?;
        self.activate(snapshot.through_sequence, &generation_name)?;
        store.mark_published(snapshot.through_sequence)?;
        self.remove_old_generations(snapshot.through_sequence)?;

        Ok(Some(PublishedGeneration {
            sequence: snapshot.through_sequence,
            path: final_path,
            engine,
            report,
        }))
    }

    fn build_generation(
        &self,
        snapshot: &MutationSnapshot,
        candidate_path: &Path,
    ) -> Result<BuildReport, IndexStoreError> {
        let video_mutations = snapshot
            .videos
            .iter()
            .map(|mutation| (mutation.video_id.as_str(), mutation))
            .collect::<HashMap<_, _>>();
        let cascaded_channels = snapshot
            .channels
            .iter()
            .filter(|mutation| mutation.document.is_none() && mutation.cascade)
            .map(|mutation| mutation.channel_id.as_str())
            .collect::<HashSet<_>>();

        let base_documents = gzip_lines(&self.paths.base_chunks_gzip)?.filter_map(|line| {
            let line = match line {
                Ok(line) => line,
                Err(error) => return Some(Err(crate::Error::Io(error))),
            };
            let key = match serde_json::from_str::<ChunkKey>(&line) {
                Ok(key) => key,
                Err(error) => return Some(Err(crate::Error::Json(error))),
            };
            if video_mutations.contains_key(key.video_id.as_str())
                || cascaded_channels.contains(key.youtuber_id.as_str())
            {
                None
            } else {
                Some(Ok(line))
            }
        });
        let replacement_documents = snapshot
            .videos
            .iter()
            .flat_map(|mutation| {
                mutation
                    .chunks
                    .as_deref()
                    .filter(|chunks| {
                        !chunks
                            .first()
                            .is_some_and(|chunk| cascaded_channels.contains(chunk.youtuber_id()))
                    })
                    .unwrap_or_default()
                    .iter()
            })
            .map(|chunk| serde_json::to_string(chunk).map_err(crate::Error::Json));

        let mut channels = BTreeMap::<String, ChannelDocument>::new();
        for line in gzip_lines(&self.paths.base_channels_gzip)? {
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
        build_index_from_lines(
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

    fn remove_uncommitted_artifacts(&self) -> Result<(), IndexStoreError> {
        let work = self.paths.state_directory.join("work");
        for entry in fs::read_dir(&work).map_err(map_io)? {
            remove_scratch_entry(&self.paths.state_directory, entry.map_err(map_io)?)?;
        }

        let generations = self.paths.state_directory.join("generations");
        for entry in fs::read_dir(&generations).map_err(map_io)? {
            let entry = entry.map_err(map_io)?;
            if entry
                .path()
                .extension()
                .is_some_and(|value| value == "candidate")
            {
                remove_scratch_entry(&self.paths.state_directory, entry)?;
            }
        }
        Ok(())
    }
}

fn remove_scratch_entry(root: &Path, entry: fs::DirEntry) -> Result<(), IndexStoreError> {
    let path = entry.path();
    ensure_generation_path(root, &path)?;
    let file_type = entry.file_type().map_err(map_io)?;
    if file_type.is_dir() {
        fs::remove_dir_all(path).map_err(map_io)
    } else if file_type.is_file() {
        fs::remove_file(path).map_err(map_io)
    } else {
        Err(IndexStoreError::InvalidInput(format!(
            "publisher scratch path has unsupported type: {}",
            path.display()
        )))
    }
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

fn reset_private_directory(root: &Path, path: &Path) -> Result<(), IndexStoreError> {
    ensure_generation_path(root, path)?;
    if path.exists() {
        fs::remove_dir_all(path).map_err(map_io)?;
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

fn report_for_existing(engine: &RawSearchEngine) -> BuildReport {
    BuildReport {
        documents: engine.document_count(),
        videos: engine.video_count(),
        channels: engine.channel_count(),
        terms: engine.term_count(),
        index_bytes: engine.index_size(),
        elapsed_ms: 0,
    }
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
