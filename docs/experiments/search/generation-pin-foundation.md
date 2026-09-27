# Generation pinning: existing-reader experiment

Status: local foundation verified; not a session implementation or a production
release gate. No generation was deployed, deleted, or activated in production.

## Decision and observed behavior

Reuse `ReloadableEngine::snapshot()` to acquire an immutable reader and its
activation sequence together. It already returns an `Arc<RawSearchEngine>` and
sequence while holding the same read lock; replacement updates both under the
matching write lock. A separate generation manager is not needed merely to
retain a reader across activation.

The local fixture builds real indexes containing three old videos and two new
videos. The same query matches both generations. After replacement:

- The pinned reader retains all three distinct old document pages and exact
  `found = 3`; new requests see the new documents and `found = 2`.
- The pinned sequence remains 41 while the active sequence advances to 42.
- A weak reference confirms the retired reader survives while leased and is
  released after its last strong reference drops.
- A 1,000-snapshot race smoke test observes only reader/sequence pairs with
  matching document counts. Scheduling does not guarantee overlap; this is
  supporting stress coverage, not a deterministic proof of all interleavings.

Run the focused checks:

```bash
cargo test -p youtube_raw_search reloadable::tests
```

Independent review confirmed the lock discipline and lease test scope. The
three focused tests pass. No performance budget is claimed for this tiny fixture.

## Still required

The public search API currently reacquires the active reader per request. It
does not expose a pinned session, authenticated cursor, exact whole-set overview,
or a no-rescan rank frontier. Separate `search()` and `cache_generation()` calls
are not an atomic snapshot and must not be used to bind future cursor claims.

A future session must own this lease and reuse it for every page, summary, and
video-moment request. TTL/admission, explicit expiry on restart, refresh notices,
filesystem cleanup protection, multiple retained-generation memory cost, and
2-GB-reader resource measurements remain unverified. `Arc` lifetime alone does
not prevent publisher cleanup from unlinking generation files.
