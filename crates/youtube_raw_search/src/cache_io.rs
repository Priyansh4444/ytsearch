use std::{
    fs::File,
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tokio::sync::oneshot;

mod metrics;

use metrics::{PublicationIoMeasurement, WriterClass, record_reader, record_seek, record_writer};

const DEFAULT_RECLAIM_BYTES: u64 = 8 * 1024 * 1024;
static RETAIN_PAGE_CACHE: AtomicBool = AtomicBool::new(false);

/// For processes that never serve a generation (the CLI's publication build,
/// offline `build`): keep build pages cached and skip the per-window syncs.
/// Spools are re-read soon after they are written, and on a volume with a
/// bandwidth cap every dropped page is read back from disk. The reader keeps
/// the default, so uploaded generation files cannot evict its mapped index.
pub fn retain_build_page_cache() {
    RETAIN_PAGE_CACHE.store(true, Ordering::Relaxed);
}

fn retaining_page_cache() -> bool {
    RETAIN_PAGE_CACHE.load(Ordering::Relaxed)
}
const SEQUENTIAL_RECLAIM_BYTES: u64 = 8 * 1024 * 1024;
#[cfg(target_os = "linux")]
const BACKGROUND_CPU_NICE: libc::c_int = 10;

/// Runs full-corpus work on a disposable OS thread. On Linux the thread gets a
/// lower CPU priority before `work` starts, so publication cannot consume an
/// equal scheduler share with latency-sensitive query threads. A dedicated
/// thread is required: an unprivileged process can increase niceness but
/// generally cannot restore it before Tokio reuses a blocking-pool thread.
pub(crate) async fn run_background_work<T, F>(name: &str, work: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            lower_current_thread_cpu_priority();
            let _ = sender.send(work());
        })?;
    receiver.await.map_err(|_| {
        io::Error::other(format!(
            "background thread {name:?} terminated without a result"
        ))
    })
}

#[cfg(target_os = "linux")]
fn lower_current_thread_cpu_priority() {
    // SAFETY: setpriority only consumes integer arguments. On Linux, who=0
    // targets the calling task/thread, which is disposable and never reused by
    // the query executor.
    let result = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, BACKGROUND_CPU_NICE) };
    if result != 0 {
        tracing::warn!(
            error = %io::Error::last_os_error(),
            nice = BACKGROUND_CPU_NICE,
            "failed to lower background CPU priority"
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn lower_current_thread_cpu_priority() {}

/// Writes build-only files through a bounded dirty-page window, then asks the
/// kernel to reclaim those pages. Immutable generations are opened with mmap,
/// so retaining disposable spool/output pages would evict the active reader.
pub(crate) struct ColdWriter {
    output: BufWriter<File>,
    bytes_since_reclaim: u64,
    reclaim_bytes: u64,
    position: u64,
    high_watermark: u64,
    advised_through: u64,
    class: WriterClass,
}

impl ColdWriter {
    pub(crate) fn create(path: &Path) -> io::Result<Self> {
        Self::create_classified(path, DEFAULT_RECLAIM_BYTES, WriterClass::General)
    }

    #[cfg(test)]
    fn create_with_reclaim_bytes(path: &Path, reclaim_bytes: u64) -> io::Result<Self> {
        Self::create_classified(path, reclaim_bytes, WriterClass::General)
    }

    pub(crate) fn create_posting_shard(path: &Path) -> io::Result<Self> {
        Self::create_classified(path, 1024 * 1024, WriterClass::PostingShard)
    }

    fn create_classified(path: &Path, reclaim_bytes: u64, class: WriterClass) -> io::Result<Self> {
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
            position: 0,
            high_watermark: 0,
            advised_through: 0,
            class,
        })
    }

    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.reclaim()
    }

    pub(crate) fn position(&self) -> u64 {
        self.position
    }

    fn reclaim(&mut self) -> io::Result<()> {
        let flush_started = Instant::now();
        self.output.flush()?;
        let flush_elapsed = flush_started.elapsed();
        if retaining_page_cache() {
            self.bytes_since_reclaim = 0;
            return Ok(());
        }
        let sync_started = Instant::now();
        self.output.get_ref().sync_data()?;
        let sync_elapsed = sync_started.elapsed();
        let advise_bytes = self.high_watermark.saturating_sub(self.advised_through);
        let advise_started = Instant::now();
        if advise_bytes > 0 {
            advise_dont_need(self.output.get_ref(), self.advised_through, advise_bytes);
            self.advised_through = self.high_watermark;
        }
        let advise_elapsed = advise_started.elapsed();
        record_writer(
            self.class,
            advise_bytes,
            flush_elapsed,
            sync_elapsed,
            advise_elapsed,
        );
        self.bytes_since_reclaim = 0;
        Ok(())
    }
}

