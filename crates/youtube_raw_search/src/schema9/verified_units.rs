//! Lazy, once-per-unit integrity checks for immutable generation files.
//!
//! The caller must keep the backing bytes immutable for this object's lifetime.
//! Publication and cleanup must preserve that guarantee before this is wired to mmap.

use std::{num::NonZeroUsize, sync::OnceLock};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) const MIN_UNIT_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_UNIT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum UnitError {
    #[error("integrity unit size must be between 2 and 8 MiB")]
    InvalidUnitSize,
    #[error("integrity digest count does not match file length")]
    DigestCount,
    #[error("verified byte range exceeds file length")]
    OutOfBounds,
    #[error("integrity unit {0} differs from its digest")]
    Corrupt(usize),
}

pub(crate) struct VerifiedUnits<'a> {
    bytes: &'a [u8],
    unit_bytes: NonZeroUsize,
    digests: Vec<[u8; 32]>,
    checked: Vec<OnceLock<Result<(), UnitError>>>,
    #[cfg(test)]
    hash_attempts: Vec<AtomicUsize>,
}

impl<'a> VerifiedUnits<'a> {
    pub(crate) fn new(
        bytes: &'a [u8],
        unit_bytes: usize,
        digests: Vec<[u8; 32]>,
    ) -> Result<Self, UnitError> {
        if !(MIN_UNIT_BYTES..=MAX_UNIT_BYTES).contains(&unit_bytes) {
            return Err(UnitError::InvalidUnitSize);
        }
        let unit_bytes = NonZeroUsize::new(unit_bytes).ok_or(UnitError::InvalidUnitSize)?;
        if digests.len() != bytes.len().div_ceil(unit_bytes.get()) {
            return Err(UnitError::DigestCount);
        }
        let checked = (0..digests.len()).map(|_| OnceLock::new()).collect();
        #[cfg(test)]
        let hash_attempts = (0..digests.len()).map(|_| AtomicUsize::new(0)).collect();
        Ok(Self {
            bytes,
            unit_bytes,
            digests,
            checked,
            #[cfg(test)]
            hash_attempts,
        })
    }

    pub(crate) fn read(&self, offset: usize, length: usize) -> Result<&'a [u8], UnitError> {
        let end = offset.checked_add(length).ok_or(UnitError::OutOfBounds)?;
        let range = self.bytes.get(offset..end).ok_or(UnitError::OutOfBounds)?;
        if length == 0 {
            return Ok(range);
        }
        let first = offset / self.unit_bytes;
        let last = (end - 1) / self.unit_bytes;
        for index in first..=last {
            self.checked[index]
                .get_or_init(|| {
                    #[cfg(test)]
                    self.hash_attempts[index].fetch_add(1, Ordering::Relaxed);
                    let start = index * self.unit_bytes.get();
                    let stop = start
                        .saturating_add(self.unit_bytes.get())
                        .min(self.bytes.len());
                    let actual = blake3::hash(&self.bytes[start..stop]);
                    if actual.as_bytes() == &self.digests[index] {
                        Ok(())
                    } else {
                        Err(UnitError::Corrupt(index))
                    }
                })
                .clone()?;
        }
        Ok(range)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{MIN_UNIT_BYTES, UnitError, VerifiedUnits};

    fn digests(bytes: &[u8]) -> Vec<[u8; 32]> {
        bytes
            .chunks(MIN_UNIT_BYTES)
            .map(|unit| *blake3::hash(unit).as_bytes())
            .collect()
    }

    #[test]
    fn validates_only_touched_units_and_short_tail() {
        let bytes = vec![7; MIN_UNIT_BYTES * 2 + 3];
        let verifier = VerifiedUnits::new(&bytes, MIN_UNIT_BYTES, digests(&bytes)).unwrap();
        assert_eq!(verifier.read(4, 8).unwrap(), &[7; 8]);
        assert_eq!(verifier.read(4, 8).unwrap(), &[7; 8]);
        assert_eq!(verifier.hash_attempts[0].load(super::Ordering::Relaxed), 1);
        assert_eq!(verifier.hash_attempts[1].load(super::Ordering::Relaxed), 0);
        assert_eq!(verifier.read(MIN_UNIT_BYTES * 2 - 1, 4).unwrap().len(), 4);
        assert_eq!(verifier.hash_attempts[1].load(super::Ordering::Relaxed), 1);
        assert_eq!(verifier.hash_attempts[2].load(super::Ordering::Relaxed), 1);
    }

