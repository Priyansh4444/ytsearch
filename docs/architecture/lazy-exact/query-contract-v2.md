# Raw query contract: observed legacy and schema-9 v2

Status: **P00 policy reviewed; executable semantic gates pending**  
Scope: query parsing, match membership, score reduction, video/moment ranking, and request validation  
Observed implementation: `schema9-p00` at `97f6031`

This document deliberately has two contracts:

- **Legacy observed** describes what the current Raw reader does. It is evidence for compatibility; it is not automatically desirable behavior.
- **Schema-9 v2** describes the behavior the lazy exact engine must implement. A row marked **FAILED GATE** is not implementer discretion. P01/P06 cannot claim semantic or ranking completion until that row has an approved value and golden fixtures.

Execution strategy, posting representation, candidate cardinality, bitmap availability, and page depth may not change v2 membership or ordering.

## Source of truth and precedence

The observations below are derived from:

- [`engine.rs`](../../../crates/youtube_raw_search/src/engine.rs), which selects wildcard, quoted, single-token, conjunction, typo, and token-drop paths and formats results;
- [`query_support.rs`](../../../crates/youtube_raw_search/src/engine/query_support.rs), which parses and validates queries, defines score formulas, counts moments, and compares videos;
- [`conjunction.rs`](../../../crates/youtube_raw_search/src/engine/conjunction.rs), which constructs exact/prefix/typo alternatives and reduces their contributions;
- [`single_token.rs`](../../../crates/youtube_raw_search/src/engine/single_token.rs), which defines compact-token splitting and the single-token fallback ladder;
- [`youtube_search_service`](../../../crates/youtube_search_service/src/lib.rs), which defines the accepted HTTP parameter shape;
- [`design.md`](./design.md) and the normative requirements in [`schema9.md`](../../../schema9.md).

For v2, this document takes precedence over legacy implementation comments. Tests and examples do not override this document; disagreements are contract failures and require an explicit contract revision.

## Terms and identities

| Name | Meaning |
| --- | --- |
| query slot | Position of a distinct, positive, normalized unquoted token in canonical query order, or a token occurrence inside a phrase |
| indexed term | Canonical term bytes selected from the lexicon for one exact, prefix, typo, or split branch |
| candidate document | Transcript chunk admitted by the complete query predicate; a title match may expand to the video's documents for ranking/response compatibility |
| transcript provenance | The candidate matched the transcript field under the admitted predicate |
| title provenance | The candidate's video title matched under the admitted predicate |
| complete text score | Every v2 contribution and multiplier specified below, evaluated in canonical order regardless of execution path |
| external video ID | Stable YouTube video identifier; never a generation-local dense ID |
| exact bits | IEEE-754 binary32 bits produced by the specified operation order |

Canonical contribution identity is the tuple:

```text
(query_slot, indexed_term_utf8, field, expansion_kind, split_component)
```

`field` is `text` or `title`. `expansion_kind` is one of `exact`, `prefix`, `split_exact`, `split_fuzzy`, `typo_1`, or `typo_2`. `split_component` is absent except for a split branch, where it is left or right plus its normalized bytes.

## Grammar

### Tokenization shared by the current index and query path

Legacy `ordered_tokens` reads Unicode scalar values, retains runs for which Rust `char::is_alphanumeric()` is true, lowercases each retained character with `to_lowercase()`, and treats every other character as a separator. It does **not** explicitly apply NFC, NFD, NFKC, case folding, stemming, stop-word removal, or accent folding.

Consequences that are part of the observation, not assumptions:

- punctuation does not survive inside a term;
- a hyphen is normally a separator, except when parsed as a leading negation marker;
- lowercasing can emit more than one scalar value;
- byte length, not character count, controls legacy typo thresholds;
- canonically equivalent Unicode strings are not guaranteed to produce identical term bytes.

### Syntax table

