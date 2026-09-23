# Schema-9 exact set API

Status: implementation slice for P02. This module is internal and is not yet
connected to the production reader.

## Caller usage

```rust,ignore
let memory = SetMemory::new(query_byte_budget);
let universe = Universe::<DocId>::new(generation, segment, document_count)?;
let title = FrozenSet::from_sorted_raw(universe, title_ids, &memory)?;
let text = FrozenSet::from_dense_words(universe, text_bits, &memory)?;
let exclude = FrozenSet::from_sorted_raw(universe, negative_ids, &memory)?;

let title = SetExpr::leaf(&title);
let text = SetExpr::leaf(&text);
let positive = SetExpr::or(&title, &text)?;
let exclude = SetExpr::leaf(&exclude);
let query = SetExpr::subtract(&positive, &exclude)?;
let exact_count = query.count(&memory)?;
let retained = query.materialize(&memory)?;
```

`DocId` and `VideoId` have different Rust types. A set expression checks the
complete generation, segment, and length identity even when one operand is
empty. Subtraction requires an explicit positive left operand. The set module
does not expose complement of the entire universe.

## Types and operation contract

```rust,ignore
struct GenerationId(u64);
struct SegmentId(u32);
struct DocId(u32);
struct VideoId(u32);
struct Universe<Id> { generation: GenerationId, segment: SegmentId, len: u64 }
struct FrozenSet<Id> { universe: Universe<Id>, payload: Sparse | Dense, count: u64 }
enum SetExpr<'a, Id> {
    Leaf(&'a FrozenSet<Id>),
    And(&'a SetExpr<'a, Id>, &'a SetExpr<'a, Id>),
    Or(&'a SetExpr<'a, Id>, &'a SetExpr<'a, Id>),
    Subtract(&'a SetExpr<'a, Id>, &'a SetExpr<'a, Id>),
}
```

The universe length is `0..=2^32`; at the upper bound `u32::MAX` is a valid
ID. Constructors reject unsorted, duplicate, and out-of-range sparse IDs and
wrong-length or dirty-tail dense words. They publish only immutable valid
sets. A dense cursor scans words once with bit operations and jumps directly
to the target word on seek. Sparse seeks use binary partitioning over the
remaining IDs. Boolean cursors push seeks down to their operands. Expressions
are bounded to 64 levels and 4096 expanded nodes before cursor construction.
Boolean cursors
stream increasing IDs, counting without a result allocation. Direct leaf pairs
count by sparse merge, sparse bitmap probes, or wordwise bitmap operations.
Materialization
counts first, chooses the smaller of exact sparse and dense payload costs with
checked `u64` arithmetic, then traverses again to fill one representation.
It never builds a full vector of the dense input's IDs as scratch.

The generic cursor is the exactness baseline for later specialized wordwise
operations. `SetMemory` charges owned vector capacity and an estimate for cursor
nodes while each value lives. It does not yet include allocator metadata,
borrowed decoder inputs, or the expression nodes held on the caller's stack.
An allocator may reserve more capacity than requested before that excess is
charged; the temporary allocation peak is not a hard RSS bound.
The full query/session memory ledger must include those costs and conversion
peaks before the 2 GiB release gate. Passing this unit's tests does not prove
that gate or the 200 ms search target.

## Design choice

Two independent sketches were compared. The streaming design is the base
because exact counting can avoid materializing a match set. The immutable-set
design contributed the
one-time validation boundary, complete universe mismatch diagnostics, and
explicit owned-byte accounting. An independent review agreed while
correcting both sketches to support the full `2^32`-ID universe. A's always
materialized algebra was rejected for the initial query path. B's private
binary expression tree remains internal so the planner can later flatten or
specialize broad OR plans without changing a public API.
