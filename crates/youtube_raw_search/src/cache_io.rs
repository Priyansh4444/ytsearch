use std::{
    fs::File,
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

const DEFAULT_RECLAIM_BYTES: u64 = 8 * 1024 * 1024;
const SEQUENTIAL_RECLAIM_BYTES: u64 = 8 * 1024 * 1024;

/// Writes build-only files through a bounded dirty-page window, then asks the
/// kernel to reclaim those pages. Immutable generations are opened with mmap,
/// so retaining disposable spool/output pages would evict the active reader.
pub(crate) struct ColdWriter {
    output: BufWriter<File>,
    bytes_since_reclaim: u64,
    reclaim_bytes: u64,
}

impl ColdWriter {
    pub(crate) fn create(path: &Path) -> io::Result<Self> {
        Self::create_with_reclaim_bytes(path, DEFAULT_RECLAIM_BYTES)
    }

    pub(crate) fn create_with_reclaim_bytes(path: &Path, reclaim_bytes: u64) -> io::Result<Self> {
        if reclaim_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache reclaim interval must be positive",
            ));
        }
        Ok(Self {
            output: BufWriter::new(File::create(path)?),
            bytes_since_reclaim: 0,
            reclaim_bytes,
        })
    }

    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.reclaim()
    }

    fn reclaim(&mut self) -> io::Result<()> {
        self.output.flush()?;
        self.output.get_ref().sync_data()?;
        advise_dont_need(self.output.get_ref(), 0, 0);
        self.bytes_since_reclaim = 0;
        Ok(())
    }
}

impl Write for ColdWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.output.write(buffer)?;
        self.bytes_since_reclaim = self.bytes_since_reclaim.saturating_add(written as u64);
        if self.bytes_since_reclaim >= self.reclaim_bytes {
            self.reclaim()?;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

impl Seek for ColdWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.reclaim()?;
        self.output.seek(position)
    }
}

/// Marks sequential source bytes cold as soon as the decoder has consumed
/// them. This keeps the active mmap generation resident during publication.
pub(crate) struct ColdReader {
    input: File,
    position: u64,
    advised_through: u64,
}

/// Lowers the current publication thread's Linux block-I/O priority and
/// restores it before Tokio can reuse that blocking-pool thread. Search page
/// faults then take precedence over generation scans and writeback.
pub(crate) struct BackgroundIoGuard {
    #[cfg(target_os = "linux")]
    previous: Option<i32>,
}

impl BackgroundIoGuard {
    pub(crate) fn enter() -> Self {
        #[cfg(target_os = "linux")]
        {
            let previous = linux_ioprio_get();
            if previous.is_some() {
                linux_ioprio_set(3 << 13);
            }
            Self { previous }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self {}
        }
    }
}

impl Drop for BackgroundIoGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(previous) = self.previous {
            linux_ioprio_set(previous);
        }
    }
}

impl ColdReader {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let input = File::open(path)?;
        advise_sequential(&input);
        Ok(Self {
            input,
            position: 0,
            advised_through: 0,
        })
    }

    fn reclaim_consumed(&mut self) {
        let length = self.position.saturating_sub(self.advised_through);
        if length >= SEQUENTIAL_RECLAIM_BYTES {
            advise_dont_need(&self.input, self.advised_through, length);
            self.advised_through = self.position;
        }
    }
}

impl Read for ColdReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.input.read(buffer)?;
        self.position = self.position.saturating_add(read as u64);
        self.reclaim_consumed();
        Ok(read)
    }
}

impl Drop for ColdReader {
    fn drop(&mut self) {
        let length = self.position.saturating_sub(self.advised_through);
        advise_dont_need(&self.input, self.advised_through, length);
    }
}

#[cfg(target_os = "linux")]
fn advise_sequential(file: &File) {
    use std::os::fd::AsRawFd;
    // SAFETY: posix_fadvise only consumes the valid descriptor and numeric
    // range; it does not retain pointers or alter Rust-owned memory.
    unsafe {
        libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_SEQUENTIAL);
    }
}

#[cfg(not(target_os = "linux"))]
fn advise_sequential(_file: &File) {}

#[cfg(target_os = "linux")]
fn advise_dont_need(file: &File, offset: u64, length: u64) {
    use std::os::fd::AsRawFd;
    let Ok(offset) = i64::try_from(offset) else {
        return;
    };
    let Ok(length) = i64::try_from(length) else {
        return;
    };
    // SAFETY: posix_fadvise only consumes the valid descriptor and numeric
    // range; it does not retain pointers or invalidate userspace buffers.
    unsafe {
        libc::posix_fadvise(file.as_raw_fd(), offset, length, libc::POSIX_FADV_DONTNEED);
    }
}

#[cfg(not(target_os = "linux"))]
fn advise_dont_need(_file: &File, _offset: u64, _length: u64) {}

#[cfg(target_os = "linux")]
fn linux_ioprio_get() -> Option<i32> {
    // SAFETY: syscall receives only integer values and returns the calling
    // thread's encoded I/O priority without retaining any memory.
    let result = unsafe { libc::syscall(libc::SYS_ioprio_get, 1_i32, 0_i32) };
    i32::try_from(result).ok().filter(|result| *result >= 0)
}

#[cfg(target_os = "linux")]
fn linux_ioprio_set(priority: i32) {
    // SAFETY: syscall receives only integer values. who=0 scopes the change to
    // the calling thread, which the guard restores before pool reuse.
    unsafe {
        libc::syscall(libc::SYS_ioprio_set, 1_i32, 0_i32, priority);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_writer_preserves_write_and_seek_contract() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cold.bin");
        let mut output = ColdWriter::create_with_reclaim_bytes(&path, 2).unwrap();
        output.write_all(b"abcd").unwrap();
        output.seek(SeekFrom::Start(1)).unwrap();
        output.write_all(b"Z").unwrap();
        output.finish().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"aZcd");
    }

    #[test]
    fn cold_reader_preserves_sequential_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input.bin");
        std::fs::write(&path, b"streamed corpus").unwrap();
        let mut input = ColdReader::open(&path).unwrap();
        let mut value = String::new();
        input.read_to_string(&mut value).unwrap();
        assert_eq!(value, "streamed corpus");
    }
}