| Input form | Legacy observed | Schema-9 v2 contract | Status |
| --- | --- | --- | --- |
| `alpha beta` | distinct positive tokens, deduplicated by first appearance | AND of distinct positive tokens in first-appearance order | **FROZEN** |
| `alpha alpha` | one positive term; duplicate unquoted occurrences collapse | one positive query slot; no duplicate score | **FROZEN** |
| `"alpha beta"` | positive exact ordered phrase; phrase tokens also enter the positive term list | positive phrase predicate plus its required term predicates | **FROZEN** |
| `"alpha alpha"` | repeated phrase occurrences remain in the position test although the positive term list deduplicates | two ordered positional occurrences are required | **FROZEN** |
| `-alpha` | negative exact term when `-` begins a token at an allowed boundary | subtract exact-term predicate from an explicit positive universe | **FROZEN** |
| `-"alpha beta"` | negative zero-slop phrase | subtract exact phrase predicate from an explicit positive universe | **FROZEN** |
| `alpha -beta` | positive query AND NOT exact `beta` | same | **FROZEN** |
| `alpha -"beta gamma"` | positive query AND NOT exact phrase | same | **FROZEN** |
| only negative terms | parser produces no positive terms; legacy falls into wildcard/all then subtracts | explicit full-document universe followed by subtraction | **FROZEN** |
| `*` | wildcard; one visible moment per matched video in grouped mode | explicit wildcard mode, one visible moment per matched video | **FROZEN** |
| punctuation only | parser produces no positive tokens and legacy follows the same all-documents branch as wildcard | reject with `invalid_query`; only the trimmed literal `*` selects wildcard | **DECIDED QG-01; EXECUTABLE GATE PENDING** |
| empty or whitespace | rejected before parsing with `q cannot be empty` | reject | **FROZEN** |
| unmatched `"` | rejected with `query contains an unterminated quoted phrase` | reject with stable invalid-input category | **FROZEN** |
| empty quoted phrase | rejected with `quoted phrase must contain at least one searchable token` | reject | **FROZEN** |
| phrase over 32 tokens | rejected | reject; maximum 32 | **FROZEN** |
| backslash before `"` or `\` | consumes the escaped character literally into the current segment | retain this two-character escape grammar | **FROZEN** |
| backslash before another character | backslash becomes a separator during tokenization | retain unless QG-02 changes normalization grammar | provisional |
| lone `-` at a negative boundary | rejected: negative marker must prefix a searchable token | reject | **FROZEN** |
| `word-word` | two positive terms, not negation | same | **FROZEN** |
| quoted phrase across title/text | cannot cross fields | never crosses fields | **FROZEN** |
| quoted phrase across transcript chunks | cannot cross chunks | never crosses chunks | **FROZEN** |
| Unicode-equivalent spellings | no explicit normalization; equality is produced lowercase term bytes | normalize the complete input to NFC, tokenize alphanumeric runs, apply locale-neutral Unicode lowercase, then NFC-normalize each emitted token; no compatibility folding | **DECIDED QG-02; EXECUTABLE GATE PENDING** |
| numerical tokens | accepted as alphanumeric terms; typo threshold is based on UTF-8 byte length, with no numeric exception | pure Unicode-decimal tokens are exact-only with no implicit prefix; mixed alphanumeric tokens use normal expansion; nonnumeric length thresholds count Unicode scalars after normalization | **DECIDED QG-03; EXECUTABLE GATE PENDING** |

Negation markers are recognized only at the beginning of the query or following whitespace. A negative query must always have a named universe. The v2 AST must encode `All AND NOT(...)` for a negative-only query; subtraction from an empty set is forbidden.

### Positive expansion ladder

Legacy uses these constants:

| Behavior | Value |
| --- | ---: |
| typo result threshold | 5 results (videos when grouped, documents otherwise) |
| token-drop result threshold | 1 |
| maximum token-drop candidates in either ladder | 20 |
| fuzzy terms per edit distance | 4 |
| one typo permitted | term UTF-8 byte length at least 4 |
| two typos permitted | term UTF-8 byte length at least 7 |
| split minimum side length | 3 Unicode scalar values |
| maximum exact split alternatives | 8 |
| split posting-work budget | 1,000,000 estimated postings |
| split proximity maximum matches | 50,000 |

Legacy expansion order is path-dependent:

1. Exact terms.
2. Prefix alternatives for the final retained token only. Prefix score multiplier is `0.9`.
3. For a sparse single token, exact missing-separator split, then a constrained one-component fuzzy split, then whole-token edit-distance alternatives. Split branches are not exact evidence for the original token.
4. For multi-token input, bounded conjunctive alternatives remain AND by query token. Typo distance is introduced only while the threshold is not met.
5. If the full query does not meet the configured threshold, drop tokens right-to-left by trying longest retained prefixes first, then retained suffixes. At most 20 candidates per ladder are tried.
6. Edit-distance-one multiplier is `0.65`; edit-distance-two multiplier is `0.4`.

Schema-9 v2 freezes the accepted *families* and the ordered token-drop policy, but it may not let a work cap remove a semantically admitted result. `MAX_FUZZY_TERMS_PER_DISTANCE`, split alternative count, split posting budget, and 20-candidate token-drop limits are therefore **legacy execution caps**, not yet valid v2 semantic limits.

| Question | Required resolution |
| --- | --- |
| Which lexicon alternatives are semantically admitted when more than four have equal edit distance? | **DECIDED QG-04; EXECUTABLE GATE PENDING:** admit every term within the permitted distance; reduce in distance then normalized-term-byte order. DF may schedule work but never change membership. |
| Does an exact split remain valid after the legacy one-million-posting budget is exceeded? | **DECIDED QG-05; EXECUTABLE GATE PENDING:** every qualifying exact split is semantic regardless of posting work or alternative count. Execution must remain lazy without dropping membership. |
| Do queries longer than 21 distinct terms continue token dropping beyond the legacy ladders? | **DECIDED QG-06; EXECUTABLE GATE PENDING:** after the full predicate, try every prefix from n-1 through 1, then every suffix from index 1 through n-1; the first predicate meeting the threshold wins. The 512-byte query limit bounds the ladder. |
| Is prefix matching enabled per field identically? | **DECIDED QG-07; EXECUTABLE GATE PENDING:** apply the same complete normalized-prefix rule independently to each enabled field, then union while retaining field provenance. |

The QG-01 through QG-07 policy choices are reviewed and recorded in the matrix, but their executable semantic gates remain pending. P03 must implement one shared tokenizer for index and query paths and prove these choices against an exhaustive reference; standalone policy examples are not sufficient.

## Request validation

Unknown query-string fields are rejected by Serde's `deny_unknown_fields`; they must remain fail-closed in v2.

| Parameter | Legacy accepted value | v2 |
| --- | --- | --- |
| `q` | nonblank, at most 512 UTF-8 bytes | same |
| `query_by` | `text,title`, `title,text`, `text`, or `title`; no duplicate/empty field | same |
| `query_by_weights` | exactly `4,6`, `6,4`, `4`, or `6` corresponding to field order | same compatibility surface; weights map to frozen field constants |
| `filter_by` | one exact `video_id:=value` or `youtuber_handle:=value`; optional balanced backticks; max value 256 bytes | same; parsed as an exact predicate |
| `group_by` | absent or `video_id` | same |
| `group_limit` | 1 through 250; default 5 in grouped output | same |
| `per_page` | 0 through 250; default 10 | **DECIDED QV-01; EXECUTABLE GATE PENDING:** zero is a successful count-only window with exact totals/Tier 0, empty hits, no cursor, and `has_next_page=false` |
| `page` | any `u32`; zero is coerced to one | preserve for compatible GET API |
| `limit` | 0 through 250; mutually exclusive with page/per_page | zero has the same successful count-only semantics as `per_page=0` |
| `offset` | `u32`, only with `limit` | same |
| `sort_by` | six explicit expressions below or absent | same transport strings |
| `text_match_type` | absent or `sum_score` | same |
| `prioritize_exact_match` | absent or true | same |
| `prioritize_token_position` | absent or true | same |
| `phrase_slop` | 0 through 8 and requires a positive quoted phrase | same |
| `typo_tokens_threshold` | absent or exactly 5 | same compatibility input |
| `drop_tokens_threshold` | absent or exactly 1 | same compatibility input |
| `drop_tokens_mode` | absent or `right_to_left` | same |
| `split_join_tokens` | absent or `always` | same compatibility input; semantic caps remain gated above |
| `highlight_fields` | same field grammar as `query_by` | same |
| `include_fields` | absent or the exact active projection string | same until response v2 is versioned |
| `use_cache`, `cache_ttl` | accepted by service layer; cache policy rather than membership | same; must enter request/cache identity where observable |

Compound filters, unknown filter fields, malformed backticks, empty filter values, and unsupported sorts fail closed. `youtuber_handle` comparison is ASCII-case-insensitive; `video_id` is exact.

The six explicit `sort_by` expressions are:

```text
published_at_timestamp:desc
published_at_timestamp:asc
view_count:desc
like_count:desc
comment_count:desc
engagement_score:desc
```

Absence means relevant. Together these are the seven user-visible sorts.

## Legacy match membership and provenance

Legacy matches terms against `text`, `title`, or both. A title posting expands to every document belonging to that video so the existing document-shaped response can be emitted. This expansion must not be interpreted as transcript provenance.

For each admitted candidate, legacy carries:

- a `matches_text` bit;
- a four-bit exact mask (only the first four canonical query positions are distinguishable);
- an `all_retained_exact` bit;
- an `ordered_phrase` bit;
- one `f32` score.

Alternatives for the same query term are unioned. Distinct alternative contributions that hit the same document are added. Distinct term plans are intersected and their scores are added. Text and title contributions for the same alternative are also added. Exact masks are ORed. This observed sum behavior is why the API accepts only `text_match_type=sum_score`.

The four-bit mask is insufficient for arbitrary query length. It is specifically not a v2 evidence representation.

### Schema-9 v2 match set

The v2 result must retain:

```text
candidates: exact DocSet
transcript_matches: exact DocSet, subset of candidates
title_matches: exact VideoSet
matched_videos: exact VideoSet
mode: Query | Wildcard
evidence: per-candidate canonical contribution identities and phrase/proximity evidence
```

Mixed-field conjunction provenance is derived from the branches that actually admitted the final candidate after phrase, filter, and negation predicates. It is forbidden to reconstruct provenance by independently unioning field hits after the predicate is evaluated.

Title-only videos contribute one visible moment. Expanded title documents do not each create a moment.

## Legacy score formulas

All values below are `f32` in the implementation.

### Document frequency and IDF

```text
effective_title_documents = title_video_frequency * floor(total_documents / total_videos)
effective_frequency = min(total_documents,
                          text_document_frequency + effective_title_documents)
