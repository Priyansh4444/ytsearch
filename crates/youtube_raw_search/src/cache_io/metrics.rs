use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(super) enum WriterClass {
    General,
    PostingShard,
}

#[derive(Clone, Copy, Default)]
struct WriterMetrics {
    reclaims: u64,
    advised_bytes: u64,
    flush_nanos: u64,
    sync_nanos: u64,
    advise_nanos: u64,
    seeks: u64,
    seek_flush_nanos: u64,
    seek_nanos: u64,
}

#[derive(Clone, Copy, Default)]
struct PublicationIoMetrics {
    general: WriterMetrics,
    posting_shard: WriterMetrics,
    reader_reclaims: u64,
    reader_advised_bytes: u64,
    reader_advise_nanos: u64,
}

thread_local! {
    static ACTIVE_METRICS: RefCell<Option<PublicationIoMetrics>> = const { RefCell::new(None) };
}

pub(super) struct PublicationIoMeasurement {
    wall_started: Instant,
    thread_cpu_started: Option<u64>,
}

impl PublicationIoMeasurement {
    pub(super) fn start() -> Self {
        ACTIVE_METRICS
            .with(|metrics| *metrics.borrow_mut() = Some(PublicationIoMetrics::default()));
        Self {
            wall_started: Instant::now(),
            thread_cpu_started: thread_cpu_nanos(),
        }
    }
}

impl Drop for PublicationIoMeasurement {
    fn drop(&mut self) {
        let metrics = ACTIVE_METRICS.with(|metrics| metrics.borrow_mut().take());
        let Some(metrics) = metrics else {
            return;
        };
        let thread_cpu_nanos = self
            .thread_cpu_started
            .zip(thread_cpu_nanos())
            .map(|(started, finished)| finished.saturating_sub(started));
        tracing::info!(
            wall_ms = duration_millis(self.wall_started.elapsed()),
            thread_cpu_ms = thread_cpu_nanos.map(nanos_millis),
            general_reclaims = metrics.general.reclaims,
            general_advised_bytes = metrics.general.advised_bytes,
            general_flush_ms = nanos_millis(metrics.general.flush_nanos),
            general_sync_ms = nanos_millis(metrics.general.sync_nanos),
            general_advise_ms = nanos_millis(metrics.general.advise_nanos),
            general_seeks = metrics.general.seeks,
            general_seek_flush_ms = nanos_millis(metrics.general.seek_flush_nanos),
            general_seek_ms = nanos_millis(metrics.general.seek_nanos),
            posting_reclaims = metrics.posting_shard.reclaims,
            posting_advised_bytes = metrics.posting_shard.advised_bytes,
            posting_flush_ms = nanos_millis(metrics.posting_shard.flush_nanos),
            posting_sync_ms = nanos_millis(metrics.posting_shard.sync_nanos),
            posting_advise_ms = nanos_millis(metrics.posting_shard.advise_nanos),
            posting_seeks = metrics.posting_shard.seeks,
            posting_seek_flush_ms = nanos_millis(metrics.posting_shard.seek_flush_nanos),
            posting_seek_ms = nanos_millis(metrics.posting_shard.seek_nanos),
            reader_reclaims = metrics.reader_reclaims,
            reader_advised_bytes = metrics.reader_advised_bytes,
            reader_advise_ms = nanos_millis(metrics.reader_advise_nanos),
            "raw publication I/O summary"
        );
    }
}

pub(super) fn record_writer(
    class: WriterClass,
    advised_bytes: u64,
    flush: Duration,
    sync: Duration,
    advise: Duration,
) {
    with_active(|metrics| {
        let writer = metrics.writer(class);
        writer.reclaims = writer.reclaims.saturating_add(1);
        writer.advised_bytes = writer.advised_bytes.saturating_add(advised_bytes);
        writer.flush_nanos = writer.flush_nanos.saturating_add(duration_nanos(flush));
        writer.sync_nanos = writer.sync_nanos.saturating_add(duration_nanos(sync));
        writer.advise_nanos = writer.advise_nanos.saturating_add(duration_nanos(advise));
    });
}

pub(super) fn record_reader(advised_bytes: u64, advise: Duration) {
    with_active(|metrics| {
        metrics.reader_reclaims = metrics.reader_reclaims.saturating_add(1);
        metrics.reader_advised_bytes = metrics.reader_advised_bytes.saturating_add(advised_bytes);
        metrics.reader_advise_nanos = metrics
            .reader_advise_nanos
            .saturating_add(duration_nanos(advise));
    });
}

pub(super) fn record_seek(class: WriterClass, flush: Duration, seek: Duration) {
    with_active(|metrics| {
        let writer = metrics.writer(class);
        writer.seeks = writer.seeks.saturating_add(1);
        writer.seek_flush_nanos = writer
            .seek_flush_nanos
            .saturating_add(duration_nanos(flush));
        writer.seek_nanos = writer.seek_nanos.saturating_add(duration_nanos(seek));
    });
}

/// Updates the current thread's metrics, if a measurement is active.
fn with_active(update: impl FnOnce(&mut PublicationIoMetrics)) {
    ACTIVE_METRICS.with(|metrics| metrics.borrow_mut().as_mut().map(update));
}

impl PublicationIoMetrics {
    fn writer(&mut self, class: WriterClass) -> &mut WriterMetrics {
        match class {
            WriterClass::General => &mut self.general,
            WriterClass::PostingShard => &mut self.posting_shard,
        }
    }
}

fn duration_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn nanos_millis(nanos: u64) -> f64 {
    nanos as f64 / 1_000_000.0
}

#[cfg(target_os = "linux")]
fn thread_cpu_nanos() -> Option<u64> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes to the valid stack-owned timespec and does
    // not retain the pointer.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut value) };
    if result != 0 || value.tv_sec < 0 || value.tv_nsec < 0 {
        return None;
    }
    let seconds = u64::try_from(value.tv_sec).ok()?;
    let nanos = u64::try_from(value.tv_nsec).ok()?;
    seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
}

#[cfg(not(target_os = "linux"))]
fn thread_cpu_nanos() -> Option<u64> {
    None
}
