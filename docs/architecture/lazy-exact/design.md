# Lazy exact Raw engine: usage-first architecture

Status: architecture sketch only. Types, signatures, and module boundaries below precede implementation and remain subject to prototype evidence.

## User-visible flow

```text
POST /raw/search { query, youtuber?, sort, filters?, per_page }
  -> exact totals + exact Tier-0 overview + page 1
  -> generation + opaque session_cursor + timing fields

POST /raw/search/page { cursor }
  -> next exact window from the retained rank frontier

POST /raw/search/summary { cursor, tier, dimensions }
  -> exact summary of the complete pinned match set

POST /raw/search/video-moments { cursor, video_id }
  -> every exact moment for that video in the same snapshot
```

Existing GET parameters and response fields remain compatible. The frontend keeps the opaque cursor in history state. A fresh deep `page=N` URL honestly computes preceding windows when no retained chain exists; it does not claim constant-time random access. If the active generation advances, an old session continues on its pinned generation and returns `results_may_have_changed: true` plus an explicit refresh action.

## Domain types

```rust
#[repr(transparent)] struct GenerationId(u64);
#[repr(transparent)] struct SegmentId(u32);
#[repr(transparent)] struct DocId(u32);
#[repr(transparent)] struct VideoId(u32);
#[repr(transparent)] struct QueryFingerprint([u8; 16]);

struct DocSet(FrozenSet<DocId>);
struct VideoSet(FrozenSet<VideoId>);

enum FrozenSet<Id> {
    Dense(DenseBits<Id>),
    Roaring(FrozenRoaring<Id>),
    Sparse(SkipBackedSet<Id>),
}

struct GenerationLease {
    id: GenerationId,
    generation: Arc<Generation>,
}
```

The typed universes prevent document/video intersections and counts from being confused. Sets are immutable once admitted to a session cache. Query-local mutable scratch storage is byte-accounted.

## Query planning and exact set construction

```rust
enum QueryNode {
    Term(TermExpr),
    And(Vec<QueryNode>),
    Or(Vec<QueryNode>),
    Phrase(PhraseExpr),
    Not { include: Box<QueryNode>, exclude: Box<QueryNode> },
    Filter(FilterExpr),
}

trait DocIterator {
    fn next(&mut self) -> Result<Option<DocId>>;
    fn advance(&mut self, target: DocId) -> Result<Option<DocId>>;
    fn estimated_cost(&self) -> IteratorCost;
}

trait ExactSetBuilder {
    fn materialize(&mut self, scratch: &mut QueryArena) -> Result<DocSet>;
    fn exact_count(&mut self, scratch: &mut QueryArena) -> Result<u64>;
}
```

Parsing and validation fail closed. Every exact, prefix, typo, split, token-drop, phrase, negation, and explicit-filter alternative admitted by the compatibility grammar appears in the tree. Planner choices change representation and execution order only.

Term cursors seek through one canonical score-bearing posting stream with skip checkpoints around 8/64/512/4096 occurrences. High-DF term/field leaves may expose an exact bitmap selected by the generation's deterministic byte budget. AND uses bitmap algebra or rare-lead leapfrog; OR uses bitmap union or a loser tree; NOT subtracts only from an explicit positive universe.

Phrase nodes intersect exact candidate sets first, then load the separate position stream only for survivors. Highlight positions remain lazy until returned documents are rendered.

## Totals and summaries

```rust
struct ExactTotals {
    documents: u64,
    videos: u64,
    moments: u64,
}

struct Tier0Overview {
    creators: Vec<BucketCount<CreatorId>>,
    content_kinds: Vec<BucketCount<ContentKind>>,
    years: Vec<BucketCount<Year>>,
    durations: Vec<BucketCount<DurationBucket>>,
}

enum SummaryRequest {
    Tier1 { histograms: NonEmpty<NumericDimension> },
    Tier2 { aggregates: NonEmpty<ExactAggregate> },
    Tier3Approximate { operations: NonEmpty<ApproximateAggregate> },
}

trait AggregateEngine {
    fn totals(&self, docs: &DocSet) -> Result<(ExactTotals, VideoSet)>;
    fn tier0(&self, videos: &VideoSet) -> Result<Tier0Overview>;
    fn summarize(&self, videos: &VideoSet, request: SummaryRequest) -> Result<SummaryResult>;
}
```

Document count is set cardinality. Video projection is exact and produces one `VideoSet`. Moment count applies the existing greedy 20-second reducer while streaming selected `(video_id, t_start_ms)` fast fields in order; it is never replaced with fixed buckets or term cardinality. Tier 0 intersects the video set with exact creator/kind/year/duration facet sets. Tier 1 scans matching video IDs over bitpacked numeric columns. Tier 2 streams exact whole-set aggregations on demand. Only the distinct Tier-3 variant may be approximate, and both request and response label it.

## Exact resumable ranking

```rust
enum RankProgram {
    Relevance(RelevanceProgram),
    RelevanceEngagement(CompositeProgram),
    Views(FastFieldProgram),
    Likes(FastFieldProgram),
    Comments(FastFieldProgram),
    PublishedAsc(FastFieldProgram),
    PublishedDesc(FastFieldProgram),
}

struct RankKey {
    token_coverage: CoverageKey,
    phrase_and_position: PositionKey,
    text_score: OrderedScore,
    engagement: EngagementKey,
    stable_video: VideoId,
    stable_doc: DocId,
}

struct RankFrontier {
    unexpanded: BinaryHeap<BoundedBlock>,
    candidates: BinaryHeap<RankedVideo>,
    partial_videos: VideoAccumulatorMap,
    emitted: Vec<PageWindow>,
    counters: FrontierCounters,
}

trait ExactRanker {
    fn next_page(&mut self, matches: &DocSet, size: NonZeroU16) -> Result<PageWindow>;
}
```

