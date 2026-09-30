//! Per-window score bounds for broad last-token plans (`plan-bounds.bin`).
//!
//! The last query token also matches its prefix extensions ("the" matches
//! "there", "they", ...). Summing each alternative's own window maximum
//! overstates the plan's score several times, because the alternatives'
//! maxima rarely fall on one document, and then no window can be pruned.
//! For every lexicon term whose last-token plan is broad and has several
//! alternatives, this file stores the plan's true maximum score in each
//! window, computed with the scan itself, rounded up to one byte.
//!
//! The entry records the alternatives it was computed for. A query plan may
//! use it only when each of its alternatives (term and multiplier) is among
//! them: every contribution is non-negative, so dropping alternatives, a
//! field, or the position boost only lowers the plan's score.

use super::conjunction::TermPlan;
use super::*;

pub(super) const PLAN_BOUNDS_FILE: &str = "plan-bounds.bin";
const PLAN_BOUNDS_MAGIC: &[u8; 8] = b"RAWPLN01";
const PARAMETER_BYTES: usize = 8;
const DIRECTORY_BYTES: usize = 24;
const ALTERNATIVE_BYTES: usize = 8;
const MAX_ALTERNATIVES: u32 = 16;

#[derive(Clone, Copy, Debug)]
pub(super) struct PlanBoundConfig {
    pub(super) window_documents: u32,
    /// Plans estimated below this are not broad and never take the path.
    pub(super) min_documents: u64,
}

/// The fields and boosts the stored bounds are computed with; every query
/// configuration scores at most this.
const BOUND_FIELDS: QueryFields = QueryFields {
    text: true,
    title: true,
    prioritize_position: true,
    exact_mask: 0,
};

pub(super) fn write_plan_bounds(
    engine: &RawSearchEngine,
    index_path: &Path,
    config: PlanBoundConfig,
) -> Result<()> {
    let documents = engine.inner.manifest.documents;
    let window = config.window_documents.max(1);
    let windows = documents.div_ceil(window) as usize;
    let mut candidates = Vec::new();
    for index in 0..engine.inner.lexicon.count {
        let (term, _) = engine.inner.lexicon.entry_at(index)?;
        let (plans, _) = engine.conjunction_plans(&[term.to_string()], BOUND_FIELDS, true, 0)?;
        let [plan] = plans.as_slice() else {
            continue;
        };
        if plan.alternatives.len() >= 2 && plan.estimated_documents >= config.min_documents {
            candidates.push(index as u32);
        }
    }

    let workers = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .clamp(1, 8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let computed = std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|_| {
                scope.spawn(|| -> Result<Vec<(usize, Vec<u8>)>> {
                    let mut output = Vec::new();
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&term) = candidates.get(index) else {
                            return Ok(output);
                        };
                        let (name, _) = engine.inner.lexicon.entry_at(term as usize)?;
                        let (plans, _) =
                            engine.conjunction_plans(&[name.to_string()], BOUND_FIELDS, true, 0)?;
                        let mut maxima = vec![0.0_f32; windows];
                        engine.visit_term_plan_range(&plans[0], 0..documents, |candidate, _| {
                            let slot = &mut maxima[(candidate.document() / window) as usize];
                            *slot = slot.max(candidate.score);
                            Ok(())
                        })?;
                        let mut entry = Vec::new();
                        let alternatives = plans[0]
                            .alternatives
                            .iter()
                            .map(|alternative| {
                                (alternative.entry.term, alternative.score_multiplier)
                            })
                            .collect::<Vec<_>>();
                        encode_entry(&mut entry, &alternatives, &maxima);
                        output.push((index, entry));
                    }
                })
            })
            .collect::<Vec<_>>();
        let mut computed = Vec::new();
        for handle in handles {
            computed.extend(
                handle
                    .join()
                    .map_err(|_| Error::Corrupt("plan bound worker panicked".to_string()))??,
            );
        }
        Ok::<_, Error>(computed)
    })?;
    let mut computed = computed;
    computed.sort_unstable_by_key(|(index, _)| *index);

    let payload_start = HEADER_BYTES + PARAMETER_BYTES + candidates.len() * DIRECTORY_BYTES;
    let mut directory = Vec::with_capacity(candidates.len() * DIRECTORY_BYTES);
    let mut payload = Vec::new();
    for ((index, entry), term) in computed.into_iter().zip(&candidates) {
        debug_assert_eq!(candidates[index], *term);
        let offset = u64::try_from(payload_start + payload.len())
            .map_err(|_| Error::Corrupt("plan bound offset overflows".to_string()))?;
        directory.extend_from_slice(&term.to_le_bytes());
        directory.extend_from_slice(&entry[..8]); // alternatives and scale
        directory.extend_from_slice(&0_u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        payload.extend_from_slice(&entry[8..]);
    }
    let mut output = Vec::with_capacity(payload_start + payload.len());
    write_header(
        &mut output,
        PLAN_BOUNDS_MAGIC,
        u32::try_from(candidates.len())
            .map_err(|_| Error::Corrupt("too many plan bounds".to_string()))?,
    );
    output.extend_from_slice(&window.to_le_bytes());
    output.extend_from_slice(&(windows as u32).to_le_bytes());
    output.extend_from_slice(&directory);
    output.extend_from_slice(&payload);
    let mut file = ColdWriter::create(&index_path.join(PLAN_BOUNDS_FILE))?;
    file.write_all(&output)?;
    Ok(file.finish()?)
}