idf = ln(1 + (N - effective_frequency + 0.5) /
             (effective_frequency + 0.5))
```

Targeted/planned paths estimate each enabled field then sum them, producing the same intended effective-frequency model. P00 does not yet have bit-golden evidence proving all legacy execution paths are identical at every overflow/boundary input.

### Text contribution

Constants: `k1=1.2`, `b=0.75`, text field weight `4.0`.

```text
length_ratio = document_tokens / average_document_tokens
              (or 1.0 when average is zero)
bm25_tf = tf * (k1 + 1) /
          (tf + k1 * (1 - b + b * length_ratio))
position_multiplier = 1 + 0.15 / sqrt(first_token_position + 1)
text = bm25_tf * idf * 4.0 * position_multiplier * expansion_multiplier
```

Position multiplier is `1.0` when disabled internally. The public validator rejects `prioritize_token_position=false`, so ordinary public legacy requests enable it.

### Title contribution

Title field weight is `6.0`.

```text
title = idf * 6.0 * (1 + ln(title_tf))
        * position_multiplier * expansion_multiplier
```

### Expansion and split contributions

| Branch | Legacy multiplier/addition |
| --- | ---: |
| exact | `1.0` |
| prefix | `0.9` |
| edit distance 1 | `0.65` |
| edit distance 2 | `0.4` |
| exact missing-separator split | two component scores are added, then `8.0` is added |

The fuzzy-split branch has additional path-specific phrase preference in `single_token.rs`. Its complete reduction order and overlap behavior do not yet have a bit-exact golden fixture; see QS-02.

### Unquoted proximity

For two or more terms, legacy may compute a minimum window within text and title separately:

```text
minimum_displacement = token_count - 1
extra_gap = displacement - minimum_displacement, saturating at zero
if displacement > 10: multiplier = 1
otherwise:
  proximity = 0.20 / sqrt(extra_gap + 1)
  early_position = 0.05 / sqrt(max_offset + 1)
  multiplier = 1 + proximity + early_position