Every posting block stores a conservative `MaxRankKey` covering all ordering dimensions. A component without a proved maximum uses its global maximum, reducing pruning rather than correctness. Grouped video accumulators participate in the bound. A page is final only when no unexplored or partial block can beat its last complete rank tuple.

The frontier persists inside the live session. Page N+1 resumes offsets and heaps, and counters prove it neither reopens exhausted blocks nor rescores documents consumed for earlier windows.

## Sessions and cursors

```rust
struct SearchSession {
    lease: GenerationLease,
    fingerprint: QueryFingerprint,
    query: Arc<NormalizedQuery>,
    documents: Arc<DocSet>,
    videos: Arc<VideoSet>,
    totals: ExactTotals,
    tier0: Tier0Overview,
    rank: Mutex<RankFrontier>,
    expiry: Instant,
}

struct CursorClaims {
    version: CursorVersion,
    session: SessionId,
    generation: GenerationId,
    query: QueryFingerprint,
    sequence: u32,
    expiry_unix_ms: u64,
}

enum SessionLookup {
    Current(Arc<SearchSession>),
    Superseded(Arc<SearchSession>),
    Expired { refresh: CanonicalRequest },
}
```

Cursors are authenticated and contain no trusted offsets or serialized heaps. Activation changes only the generation used for new sessions. A lease keeps old mmaps and files available. Cleanup requires: not active, outside rollback retention, no strong session leases, and grace elapsed. Restart may expire in-memory sessions explicitly; it never rebinds a cursor to another generation. Durable frontier persistence is deferred until prototype evidence shows it is necessary.

## Schema-9 structural files

```text
manifest.footer        schema/generation/corpus identity, file and unit directory
terms.fst              term -> per-field posting descriptor
postings.docs          block-coded doc deltas and term frequencies
postings.skips         8/64/512/4096 seek checkpoints
postings.blockmax      complete conservative rank bounds
postings.bitmaps       byte-budgeted high-DF field occurrence pages
positions.first        separately addressed first positions
positions.rest         cold remaining positions
documents.fast         bitpacked video/time/length/field flags
videos.fast            engagement/date/duration/kind/creator columns
videos.ranges          contiguous document range per video
facets/*               exact video-level sparse or frozen-Roaring sets
documents.blocks       independently compressed response payloads
checksums.units        independently verifiable 2–8 MiB byte ranges
```

Open validates only the footer, directories, and bounded hot metadata. Each immutable unit is checksummed and structurally validated once on first use through a single-flight state. The builder performs full verification before publication. Reader cold open must physically read under 200 MB.

## Module map

```text
crates/youtube_raw_search/src/
  engine/
    query/ast.rs             validated normalized grammar
    query/plan.rs            representation/cost selection without semantic caps
    postings/cursor.rs       block decoder and multi-level seek
    postings/skips.rs        checkpoint directories
    postings/block_max.rs    complete rank bounds
    sets/doc_set.rs          typed exact document sets
    sets/video_set.rs        typed exact video sets
    positions/verify.rs      phrase/proximity candidate verification
    ranking/frontier.rs      resumable grouped branch-and-bound
    ranking/program.rs       sort-specific exact rank programs
    aggregate/totals.rs      document/video/20-second moment totals
    aggregate/tier0.rs       exact facet overview
    aggregate/histogram.rs   exact Tier-1 numeric histograms
    fast_fields.rs           bitpacked document/video columns
  generation/
    footer.rs                footer and unit directory
    verification.rs          lazy single-flight checks
    registry.rs              active/retained generations and leases
  session/
    cursor.rs                versioned authenticated claims
    store.rs                 byte/TTL-bounded sessions
    state.rs                 match sets, summaries, rank frontier
  schema9/
    builder.rs               structural writer; no query-plan inputs
    measure.rs               throwaway prototype instrumentation
    manifest.rs              sizes, budgets, compatibility metadata
```

Existing tokenization, fuzzy expansion, document payloads, canonical archives, WAL snapshot, publisher receipts, activation protocol, and response adapters are reused when differential tests prove compatibility. `broad-single.bin` is ignored by schema 9 and retained untouched as rollback evidence.

## Prototype-to-PR sequence

1. Measurement harness only: corpus histogram, posting/block codecs, skip levels, bitmap crossover, moment scan, histograms, ranking frontier, and session pinning forks.
2. Exact iterator/set layer plus exhaustive reference oracle; no HTTP or format freeze.
3. Schema-9 structural writer/reader with lazy unit verification and artifact-shape assertion.
4. Exact summaries and generation-pinned session/cursor contract.
5. Resumable complete-bound ranking and every-page differential tests.
6. Compatible API/frontend integration and the complete website regression lane.
7. Separate 2 GB live canary with publication overlap, activation, restart, rollback, memory, cold-read, and worst-query evidence.

Every PR carries unit, differential, resource, and latency evidence appropriate to its boundary. Production stays on generation 2145 until all gates pass and the user explicitly approves a switch.
