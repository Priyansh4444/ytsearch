# Tantivy search trial (retired)

`youtube_tantivy` was the first read-only alternative backend. It stored source
documents and indexed text in Tantivy, then implemented the same contract used
by `youtube_api`.

The trial was retired on 2026-08-01. Its only Fly machine
(`839741f7620768`) was destroyed and the active workspace no longer contains or
builds the crate. The complete source, manifests, deployment configuration,
harnesses, and reports are preserved on branch `archive/tantivy-experiment` at
commit `3aa3f55ffbf46df2fd107f1aed0f1274b7c782c7`.

The crate was useful as a maintained-library control for the custom raw format.
Its guarded strict, leading-phrase, exact, and fuzzy stages established
the relevance behavior now shared by both experimental engines.

## Historical build and serve

```sh
cargo run --release -p youtube_tantivy -- build \
  .tantivy-experiment/corpus/chunks.jsonl \
  .tantivy-experiment/corpus/channels.jsonl \
  .tantivy-experiment/index

TANTIVY_BIND=127.0.0.1:8108 \
cargo run --release -p youtube_tantivy -- serve \
  .tantivy-experiment/index
```

## Current interpretation

Tantivy remains much smaller and faster than the matched Typesense control, but
Raw is smaller and uses less query CPU on this workload. Tantivy builds the full
sample faster and carries substantially more upstream correctness,
internationalization, query-language, and index-maintenance capability.

See [common-results.md](common-results.md) for retained measurements and
[methodology.md](methodology.md) for the corrected scaling corpus.

The Fly app name remains as a historical record, but it has no image and no
machines. There is no live Tantivy endpoint.
