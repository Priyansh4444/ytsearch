//! Finalize completed structural files in an unpublished generation directory.
//! The source writer and authenticated publication manifest are separate gates.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

use super::footer::{
    Directory, DirectoryBinding, DirectoryError, FileKind, FileRecord, OpenedGeneration,
};

pub(crate) struct DirectoryHeader {
    pub(crate) normalization_version: u16,
    pub(crate) rank_version: u16,
    pub(crate) generation: u64,
    pub(crate) mutation_sequence: u64,
    pub(crate) corpus_digest: [u8; 32],
    pub(crate) document_count: u64,
    pub(crate) video_count: u64,
    pub(crate) term_count: u64,
    pub(crate) bitmap_budget_bytes: u64,
}

/// Hash every completed file, write the bounded directory, then reopen and
/// verify every integrity unit before returning values for a trusted manifest.
pub(crate) fn finalize_directory(
    root: &Path,
    header: DirectoryHeader,
    unit_bytes: usize,
) -> Result<DirectoryBinding, DirectoryError> {
    let final_path = root.join("directory.bin");
    match final_path.symlink_metadata() {
        Ok(_) => return Err(DirectoryError::FileLength),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(DirectoryError::Io),
    }
    for entry in fs::read_dir(root).map_err(|_| DirectoryError::Io)? {
        let entry = entry.map_err(|_| DirectoryError::Io)?;
        if !FileKind::ALL
            .iter()
            .any(|kind| entry.file_name() == kind.file_name())
        {
            return Err(DirectoryError::UnexpectedFile);
        }
    }
    let mut files = Vec::new();
    for kind in FileKind::ALL {
        let path = root.join(kind.file_name());
        match path.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_file() => {
                files.push(FileRecord::from_file(kind, &path, unit_bytes)?);
            }
            Ok(_) => return Err(DirectoryError::FileLength),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !kind.required() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(DirectoryError::FileLength);
            }
            Err(_) => return Err(DirectoryError::Io),
        }
    }
    let directory = Directory {
        normalization_version: header.normalization_version,
        rank_version: header.rank_version,
        generation: header.generation,
        mutation_sequence: header.mutation_sequence,
        corpus_digest: header.corpus_digest,
        document_count: header.document_count,
        video_count: header.video_count,
        term_count: header.term_count,
        bitmap_budget_bytes: header.bitmap_budget_bytes,
        files,
    };
    let encoded = directory.encode()?;
    let binding = DirectoryBinding {
        generation: directory.generation,
        mutation_sequence: directory.mutation_sequence,
        corpus_digest: directory.corpus_digest,
        directory_digest: Directory::digest(&encoded)?,
    };
    let temporary = root.join(".directory.bin.pending");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| DirectoryError::Io)?;
    file.write_all(&encoded).map_err(|_| DirectoryError::Io)?;
    file.sync_all().map_err(|_| DirectoryError::Io)?;
    fs::rename(&temporary, &final_path).map_err(|_| DirectoryError::Io)?;
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| DirectoryError::Io)?;

    let opened = OpenedGeneration::open_bound(root, &binding)?;
    for record in &opened.directory.files {
        let length = usize::try_from(record.length).map_err(|_| DirectoryError::Size)?;
        opened.verified_file(record.kind)?.read(0, length)?;
    }
    Ok(binding)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> DirectoryHeader {
        DirectoryHeader {
            normalization_version: 2,
            rank_version: 2,
            generation: 7,
            mutation_sequence: 81,
            corpus_digest: [9; 32],
            document_count: 1,
            video_count: 1,
            term_count: 1,
            bitmap_budget_bytes: 8 * 1024 * 1024,
        }
    }

    #[test]
    fn completed_files_are_hashed_written_and_reopened() {
        let temporary = tempfile::tempdir().unwrap();
        for kind in FileKind::ALL.into_iter().filter(|kind| kind.required()) {
            fs::write(temporary.path().join(kind.file_name()), [kind as u8]).unwrap();
        }
        let binding = finalize_directory(temporary.path(), header(), 2 * 1024 * 1024).unwrap();
        let opened = OpenedGeneration::open_bound(temporary.path(), &binding).unwrap();
        assert_eq!(opened.files.len(), 10);
        assert_eq!(opened.directory.document_count, 1);
        assert_eq!(
            opened.verified_file(FileKind::Terms).unwrap().read(0, 1),
            Ok(&[1][..])
        );
        assert_eq!(
            finalize_directory(temporary.path(), header(), 2 * 1024 * 1024),
            Err(DirectoryError::FileLength)
        );
    }

    #[test]
    fn missing_required_file_never_writes_a_directory() {
        let temporary = tempfile::tempdir().unwrap();
        for kind in FileKind::ALL
            .into_iter()
            .filter(|kind| kind.required() && *kind != FileKind::Terms)
        {
            fs::write(temporary.path().join(kind.file_name()), [kind as u8]).unwrap();
        }
        assert_eq!(
            finalize_directory(temporary.path(), header(), 2 * 1024 * 1024),
            Err(DirectoryError::FileLength)
        );
        assert!(!temporary.path().join("directory.bin").exists());
    }

    #[test]
    fn stale_query_plan_artifact_is_rejected() {
        let temporary = tempfile::tempdir().unwrap();
        for kind in FileKind::ALL.into_iter().filter(|kind| kind.required()) {
            fs::write(temporary.path().join(kind.file_name()), [kind as u8]).unwrap();
        }
        fs::write(temporary.path().join("broad-single.bin"), [0]).unwrap();
        assert_eq!(
            finalize_directory(temporary.path(), header(), 2 * 1024 * 1024),
            Err(DirectoryError::UnexpectedFile)
        );
        assert!(!temporary.path().join("directory.bin").exists());
    }
}