    #[test]
    fn caches_corruption_across_concurrent_callers_without_touching_other_units() {
        let mut bytes = vec![3; MIN_UNIT_BYTES * 2];
        let expected = digests(&bytes);
        bytes[MIN_UNIT_BYTES + 1] = 9;
        let verifier = Arc::new(VerifiedUnits::new(&bytes, MIN_UNIT_BYTES, expected).unwrap());
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let verifier = Arc::clone(&verifier);
                scope.spawn(move || {
                    assert_eq!(
                        verifier.read(MIN_UNIT_BYTES + 1, 1),
                        Err(UnitError::Corrupt(1))
                    );
                });
            }
        });
        assert_eq!(verifier.hash_attempts[1].load(super::Ordering::Relaxed), 1);
        assert_eq!(verifier.hash_attempts[0].load(super::Ordering::Relaxed), 0);
        assert_eq!(verifier.read(0, 1).unwrap(), &[3]);
    }

    #[test]
    fn concurrent_success_and_exact_boundary_hash_only_one_unit() {
        let bytes = vec![4; MIN_UNIT_BYTES * 2];
        let verifier =
            Arc::new(VerifiedUnits::new(&bytes, MIN_UNIT_BYTES, digests(&bytes)).unwrap());
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let verifier = Arc::clone(&verifier);
                scope.spawn(move || {
                    assert_eq!(
                        verifier.read(0, MIN_UNIT_BYTES).unwrap().len(),
                        MIN_UNIT_BYTES
                    );
                });
            }
        });
        assert_eq!(verifier.hash_attempts[0].load(super::Ordering::Relaxed), 1);
        assert_eq!(verifier.hash_attempts[1].load(super::Ordering::Relaxed), 0);
    }

    #[test]
    fn spanning_corruption_and_invalid_ranges_do_not_return_unverified_bytes() {
        let mut bytes = vec![5; MIN_UNIT_BYTES * 2];
        let expected = digests(&bytes);
        bytes[MIN_UNIT_BYTES] = 6;
        let verifier = VerifiedUnits::new(&bytes, MIN_UNIT_BYTES, expected).unwrap();
        assert_eq!(
            verifier.read(0, bytes.len() + 1),
            Err(UnitError::OutOfBounds)
        );
        assert_eq!(verifier.hash_attempts[0].load(super::Ordering::Relaxed), 0);
        assert_eq!(verifier.hash_attempts[1].load(super::Ordering::Relaxed), 0);
        assert_eq!(
            verifier.read(MIN_UNIT_BYTES - 1, 2),
            Err(UnitError::Corrupt(1))
        );
        assert_eq!(verifier.hash_attempts[0].load(super::Ordering::Relaxed), 1);
        assert_eq!(verifier.hash_attempts[1].load(super::Ordering::Relaxed), 1);
    }

    #[test]
    fn rejects_bad_metadata_and_ranges_and_accepts_empty_file() {
        assert!(matches!(
            VerifiedUnits::new(&[], 0, vec![]),
            Err(UnitError::InvalidUnitSize)
        ));
        assert!(matches!(
            VerifiedUnits::new(&[1], MIN_UNIT_BYTES, vec![]),
            Err(UnitError::DigestCount)
        ));
        let empty = VerifiedUnits::new(&[], MIN_UNIT_BYTES, vec![]).unwrap();
        assert!(empty.read(0, 0).unwrap().is_empty());
        assert_eq!(empty.read(0, 1), Err(UnitError::OutOfBounds));
        let one = VerifiedUnits::new(&[1], MIN_UNIT_BYTES, digests(&[1])).unwrap();
        assert_eq!(one.read(usize::MAX, 2), Err(UnitError::OutOfBounds));
        assert!(one.read(1, 0).unwrap().is_empty());
    }
}
