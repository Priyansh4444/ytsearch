# Raw request validation contract

Verified 2026-08-08. Raw rejects request shapes it cannot execute faithfully;
it does not reinterpret them as a broader query or silently remove a filter,
sort, facet, grouping rule, projection, or cache constraint.

## HTTP envelope

`SearchParams` uses Serde's `deny_unknown_fields`, so unknown parameters and
unimplemented Typesense features such as `facet_by` and `infix` return HTTP 400
before backend execution. Collection names are also exact; an unknown
collection returns 404.

Cache entries are bounded to 128 responses. `cache_ttl` now requires
`use_cache=true` and an explicit value from 1 through 300 seconds. Values are
rejected instead of being silently clamped, and validation runs before cache
lookup so a malformed request cannot hit a prior entry.

## Transcript collection

The accepted contract is deliberately narrow:

- `query_by` contains each of `text` and `title` at most once;
- filters are one exact `:=` comparison on `video_id` or
  `youtuber_handle`;
- compound expressions, lists, ranges, repeated operators, and malformed
  backticks are rejected;
- sort expressions must be one of the six implemented publication or
  engagement sorts;
- grouping is either absent or exactly `video_id`;
- pagination cannot mix page/per-page with limit/offset, and offset requires a
  limit;
- Typesense compatibility knobs are accepted only at the values Raw actually
  implements; unsupported values return 400;
- highlight fields and the document projection are checked against the
  implemented field set.

Page zero and `per_page=0` remain valid because Typesense itself treats page
zero as its default first page and permits zero-hit result windows. Page-size
values above 250 fail closed.

## Channel collection

Channel search now returns backend errors through the shared service contract
instead of an infallible JSON value. It accepts `q`, `query_by=name,handle`, one
pagination style, and the shared bounded-cache controls. Transcript-only
filters, grouping, scoring, typo, phrase, sort, highlight, and projection
parameters return HTTP 400. The Raw contract script now sends the channel field
set explicitly rather than relying on ignored transcript parameters.

## Evidence

Unit coverage includes unknown query keys, `facet_by`, `infix`, cache TTL zero
and overflow, TTL without caching, unknown and duplicate fields, unsupported
filters/sorts/groups, compound/list/repeated filter operators, malformed
backticks, valid quoted exact filters, conflicting pagination, empty queries,
and transcript-only parameters on channel search.

The workspace test and Clippy gates exercise both `youtube_search_service` and
`youtube_raw_search`. This closes fail-closed validation only for the active
application contract; arbitrary Typesense filter/facet syntax remains
deliberately unsupported.
