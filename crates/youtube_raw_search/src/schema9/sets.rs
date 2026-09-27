//! Generation-bound exact document and video sets.

use std::{
    marker::PhantomData,
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

const MAX_UNIVERSE_LEN: u64 = u32::MAX as u64 + 1;
const MAX_EXPRESSION_DEPTH: u32 = 64;
const MAX_EXPRESSION_NODES: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GenerationId(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SegmentId(pub(crate) u32);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct DocId(u32);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct VideoId(u32);

mod private {
    pub trait Sealed {}
}

pub(crate) trait SetId: private::Sealed + Copy {
    fn from_raw(raw: u32) -> Self;
    fn raw(self) -> u32;
}

impl private::Sealed for DocId {}
impl private::Sealed for VideoId {}

impl SetId for DocId {
    fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    fn raw(self) -> u32 {
        self.0
    }
}

impl SetId for VideoId {
    fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Universe<Id: SetId> {
    generation: GenerationId,
    segment: SegmentId,
    len: u64,
    marker: PhantomData<Id>,
}

impl<Id: SetId> Universe<Id> {
    pub(crate) fn new(
        generation: GenerationId,
        segment: SegmentId,
        len: u64,
    ) -> Result<Self, SetError> {
        if len > MAX_UNIVERSE_LEN {
            return Err(SetError::InvalidUniverseLength(len));
        }
        Ok(Self {
            generation,
            segment,
            len,
            marker: PhantomData,
        })
    }

    pub(crate) fn len(self) -> u64 {
        self.len
    }

    fn identity(self) -> UniverseIdentity {
        UniverseIdentity {
            generation: self.generation,
            segment: self.segment,
            len: self.len,
        }
    }

    fn check(self, other: Self) -> Result<(), SetError> {
        if self.identity() != other.identity() {
            return Err(SetError::UniverseMismatch {
                left: self.identity(),
                right: other.identity(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UniverseIdentity {
    generation: GenerationId,
    segment: SegmentId,
    len: u64,
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum SetError {
    #[error("schema-9 universe length {0} exceeds 2^32")]
    InvalidUniverseLength(u64),
    #[error("schema-9 set universes differ: {left:?} vs {right:?}")]
    UniverseMismatch {
        left: UniverseIdentity,
        right: UniverseIdentity,
    },
    #[error("schema-9 set contains duplicate ID {0}")]
    Duplicate(u32),
    #[error("schema-9 set IDs decrease at offset {0}")]
    Unsorted(usize),
    #[error("schema-9 set ID {id} is outside universe of length {len}")]
    OutOfRange { id: u32, len: u64 },
    #[error("schema-9 dense set has {actual} words; expected {expected}")]
    WrongWordCount { expected: u64, actual: u64 },
    #[error("schema-9 dense set has nonzero tail bits")]
    DirtyTailBits,
    #[error("schema-9 set byte count overflows")]
    ByteOverflow,
    #[error("schema-9 expression exceeds maximum depth of 64")]
    ExpressionTooDeep,
    #[error("schema-9 expression exceeds maximum size of 4096 nodes")]
    ExpressionTooLarge,
    #[error("schema-9 set allocation failed")]
    AllocationFailed,
    #[error("schema-9 set needs {requested} bytes with {available} available")]
    BudgetExceeded { requested: u64, available: u64 },
}

/// Charges owned payload and temporary cursor state while each value lives.
#[derive(Debug)]
pub(crate) struct SetMemory {
    limit: u64,
    live: AtomicU64,
    peak: AtomicU64,
}

impl SetMemory {
    pub(crate) fn new(limit: u64) -> Arc<Self> {
        Arc::new(Self {
            limit,
            live: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        })
    }

    fn charge(self: &Arc<Self>, bytes: u64) -> Result<Charge, SetError> {
        let mut live = self.live.load(Ordering::Relaxed);
        loop {
            let available = self.limit.saturating_sub(live);
            if bytes > available {
                return Err(SetError::BudgetExceeded {
                    requested: bytes,
                    available,
                });
            }
            let new_live = live.checked_add(bytes).ok_or(SetError::ByteOverflow)?;
            match self.live.compare_exchange_weak(
                live,
                new_live,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.peak.fetch_max(new_live, Ordering::Relaxed);
                    return Ok(Charge {
                        memory: Arc::clone(self),
                        bytes,
                    });
                }
                Err(observed) => live = observed,
            }
        }
    }

    pub(crate) fn live_bytes(&self) -> u64 {
        self.live.load(Ordering::Relaxed)
    }

    pub(crate) fn peak_bytes(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }
}

#[derive(Debug)]
struct Charge {
    memory: Arc<SetMemory>,
    bytes: u64,
}

impl Charge {
    fn grow(&mut self, additional: u64) -> Result<(), SetError> {
        let mut extra = self.memory.charge(additional)?;
        self.bytes = self
            .bytes
            .checked_add(extra.bytes)
            .ok_or(SetError::ByteOverflow)?;
        extra.bytes = 0;
        Ok(())
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.memory.live.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

enum Payload {
    Sparse(Vec<u32>),
    Dense(Vec<u64>),
}

pub(crate) struct FrozenSet<Id: SetId> {
    universe: Universe<Id>,
    payload: Payload,
    count: u64,
    _charge: Charge,
}

pub(crate) type DocSet = FrozenSet<DocId>;
pub(crate) type VideoSet = FrozenSet<VideoId>;

impl<Id: SetId> FrozenSet<Id> {
    pub(crate) fn from_sorted_raw(
        universe: Universe<Id>,
        raw: &[u32],
        memory: &Arc<SetMemory>,
    ) -> Result<Self, SetError> {
        for (index, &id) in raw.iter().enumerate() {
            if u64::from(id) >= universe.len {
                return Err(SetError::OutOfRange {
                    id,
                    len: universe.len,
                });
            }
            if index > 0 {
                if raw[index - 1] == id {
                    return Err(SetError::Duplicate(id));
                }
                if raw[index - 1] > id {
                    return Err(SetError::Unsorted(index));
                }
            }
        }
        let (ids, charge) = copy_charged(raw, memory)?;
        Ok(Self {
            universe,
            count: ids.len() as u64,
            payload: Payload::Sparse(ids),
            _charge: charge,
        })
    }

    pub(crate) fn from_dense_words(
        universe: Universe<Id>,
        words: &[u64],
        memory: &Arc<SetMemory>,
    ) -> Result<Self, SetError> {
        let expected = dense_word_count(universe.len);
        if words.len() as u64 != expected {
            return Err(SetError::WrongWordCount {
                expected,
                actual: words.len() as u64,
            });
        }
        if !universe.len.is_multiple_of(64) {
            let valid = (1_u64 << (universe.len % 64)) - 1;
            if words.last().is_some_and(|word| word & !valid != 0) {
                return Err(SetError::DirtyTailBits);
            }
        }
        let count = words.iter().map(|word| u64::from(word.count_ones())).sum();
        let (words, charge) = copy_charged(words, memory)?;
        Ok(Self {
            universe,
            payload: Payload::Dense(words),
            count,
            _charge: charge,
        })
    }

    pub(crate) fn len(&self) -> u64 {
        self.count
    }

    pub(crate) fn universe(&self) -> Universe<Id> {
        self.universe
    }

    pub(crate) fn owned_bytes(&self) -> u64 {
        match &self.payload {
            Payload::Sparse(ids) => ids.capacity() as u64 * size_of::<u32>() as u64,
            Payload::Dense(words) => words.capacity() as u64 * size_of::<u64>() as u64,
        }
    }

    /// Numeric membership in this set's own universe; IDs do not carry a generation.
    pub(crate) fn contains_raw(&self, raw: u32) -> bool {
        if u64::from(raw) >= self.universe.len {
            return false;
        }
        match &self.payload {
            Payload::Sparse(ids) => ids.binary_search(&raw).is_ok(),
            Payload::Dense(words) => words[raw as usize / 64] & (1_u64 << (raw % 64)) != 0,
        }
    }
}

fn dense_word_count(universe_len: u64) -> u64 {
    universe_len.div_ceil(64)
}

fn copy_charged<T: Copy>(
    source: &[T],
    memory: &Arc<SetMemory>,
) -> Result<(Vec<T>, Charge), SetError> {
    let bytes = (source.len() as u64)
        .checked_mul(size_of::<T>() as u64)
        .ok_or(SetError::ByteOverflow)?;
    let mut charge = memory.charge(bytes)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(source.len())
        .map_err(|_| SetError::AllocationFailed)?;
    let actual = (output.capacity() as u64)
        .checked_mul(size_of::<T>() as u64)
        .ok_or(SetError::ByteOverflow)?;
    charge.grow(actual.saturating_sub(bytes))?;
    output.extend_from_slice(source);
    Ok((output, charge))
}

pub(crate) enum SetExpr<'a, Id: SetId> {
    Leaf(&'a FrozenSet<Id>),
    And {
        universe: Universe<Id>,
        left: &'a Self,
        right: &'a Self,
    },
    Or {
        universe: Universe<Id>,
        left: &'a Self,
        right: &'a Self,
    },
    Subtract {
        universe: Universe<Id>,
        positive: &'a Self,
        exclude: &'a Self,
    },
}

impl<'a, Id: SetId> SetExpr<'a, Id> {
    pub(crate) fn leaf(set: &'a FrozenSet<Id>) -> Self {
        Self::Leaf(set)
    }

    pub(crate) fn and(left: &'a Self, right: &'a Self) -> Result<Self, SetError> {
        let universe = left.common_universe(right)?;
        Ok(Self::And {
            universe,
            left,
            right,
        })
    }

    pub(crate) fn or(left: &'a Self, right: &'a Self) -> Result<Self, SetError> {
        let universe = left.common_universe(right)?;
        Ok(Self::Or {
            universe,
            left,
            right,
        })
    }

    pub(crate) fn subtract(positive: &'a Self, exclude: &'a Self) -> Result<Self, SetError> {
        let universe = positive.common_universe(exclude)?;
        Ok(Self::Subtract {
            universe,
            positive,
            exclude,
        })
    }

    fn universe(&self) -> Universe<Id> {
        match self {
            Self::Leaf(set) => set.universe(),
            Self::And { universe, .. }
            | Self::Or { universe, .. }
            | Self::Subtract { universe, .. } => *universe,
        }
    }

    fn common_universe(&self, other: &Self) -> Result<Universe<Id>, SetError> {
        let universe = self.universe();
        universe.check(other.universe())?;
        Ok(universe)
    }

    fn node_count(&self) -> Result<u64, SetError> {
        self.node_count_at_depth(1)
    }

    fn node_count_at_depth(&self, depth: u32) -> Result<u64, SetError> {
        if depth > MAX_EXPRESSION_DEPTH {
            return Err(SetError::ExpressionTooDeep);
        }
        let count = match self {
            Self::Leaf(_) => Ok(1),
            Self::And { left, right, .. } | Self::Or { left, right, .. } => left
                .node_count_at_depth(depth + 1)?
                .checked_add(right.node_count_at_depth(depth + 1)?)
                .and_then(|count| count.checked_add(1))
                .ok_or(SetError::ByteOverflow),
            Self::Subtract {
                positive, exclude, ..
            } => positive
                .node_count_at_depth(depth + 1)?
                .checked_add(exclude.node_count_at_depth(depth + 1)?)
                .and_then(|count| count.checked_add(1))
                .ok_or(SetError::ByteOverflow),
        }?;
        if count > MAX_EXPRESSION_NODES {
            return Err(SetError::ExpressionTooLarge);
        }
        Ok(count)
    }

    pub(crate) fn cursor(&self, memory: &Arc<SetMemory>) -> Result<SetCursor<'_, Id>, SetError> {
        let bytes = self
            .node_count()?
            .checked_mul(size_of::<CursorNode<'_>>() as u64)
            .ok_or(SetError::ByteOverflow)?;
        let charge = memory.charge(bytes)?;
        Ok(SetCursor {
            node: CursorNode::from_expr(self),
            _charge: charge,
            marker: PhantomData,
        })
    }

    pub(crate) fn count(&self, memory: &Arc<SetMemory>) -> Result<u64, SetError> {
        if let Self::Leaf(set) = self {
            return Ok(set.len());
        }
        let direct_leaves = match self {
            Self::And { left, right, .. } | Self::Or { left, right, .. } => Some((*left, *right)),
            Self::Subtract {
                positive, exclude, ..
            } => Some((*positive, *exclude)),
            Self::Leaf(_) => None,
        };
        if let Some((Self::Leaf(left), Self::Leaf(right))) = direct_leaves {
            let overlap = leaf_overlap_count(left, right);
            return match self {
                Self::And { .. } => Ok(overlap),
                Self::Or { .. } => left
                    .count
                    .checked_add(right.count)
                    .and_then(|sum| sum.checked_sub(overlap))
                    .ok_or(SetError::ByteOverflow),
                Self::Subtract { .. } => left
                    .count
                    .checked_sub(overlap)
                    .ok_or(SetError::ByteOverflow),
                Self::Leaf(_) => unreachable!(),
            };
        }
        let mut cursor = self.cursor(memory)?;
        let mut count = 0_u64;
        while cursor.next().is_some() {
            count = count.checked_add(1).ok_or(SetError::ByteOverflow)?;
        }
        Ok(count)
    }

    pub(crate) fn materialize(&self, memory: &Arc<SetMemory>) -> Result<FrozenSet<Id>, SetError> {
        let count = self.count(memory)?;
        let universe = self.universe();
        let sparse_bytes = count.checked_mul(4).ok_or(SetError::ByteOverflow)?;
        let dense_bytes = dense_word_count(universe.len)
            .checked_mul(8)
            .ok_or(SetError::ByteOverflow)?;
        if count == 0 || sparse_bytes <= dense_bytes {
            let mut charge = memory.charge(sparse_bytes)?;
            let count = usize::try_from(count).map_err(|_| SetError::ByteOverflow)?;
            let mut ids = Vec::new();
            ids.try_reserve_exact(count)
                .map_err(|_| SetError::AllocationFailed)?;
            let actual = ids.capacity() as u64 * size_of::<u32>() as u64;
            charge.grow(actual.saturating_sub(sparse_bytes))?;
            let mut cursor = self.cursor(memory)?;
            while let Some(id) = cursor.next() {
                ids.push(id.raw());
            }
            if ids.len() != count {
                return Err(SetError::ByteOverflow);
            }
            Ok(FrozenSet {
                universe,
                count: count as u64,
                payload: Payload::Sparse(ids),
                _charge: charge,
            })
        } else {
            let mut charge = memory.charge(dense_bytes)?;
            let words = usize::try_from(dense_word_count(universe.len))
                .map_err(|_| SetError::ByteOverflow)?;
            let mut bits = Vec::new();
            bits.try_reserve_exact(words)
                .map_err(|_| SetError::AllocationFailed)?;
            let actual = bits.capacity() as u64 * size_of::<u64>() as u64;
            charge.grow(actual.saturating_sub(dense_bytes))?;
            bits.resize(words, 0);
            let observed = if self.fill_direct_dense(&mut bits) {
                bits.iter().map(|word| u64::from(word.count_ones())).sum()
            } else {
                let mut cursor = self.cursor(memory)?;
                let mut observed = 0_u64;
                while let Some(id) = cursor.next() {
                    let raw = id.raw();
                    bits[raw as usize / 64] |= 1_u64 << (raw % 64);
                    observed += 1;
                }
                observed
            };
            if observed != count {
                return Err(SetError::ByteOverflow);
            }
            Ok(FrozenSet {
                universe,
                count,
                payload: Payload::Dense(bits),
                _charge: charge,
            })
        }
    }

    /// Fills a dense result directly when leaf encodings permit wordwise work.
    fn fill_direct_dense(&self, output: &mut [u64]) -> bool {
        match self {
            Self::And { left, right, .. }
                if let (Self::Leaf(a), Self::Leaf(b)) = (*left, *right)
                    && let (Payload::Dense(a), Payload::Dense(b)) = (&a.payload, &b.payload) =>
            {
                for ((out, a), b) in output.iter_mut().zip(a).zip(b) {
                    *out = a & b;
                }
                true
            }
            Self::Or { left, right, .. }
                if let (Self::Leaf(a), Self::Leaf(b)) = (*left, *right) =>
            {
                match (&a.payload, &b.payload) {
                    (Payload::Dense(a), Payload::Dense(b)) => {
                        for ((out, a), b) in output.iter_mut().zip(a).zip(b) {
                            *out = a | b;
                        }
                    }
                    (Payload::Dense(a), Payload::Sparse(b))
                    | (Payload::Sparse(b), Payload::Dense(a)) => {
                        output.copy_from_slice(a);
                        for &id in b {
                            output[id as usize / 64] |= 1_u64 << (id % 64);
                        }
                    }
                    (Payload::Sparse(_), Payload::Sparse(_)) => return false,
                }
                true
            }
            Self::Subtract {
                positive, exclude, ..
            } if let (Self::Leaf(a), Self::Leaf(b)) = (*positive, *exclude) => {
                match (&a.payload, &b.payload) {
                    (Payload::Dense(a), Payload::Dense(b)) => {
                        for ((out, a), b) in output.iter_mut().zip(a).zip(b) {
                            *out = a & !b;
                        }
                    }
                    (Payload::Dense(a), Payload::Sparse(b)) => {
                        output.copy_from_slice(a);
                        for &id in b {
                            output[id as usize / 64] &= !(1_u64 << (id % 64));
                        }
                    }
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }
}

fn leaf_overlap_count<Id: SetId>(left: &FrozenSet<Id>, right: &FrozenSet<Id>) -> u64 {
    match (&left.payload, &right.payload) {
        (Payload::Sparse(a), Payload::Sparse(b)) => {
            let (mut ai, mut bi, mut count) = (0, 0, 0);
            while ai < a.len() && bi < b.len() {
                match a[ai].cmp(&b[bi]) {
                    std::cmp::Ordering::Less => ai += 1,
                    std::cmp::Ordering::Greater => bi += 1,
                    std::cmp::Ordering::Equal => {
                        count += 1;
                        ai += 1;
                        bi += 1;
                    }
                }
            }
            count
        }
        (Payload::Sparse(ids), Payload::Dense(words))
        | (Payload::Dense(words), Payload::Sparse(ids)) => ids
            .iter()
            .filter(|&&id| words[id as usize / 64] & (1_u64 << (id % 64)) != 0)
            .count() as u64,
        (Payload::Dense(a), Payload::Dense(b)) => a
            .iter()
            .zip(b)
            .map(|(left, right)| u64::from((left & right).count_ones()))
            .sum(),
    }
}

pub(crate) struct SetCursor<'a, Id: SetId> {
    node: CursorNode<'a>,
    _charge: Charge,
    marker: PhantomData<Id>,
}

impl<Id: SetId> SetCursor<'_, Id> {
    pub(crate) fn current(&mut self) -> Option<Id> {
        self.node.peek().map(Id::from_raw)
    }

    pub(crate) fn advance(&mut self, target: Id) -> Option<Id> {
        self.node.advance(target.raw()).map(Id::from_raw)
    }

    pub(crate) fn next(&mut self) -> Option<Id> {
        self.node.next().map(Id::from_raw)
    }
}

enum CursorNode<'a> {
    Sparse {
        ids: &'a [u32],
        offset: usize,
    },
    Dense {
        words: &'a [u64],
        word_index: usize,
        remaining: u64,
    },
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
    Subtract(Box<Self>, Box<Self>),
}

impl<'a> CursorNode<'a> {
    fn from_expr<Id: SetId>(expr: &'a SetExpr<'_, Id>) -> Self {
        match expr {
            SetExpr::Leaf(set) => match &set.payload {
                Payload::Sparse(ids) => Self::Sparse { ids, offset: 0 },
                Payload::Dense(words) => Self::Dense {
                    words,
                    word_index: 0,
                    remaining: words.first().copied().unwrap_or(0),
                },
            },
            SetExpr::And { left, right, .. } => Self::And(
                Box::new(Self::from_expr(left)),
                Box::new(Self::from_expr(right)),
            ),
            SetExpr::Or { left, right, .. } => Self::Or(
                Box::new(Self::from_expr(left)),
                Box::new(Self::from_expr(right)),
            ),
            SetExpr::Subtract {
                positive, exclude, ..
            } => Self::Subtract(
                Box::new(Self::from_expr(positive)),
                Box::new(Self::from_expr(exclude)),
            ),
        }
    }

    fn peek(&mut self) -> Option<u32> {
        match self {
            Self::Sparse { ids, offset } => ids.get(*offset).copied(),
            Self::Dense {
                words,
                word_index,
                remaining,
            } => {
                while *remaining == 0 {
                    if *word_index >= words.len() {
                        return None;
                    }
                    *word_index += 1;
                    *remaining = words.get(*word_index).copied()?;
                }
                let id = *word_index as u64 * 64 + u64::from(remaining.trailing_zeros());
                u32::try_from(id).ok()
            }
            Self::And(left, right) => loop {
                let (Some(a), Some(b)) = (left.peek(), right.peek()) else {
                    return None;
                };
                if a == b {
                    return Some(a);
                }
                if a < b {
                    left.advance(b);
                } else {
                    right.advance(a);
                }
            },
            Self::Or(left, right) => match (left.peek(), right.peek()) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            },
            Self::Subtract(positive, exclude) => loop {
                let candidate = positive.peek()?;
                if exclude.advance(candidate) == Some(candidate) {
                    positive.next();
                } else {
                    return Some(candidate);
                }
            },
        }
    }

    fn advance(&mut self, target: u32) -> Option<u32> {
        match self {
            Self::Sparse { ids, offset } => {
                *offset += ids[*offset..].partition_point(|&id| id < target);
            }
            Self::Dense {
                words,
                word_index,
                remaining,
            } => {
                let target_word = target as usize / 64;
                if *word_index < target_word {
                    *word_index = target_word;
                    *remaining = words.get(target_word).copied().unwrap_or(0);
                }
                if *word_index == target_word {
                    *remaining &= u64::MAX << (target % 64);
                }
            }
            Self::And(left, right) | Self::Or(left, right) => {
                left.advance(target);
                right.advance(target);
            }
            Self::Subtract(positive, _) => {
                positive.advance(target);
            }
        }
        self.peek()
    }

    fn next(&mut self) -> Option<u32> {
        let candidate = self.peek()?;
        match self {
            Self::Sparse { offset, .. } => *offset += 1,
            Self::Dense { remaining, .. } => *remaining &= *remaining - 1,
            Self::And(left, right) => {
                left.next();
                right.next();
            }
            Self::Or(left, right) => {
                if left.peek() == Some(candidate) {
                    left.next();
                }
                if right.peek() == Some(candidate) {
                    right.next();
                }
            }
            Self::Subtract(positive, _) => {
                positive.next();
            }
        }
        Some(candidate)
    }
}

#[cfg(test)]
#[path = "tests/sets.rs"]
mod tests;