```

If either field supplies an ordered adjacent pair through the bigram path, legacy multiplies the document score by the larger field proximity multiplier and then by `1.25`, and marks ordered phrase true.

Legacy proximity is path-sensitive:

- at most 128 matches use direct document inspection;
- at 129 through 50,000 matches it may use bigram or full position postings;
- above 50,000 matches proximity is skipped;
- more than 10 query terms skip exact proximity;
- missing full-position structures skip the boost.

These are observed semantic discontinuities. V2 forbids every one: complete text score is independent of cardinality and format availability. P01 must select sufficient structural summaries or lazy position reads.

### Engagement

Generation maxima are used, not result-set maxima:

```text
norm(x, max) = 0 when max = 0
             = ln(1 + x) / ln(1 + max) otherwise

normalized_engagement =
    0.50 * norm(views, max_views)
  + 0.35 * norm(likes, max_likes)
  + 0.15 * norm(comments, max_comments)

blended_relevance = raw_relevance
  + max(raw_relevance, 1.0) * 0.20 * normalized_engagement
```

## Canonical v2 score reduction

The following is frozen unless explicitly marked otherwise:

1. Normalize and assign positive query slots in first-appearance order. Repeated unquoted terms share one slot. Phrase occurrences retain their own positional occurrence semantics.
2. Enumerate expansion branches by canonical indexed-term UTF-8 bytes, then by expansion kind in this order: `exact`, `prefix`, `split_exact`, `split_fuzzy`, `typo_1`, `typo_2`.
3. Within one alternative, evaluate `text` before `title`.
4. Compute each field contribution using the formulas above and apply its expansion multiplier as the final multiplication of that field contribution.
5. Deduplicate identical canonical contribution identities. Contributions with different identities remain additive even when they reach the same document or indexed term through different semantic branches.
6. Add field/alternative contributions in that canonical order using binary32 operations. Do not reassociate and do not use an FMA where it changes reference bits.
7. Add split bonus after both component reductions for that split branch.
8. Add distinct query-slot totals in query-slot order.
9. Apply the chosen document-level proximity/ordered factor after the additive raw term score.
10. Reject nonfinite intermediate or final values as corrupt index/source data; they are never sortable keys.

V2 uses `f32::total_cmp` only after finiteness is proved. Rank bounds must round outward to conservative representable values.

| Reduction question | Status |
| --- | --- |
| distinct text and title contributions for one identity | additive, **FROZEN** |
| distinct exact/prefix/typo identities that hit one document | additive after identity deduplication, **FROZEN** |
| identical identity reached twice | count once, **FROZEN** |
| exact split bonus placement | after its two components, **FROZEN** |
| overlapping split hypotheses and whole-token hypotheses | **DECIDED QS-01; EXECUTABLE GATE PENDING:** each whole-token or split parse is a mutually exclusive query-slot hypothesis; fully score each and select by finite score descending, then exact-whole, prefix-whole, split-exact, split-fuzzy, typo-1, typo-2 priority, then canonical bytes. Never add competing parses. |
| fuzzy-split phrase/proximity multiplier placement | **DECIDED QS-02; EXECUTABLE GATE PENDING:** per document, reduce canonical field contributions and corrected-component multipliers, add left then right, add binary32 `4.0`, multiply by maximum text/title proximity, then multiply by `1.25` when adjacent. Phrase evidence boosts locally and never removes loose matches elsewhere. |
| multiple phrase multipliers in one query | **DECIDED QS-03; EXECUTABLE GATE PENDING:** compute each required phrase factor, choose the maximum in canonical phrase order, and multiply once. Ordered preference is OR. Never multiply phrase factors together. |
| title and text unquoted proximity when both match | **DECIDED QS-04; EXECUTABLE GATE PENDING:** use the maximum finite field factor, then evaluate `(raw * field_max) * ordered_factor` as two binary32 multiplications without FMA. |

QS-01 through QS-04 have reviewed policy examples and score-bit fixtures in `engine/tests/query_contract_v2.rs`; those fixtures are scaffolding, not proof of runtime equivalence. An optimizer may not substitute mathematically equivalent reassociation once the executable reference is added.

## Ranking contracts

### Legacy observed video key

The best video rank tuple is copied from one actual best document. It is never assembled from componentwise maxima across documents.

Default relevant:

```text
exact token count descending
four-bit exact mask descending
ordered-phrase boolean descending
raw f32 score descending by total_cmp
external video ID ascending
```

Relevant plus engagement:

```text
exact token count descending
four-bit exact mask descending
ordered-phrase boolean descending
blended relevance descending by total_cmp
raw f32 score descending by total_cmp
external video ID ascending
```

Newest/Oldest/Most viewed/Most liked/Most discussed:

```text
named video numeric key in requested direction
raw f32 score descending by total_cmp
external video ID ascending
```

Within a group, retained documents use exact-token count, exact mask, ordered phrase, raw score, then generation-local document ID. This final dense-ID tie break is observed legacy behavior and is not accepted for stable v2 ordering.

### Schema-9 v2 video keys

The default key is:

```text
exact distinct query-token coverage descending
exact token-evidence vector in canonical query order, lexicographically descending
ordered-phrase preference descending
complete text score descending
normalized generation-wide engagement descending
external video ID ascending
```

`exact token-evidence vector` has one element per positive query slot and is not truncated to four bits. The group key is the complete tuple of the single best real matching document under this order. Engagement is not borrowed from another document; it is video metadata and therefore identical for all documents in the group.

Relevant plus engagement:

```text
coverage descending
evidence vector descending
ordered phrase descending
blended relevance descending
complete raw text score descending
external video ID ascending
```

Newest and Oldest compare publication timestamp descending/ascending, then complete raw text score descending, then external video ID ascending.

Most viewed, Most liked, and Most discussed compare their named value descending, then complete raw text score descending, then external video ID ascending.

Inside every video and for every video-level sort:

```text
the frozen v2 document relevance key
timestamp ascending
external chunk ID ascending
```

Unknown/missing numeric metadata is represented by an explicit missing bit and numeric zero in fast fields. **DECIDED QR-01; EXECUTABLE GATE PENDING:** for each named descending numeric sort, every present value including real zero sorts before missing. Missing contributes zero only to relevance arithmetic and remains distinct in the sort key.

**DECIDED QR-02; EXECUTABLE GATE PENDING:** an absent publication timestamp sorts last for both newest and oldest. Present signed timestamps compare in the requested direction, preserving pre-epoch, epoch-zero, positive, and equal values.

No generation-local dense identifier is a stable v2 tie breaker.

## Exact totals and moments

Legacy grouped `found` is matched videos; ungrouped `found` is matched documents. `out_of` is total indexed transcript documents. `total_moments` is the sum of per-video visible moments after filtering.

For query mode, transcript timestamps are processed in nondecreasing order with stable document tie order. The reducer is greedy and anchored:

1. Start a cluster at the first selected transcript timestamp.
2. Start another cluster only when `timestamp - current_anchor > 20_000` milliseconds.
3. When starting a cluster, replace the anchor with that timestamp.
4. A matched video contributes at least one moment, including a title-only match.

This is not a rolling gap and not a fixed bucket. For example, `0, 19_000, 38_000` yields two moments because `38_000 - 0 > 20_000`.

Wildcard mode contributes exactly one visible moment per matched video. Title-expanded documents never inflate the moment count.

Schema-9 v2 freezes these units for the compatibility response:

| Field | Grouped request | Ungrouped request |
| --- | --- | --- |
| `found` | exact matched videos | exact matched documents |
| `total_moments` | exact anchored visible moments | exact anchored visible moments |
| `out_of` | generation transcript-document count | generation transcript-document count |

New response fields may expose explicit `total_videos` and `total_documents`; they do not reinterpret legacy fields.

## P00 freeze ledger

The policy choice for every row is recorded above. P00 is not semantically complete while its executable golden evidence remains open.

| Gate | Missing evidence or decision | Autonomous repair instruction | Passing evidence |
| --- | --- | --- | --- |
| QG-01 | punctuation-only meaning | Add parser fixtures for `---`, emoji-only, and punctuation-plus-negation; judge reject versus wildcard explicitly | approved table row plus exact error/result fixtures |
| QG-02 | Unicode normalization | Build NFC/NFD, Turkish-I, combining-mark, full-width, and non-Latin fixtures against indexed/query tokens; choose a versioned normalization policy | normalized-byte goldens and differential results |
| QG-03 | numeric typo/prefix semantics | Test IDs, years, decimals, and long digit runs against current fuzzy index; judge numeric expansion policy | field-specific membership goldens |
| QG-04 | more than four equal-distance fuzzy terms | Construct a lexicon with at least six tied candidates; record candidate ordering and desired recall | complete deterministic alternative set |
| QG-05 | split budget semantics | Construct a valid compound whose components exceed one-million posting work; compare exhaustive split oracle | exact membership judgment independent of execution budget |
| QG-06 | long token-drop semantics | Exercise 21, 22, and 32 distinct terms with matches only beyond each legacy cap | total arbitrary-length rule and goldens |
| QG-07 | per-field prefix | Index candidates present in text-only, title-only, and both fields | exact per-field prefix matrix |
| QV-01 | zero-sized windows | Exercise grouped/ungrouped `per_page=0` and `limit=0`; judge compatibility versus reject | response/error fixtures |
| QS-01 | overlapping branch reduction | Build a document reachable through exact, prefix, split, and typo hypotheses | approved contribution identities and exact bits |
| QS-02 | fuzzy-split reduction | Trace every multiplication/addition in the branch and capture a multi-field fixture | operation-order table and exact bits |
| QS-03 | multiple phrase score factors | Build two positive phrases matching one document at different offsets | approved combination rule and exact bits |
| QS-04 | both-field proximity | Match the same tokens in title and text with different windows | approved selection rule and exact bits |
| QR-01 | missing engagement values | Preserve source presence bits in a fixture and compare missing against real zero | all five numeric sort goldens |
| QR-02 | missing publication time | Fixture missing, epoch zero, pre-epoch, and equal timestamps | newest/oldest goldens |

For each failed gate, the executor must:

1. add the smallest adversarial fixture that makes the ambiguity observable;
2. run the current implementation and an independent exhaustive reference;
3. record whether current behavior is stable, accidental, cardinality-dependent, or corrupt;
4. propose one explicit v2 rule without weakening exactness;
5. obtain the required independent review;
6. update this table and add exact expected membership/order/score bits;
7. rerun all previously frozen rows affected by the decision.

Do not mark a gate resolved merely because one current path returned an answer.

## Required invariant tests

The implementation must eventually prove all of these for every frozen row:

- exhaustive and lazy membership sets are identical;
- bitmap/no-bitmap and skip/linear paths produce identical membership;
- scoring is bit-identical under permuted execution order because reduction first restores canonical order;
- candidate cardinality at 127/128/129 and 49,999/50,000/50,001 does not change v2 score;
- query lengths 4/5 and 10/11 do not lose evidence or proximity;
- title-only matches yield one moment and no false transcript highlight;
- all pages concatenate to the exhaustive sorted video list without duplicates or omissions;
- all seven sorts use the frozen tie breakers;
- finite-score validation rejects NaN and infinities before comparison;
- filters and negations change the exact set before totals, summary, scoring, and highlighting;
- changing generation changes query identity; one session never mixes generations.

## Freeze verdict

The legacy behavior and its constants are documented from source. Independent grammar and scoring reviews selected explicit v2 rules for the core invariants, formulas, seven sort shapes, provenance model, moment units, normalization, expansions, parse hypotheses, and missing values. Those choices are no longer implementer discretion.

The contract is **decided but not yet evidence-frozen** because QG-01 through QG-07, QV-01, QS-01 through QS-04, and QR-01 through QR-02 still lack their executable adversarial and exact-bit goldens. Therefore the honest P00 verdict remains **failed gate, actionable**. Continue only with the repair instructions in the freeze ledger; do not allow P01 representation measurements or P06 ranking implementation to change the decisions implicitly.
