use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

/// Writes `body` to the new file `temporary`, renames it over `path` and syncs
/// the parent directory, so readers see either the old or the new contents.
pub(super) fn write(path: &Path, temporary: &Path, body: &[u8]) -> std::io::Result<()> {
    let directory = path.parent().expect("atomic write target has a parent");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)?;
    file.write_all(body)?;
    file.sync_all()?;
    if let Err(error) = fs::rename(temporary, path) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    File::open(directory)?.sync_all()
}