impl Write for ColdWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.output.write(buffer)?;
        self.position = self.position.saturating_add(written as u64);
        self.high_watermark = self.high_watermark.max(self.position);
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
        // BufWriter must be flushed before changing the file cursor, but a
        // seek is not a durability boundary. Seek::stream_position reaches
        // this path once per document block, so syncing here created roughly
        // 127k barriers for the full corpus. Publication still syncs completed
        // files and the candidate tree before rename.
        let flush_started = Instant::now();
        self.output.flush()?;
        let flush_elapsed = flush_started.elapsed();
        let seek_started = Instant::now();
        let position = self.output.seek(position)?;
        record_seek(self.class, flush_elapsed, seek_started.elapsed());
        self.position = position;
        Ok(position)
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
    _measurement: PublicationIoMeasurement,
    #[cfg(target_os = "linux")]
    previous: Option<i32>,
}

impl BackgroundIoGuard {
    pub(crate) fn enter() -> Self {
        #[cfg(target_os = "linux")]
        {
            let measurement = PublicationIoMeasurement::start();
            let previous = linux_ioprio_get();
            if previous.is_some() {
                linux_ioprio_set(3 << 13);
            }
            Self {
                _measurement: measurement,
                previous,
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self {
                _measurement: PublicationIoMeasurement::start(),
            }
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
            let started = Instant::now();
            advise_dont_need(&self.input, self.advised_through, length);
            record_reader(length, started.elapsed());
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
        if length > 0 {
            let started = Instant::now();
            advise_dont_need(&self.input, self.advised_through, length);
            record_reader(length, started.elapsed());
        }
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
    if retaining_page_cache() {
        return;
    }
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

    #[tokio::test]
    async fn background_work_uses_a_disposable_named_thread() {
        let caller = std::thread::current().id();
        let (worker, name) = run_background_work("raw-publisher-test", move || {
            (
                std::thread::current().id(),
                std::thread::current().name().map(str::to_owned),
            )
        })
        .await
        .unwrap();

        assert_ne!(worker, caller);
        assert_eq!(name.as_deref(), Some("raw-publisher-test"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn background_work_has_lower_linux_cpu_priority() {
        let priority = run_background_work("raw-publisher-priority-test", || {
            // SAFETY: getpriority only reads the calling thread's scheduler
            // niceness and does not retain any memory.
            unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) }
        })
        .await
        .unwrap();

        assert!(priority >= BACKGROUND_CPU_NICE);
    }

    #[test]
    fn cold_writer_preserves_write_and_seek_contract() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cold.bin");
        let mut output = ColdWriter::create_with_reclaim_bytes(&path, 1024).unwrap();
        output.write_all(b"abcd").unwrap();
        assert_eq!(output.bytes_since_reclaim, 4);
        assert_eq!(output.position(), 4);
        output.seek(SeekFrom::Start(1)).unwrap();
        assert_eq!(output.bytes_since_reclaim, 4);
        assert_eq!(output.position(), 1);
        output.write_all(b"Z").unwrap();
        assert_eq!(output.position(), 2);
        output.finish().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"aZcd");
    }

    #[test]
    fn cold_writer_advises_each_sequential_range_only_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ranges.bin");
        let mut output = ColdWriter::create_with_reclaim_bytes(&path, 4).unwrap();
        output.write_all(b"abcd").unwrap();
        assert_eq!(output.advised_through, 4);
        output.write_all(b"efgh").unwrap();
        assert_eq!(output.advised_through, 8);

        output.seek(SeekFrom::Start(1)).unwrap();
        output.write_all(b"Z").unwrap();
        output.reclaim().unwrap();
        assert_eq!(output.high_watermark, 8);
        assert_eq!(output.advised_through, 8);
        assert_eq!(std::fs::read(path).unwrap(), b"aZcdefgh");
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