/// `alternatives` (u32) and `scale` (f32), then the payload: alternatives
/// as (term, multiplier bits) and one quantized maximum per window.
fn encode_entry(output: &mut Vec<u8>, alternatives: &[(u32, f32)], maxima: &[f32]) {
    let maximum = maxima.iter().copied().fold(0.0_f32, f32::max);
    // Readers decode `steps * scale` in f64, so check the top step there.
    let mut scale = maximum / 255.0;
    while f64::from(scale) * 255.0 < f64::from(maximum) {
        scale = scale.next_up();
    }
    output.extend_from_slice(&(alternatives.len() as u32).to_le_bytes());
    output.extend_from_slice(&scale.to_le_bytes());
    for (term, multiplier) in alternatives {
        output.extend_from_slice(&term.to_le_bytes());
        output.extend_from_slice(&multiplier.to_le_bytes());
    }
    for &value in maxima {
        output.push(quantize_up(value, scale));
    }
}

/// The smallest step count whose value (`steps * scale` in f64) is at least
/// `value`.
fn quantize_up(value: f32, scale: f32) -> u8 {
    if value <= 0.0 || scale <= 0.0 {
        return 0;
    }
    let mut steps = (f64::from(value) / f64::from(scale)).ceil().min(255.0) as u8;
    while f64::from(steps) * f64::from(scale) < f64::from(value) && steps < u8::MAX {
        steps += 1;
    }
    steps
}

/// Validated view of `plan-bounds.bin`.
pub(super) struct PlanBounds {
    file: MappedFile,
    window_documents: u32,
    windows: usize,
    /// Sorted by term: (term, alternatives, scale, payload offset).
    directory: Vec<(u32, u32, f32, usize)>,
}

pub(super) struct PlanBound<'a> {
    alternatives: &'a [u8],
    scale: f32,
    maxima: &'a [u8],
}

impl PlanBound<'_> {
    /// Whether the bound covers `plan`: each alternative, with its
    /// multiplier, is one the bound was computed for.
    pub(super) fn covers(&self, plan: &TermPlan) -> bool {
        plan.alternatives.iter().all(|alternative| {
            self.alternatives
                .as_chunks::<ALTERNATIVE_BYTES>()
                .0
                .iter()
                .any(|stored| {
                    let (term, multiplier) = stored.split_at(4);
                    u32::from_le_bytes(term.try_into().expect("four bytes"))
                        == alternative.entry.term
                        && u32::from_le_bytes(multiplier.try_into().expect("four bytes"))
                            == alternative.score_multiplier.to_bits()
                })
        })
    }

    /// Upper bounds on the plan's score, one per window.
    pub(super) fn maxima(&self) -> impl Iterator<Item = f64> + '_ {
        let scale = f64::from(self.scale);
        self.maxima
            .iter()
            .map(move |&steps| f64::from(steps) * scale)
    }
}

impl PlanBounds {
    pub(super) fn open(path: &Path, terms: usize, documents: u32) -> Result<Self> {
        let file = MappedFile::open(path, PLAN_BOUNDS_MAGIC)?;
        let bytes = file.bytes();
        let count = read_u32(bytes, 8)? as usize;
        let window_documents = read_u32(bytes, HEADER_BYTES)?;
        let windows = read_u32(bytes, HEADER_BYTES + 4)? as usize;
        if window_documents == 0 || windows != documents.div_ceil(window_documents) as usize {
            return Err(Error::Corrupt(
                "plan bound windows do not match the generation".to_string(),
            ));
        }
        let mut directory: Vec<(u32, u32, f32, usize)> = Vec::with_capacity(count);
        let mut end = HEADER_BYTES + PARAMETER_BYTES + count * DIRECTORY_BYTES;
        for index in 0..count {
            let at = HEADER_BYTES + PARAMETER_BYTES + index * DIRECTORY_BYTES;
            let term = read_u32(bytes, at)?;
            let alternatives = read_u32(bytes, at + 4)?;
            let scale = f32::from_bits(read_u32(bytes, at + 8)?);
            let offset = usize::try_from(read_u64(bytes, at + 16)?)
                .map_err(|_| Error::Corrupt("plan bound offset overflows".to_string()))?;
            if term as usize >= terms
                || directory
                    .last()
                    .is_some_and(|(previous, ..)| *previous >= term)
                || !(2..=MAX_ALTERNATIVES).contains(&alternatives)
                || !scale.is_finite()
                || scale < 0.0
                || offset != end
            {
                return Err(Error::Corrupt(
                    "plan bound directory is invalid".to_string(),
                ));
            }
            end = offset + alternatives as usize * ALTERNATIVE_BYTES + windows;
            directory.push((term, alternatives, scale, offset));
        }
        if end != bytes.len() {
            return Err(Error::Corrupt(
                "plan bound file has an invalid length".to_string(),
            ));
        }
        Ok(Self {
            file,
            window_documents,
            windows,
            directory,
        })
    }

    pub(super) fn window_documents(&self) -> u32 {
        self.window_documents
    }

    pub(super) fn get(&self, term: u32) -> Option<PlanBound<'_>> {
        let index = self
            .directory
            .binary_search_by_key(&term, |(term, ..)| *term)
            .ok()?;
        let (_, alternatives, scale, offset) = self.directory[index];
        let bytes = self.file.bytes();
        let maxima_start = offset + alternatives as usize * ALTERNATIVE_BYTES;
        Some(PlanBound {
            alternatives: &bytes[offset..maxima_start],
            scale,
            maxima: &bytes[maxima_start..maxima_start + self.windows],
        })
    }
}
