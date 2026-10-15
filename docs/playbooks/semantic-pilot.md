# Local Qdrant and EmbeddingGemma pilot

## Fast path

This is an explicitly imported, bounded local snapshot experiment. It does not
subscribe to production ingestion, synchronize deletes, deploy services, or alter
Raw. Lexical search remains the default. No hosted inference or model download
exists in the binary.

Prerequisites: Rust, Python 3.12+ for setup, and Linux x86_64 for the pinned
native binaries. Review the upstream [Gemma terms](https://ai.google.dev/gemma/terms)
and [model card](https://huggingface.co/google/embeddinggemma-300m) before downloading.
The setup script verifies every asset checksum. The Rust binary performs no downloads.
ONNX Runtime 1.31.0 is exercised with `ort` rc.14 API 24.

```bash
# Downloads about 260 MB once; verified assets are reused on reruns.
python scripts/prepare-semantic-assets.py
# Run Qdrant in a separate terminal from the repository root:
data/semantic/qdrant/qdrant --config-path config/qdrant-semantic.local.yaml

# In another terminal:
cargo build --locked -p youtube_semantic -p youtube_api
cargo test -p youtube_semantic -p youtube_api
# Starts isolated disposable services; requires the build and assets above.
python scripts/test-semantic-pilot.py

export SEMANTIC_MODEL_DIR="$PWD/data/semantic/model"
export ORT_DYLIB_PATH="$PWD/data/semantic/runtime/onnxruntime-linux-x64-1.31.0/lib/libonnxruntime.so.1.31.0"
export SEMANTIC_QDRANT_URL=http://127.0.0.1:16333
export SEMANTIC_COLLECTION=ytsearch_semantic_pilot_local_128
export SEMANTIC_DIMENSIONS=128
```

The required local model directory contains `onnx/model_q4.onnx`, its external
`onnx/model_q4.onnx_data`, `tokenizer.json`, `tokenizer_config.json`, `config.json`,
and `special_tokens_map.json`. All six SHA256 hashes are verified at startup.
They lock the repository `onnx-community/embeddinggemma-300m-ONNX` at revision
`5090578d9565bb06545b4552f76e6bc2c93e4a66`. See the exact asset manifest in
[domain.rs](../../crates/youtube_semantic/src/domain.rs).

Prepare a JSONL snapshot of **complete** videos. Each line is an existing chunk
document with `id`, `video_id`, `title`, `text`, `youtuber_handle`,
`youtuber_name`, `t_start_ms`, and `t_end_ms`; ordinary extra chunk metadata is
accepted. Sort videos lexicographically by `video_id` and each video's chunks by
timestamp. ASCII `@handles` are canonicalized to lowercase for both import and filters. Chunks must be
nonempty, ordered by start timestamp, at most 60 seconds each. Source caption
ranges may overlap; those timestamps are preserved. Optional display
metadata must agree across one video's chunks. `sequence`, `prev_text`, chapter
and other chunk context may differ.

The exporter must finish every video before writing a companion receipt named
`SNAPSHOT.jsonl.manifest.json`. The receipt declares complete video chunk counts
and hashes the exact JSONL bytes. A valid JSONL prefix cannot establish snapshot
completeness by itself. This example finalizes a **known complete** local export;
it does not determine whether a source export contains all chunks:

```bash
python - /tmp/snapshot.jsonl <<'PY'
import collections, hashlib, json, pathlib, sys
path = pathlib.Path(sys.argv[1])
counts = collections.Counter()
hasher = hashlib.sha256()
with path.open('rb') as source:
    for line in source:
        hasher.update(line)
        counts[json.loads(line)['video_id']] += 1
receipt = {'schema': 1, 'sha256': hasher.hexdigest(), 'complete_videos': dict(counts)}
path.with_name(path.name + '.manifest.json').write_text(json.dumps(receipt))
PY

# Optional Qdrant authentication is read from SEMANTIC_QDRANT_WRITE_KEY.
# Supply it through the existing environment mechanism; never command arguments.
target/debug/youtube-semantic index /tmp/snapshot.jsonl
# A repeat reports embedded_passages=0 for unchanged documents.
target/debug/youtube-semantic index /tmp/snapshot.jsonl

# Serve reads only SEMANTIC_QDRANT_READ_KEY, never the importer's write key.
# With Qdrant auth enabled, provision a genuinely read-only key for this role.
SEMANTIC_BIND=127.0.0.1:8790 target/debug/youtube-semantic serve
```

Use a separate terminal for read probes:

```bash
curl -fsS http://127.0.0.1:8790/ready
curl -fsS http://127.0.0.1:8790/search \
  -H 'Content-Type: application/json' \
  -d '{"q":"Why can light not escape a collapsed star?","filter":{"creator":"@space"},"limit":10}'
# filter also supports video_id, and creator/video conditions are ANDed in Qdrant.
```

`/` describes the service; `/health` is liveness; `/ready` checks the loaded
worker and collection profile. Startup performs one model inference before
serving. The HTTP listener refuses non-loopback bindings. There is no CORS UI or
write endpoint.

For an existing locally configured Raw API, set only
`SEMANTIC_SEARCH_URL=http://127.0.0.1:8790` before starting `youtube-api`, then:

```bash
curl -fsS 'http://127.0.0.1:8787/api/search?q=collapsed%20star&mode=semantic&youtuber=%40space'
```

The optional API adapter holds no Qdrant credentials and adds no inference
packages to `youtube_api`. Without that environment variable, normal API startup
and lexical queries need neither model assets nor Qdrant; explicit semantic
relevance requests return a sanitized 503. Unknown modes fail deserialization.
No frontend control changes are included; its decoder merely accepts the new
response mode.

## Traps

- `mode=semantic` is honored only for relevance on a meaningful query. Explicit
  date/engagement sorts keep their lexical behavior. `I` still uses lexical
  newest-for-filter ordering. The response reports the **actual** mode.
- Semantic retrieval is a bounded top-100 passage candidate set. Existing API
  grouping and local pagination preserve vector order, with approximate totals.
  Deep pagination and exhaustive “all moments” are not supported by this pilot.
  Returning 100 passages does not mean there are only 100 matches in the corpus.
- Passages greedily concatenate original chunk text with newlines until either
  60 seconds or 512 **prompt-inclusive** tokens would be exceeded. Passage
  boundaries consume each source chunk once; ranges may overlap because YouTube
  caption ranges overlap. The end is the maximum original end of contributing chunks. Shorter
  passages are allowed. An individually oversized chunk fails; split it upstream
  with real timestamps. No text is silently truncated and no timestamps invented.
- Snapshots are capped at 64 MiB, 128 videos, 4 MiB / 4096 chunks per video and
  64 KiB per line. The importer freezes input in a temporary file and validates
  every video, token bound and receipt **before any collection mutation**.
- The receipt must come from a complete export. Recomputing it over accidentally
  incomplete video data asserts that incomplete data is authoritative. A pilot
  cannot infer absent source chunks from standalone JSONL documents.
- One local importer owns a filesystem lock keyed by Qdrant origin/collection.
  Use a single host and consistent origin spelling. Distributed writer leases
  are outside this pilot; never run importers from multiple hosts.
- A replacement writes missing content IDs with `wait=true`, checks completed
  acknowledgements, then removes captured obsolete IDs for that video. A failed
  replacement retains old passages. Retries skip successful points and converge;
  serving during import may temporarily see old and new passages. This is not
  transactional publication. Run explicit snapshot imports before serving for
  stable evaluations.
- Videos absent from a snapshot are untouched. An empty video replacement,
  explicit video deletion and `rewipe` are unsupported. A shortened nonempty
  complete video snapshot removes its obsolete passages after replacement.
- Do not reuse a collection for different dimensions, prompts, tokenizer,
  token cap, model or passage schema. Creation records the full semantic profile
  in Qdrant metadata; startup, readiness and searches refuse incompatible
  metadata, dimensions, distance or datatype. Choose a new pilot collection.
- Keys use distinct role-specific variables: `SEMANTIC_QDRANT_WRITE_KEY` for
  `index`, `SEMANTIC_QDRANT_READ_KEY` for `serve`. Omitting keys is for a local
  unauthenticated Qdrant only. The binary cannot validate the permissions of a
  supplied key; configure actual read-only scope in Qdrant.

## Facts and design rationale

The named shapes are `EmbeddingDimensions` (128 default or 256), `Profile`,
`Passage`, `SearchFilter`, `SnapshotReceipt`, and a content-based replacement
plan. UUIDv5 IDs include the passage content/profile and source revision. Each
passage carries a SHA256 text digest and a source revision digest of its original
contributing chunk documents. Display metadata never overwrites identity fields.

EmbeddingGemma's query prompt is `task: search result | query: {q}`; the document
prompt is `title: {title} | text: {text}`. The tokenizer inserts BOS. The Q4 graph
returns `sentence_embedding` as 768 float32 values, which are checked for shape
and finiteness, truncated to the chosen prefix and L2 normalized **after**
truncation. Qdrant stores cosine vectors as Float16 with vectors and payloads
on disk; inference activations remain float32. Q4 model weights and Float16 vector storage are independent.

Direct `ort` rc.14 loads the graph and external Q4 weights from local files.
Checksum verification streams through 64 KiB buffers. Tensor construction is
explicit: int64 IDs and attention mask, batch size one, named pooled float32
output. Dynamic runtime loading is explicit and never downloads a library.
Every reqwest dependency inherits the workspace's rustls configuration.

The FastEmbed prototype was rejected after a local loading comparison: it used
about 636 MiB steady model RSS versus 334 MiB for file-backed ORT, with the same
pinned graph and query. Its safer snapshot importer remains the final base. See
[the synthesis record](../semantic-pilot-synthesis-2026-10-15.md).
A Python sidecar would add a second deploy/runtime boundary. A Qdrant SDK duplicates the
existing reqwest transport for this small REST surface. Gemma inference in Raw,
`SearchBackend` or the public API would couple expensive work to lexical search.
Managed inference, larger models, turbopuffer, hybrid fusion and continuous
production ingestion are outside the requested cost and pilot scope.

There is one persistent inference worker, a one-item channel, one admitted search
request, batch size one, one ORT intra/inter-op thread and single-item
tokenization. Busy work returns 429. No blocking task is allocated per query. Inference waits time out after 30 seconds; the underlying CPU call
finishes on the single worker. Qdrant calls time out after 15 seconds; API service
calls after 30.
HTTP bodies and Qdrant responses have byte caps. Transport errors omit URLs,
credentials and backend bodies. Results hydrate original transcript text and
original timestamps through the existing video-card parser, without generated
highlights.

Focused tests cover dimension/profile mismatches, vector validation, passage
bounds and stable IDs, context metadata, typed in-retrieval filters, snapshot
truncation receipts, retry/shrink reconciliation, and an HTTP failure after one
successful upsert. Production defaults, explicit sorts and low-signal mode
routing have API tests. Live relevance and synchronization quality over a real
creator snapshot remain evaluation work; tiny fixture results are not a
production search-quality claim.

Local runtime evidence (2026-10-15, debug build, tiny two-video fixture): Qdrant
1.19.2 stored 128-dimensional cosine Float16 vectors. A collapsed-star query
scored astronomy passages 0.647 / 0.644 versus TypeScript 0.183; a type-error
query scored TypeScript 0.637 versus astronomy 0.254 / 0.242. Five service probes
took 42–66 ms embedding and 48–79 ms total after warmup. Creator and video filters
worked, an absent creator returned no hits, a repeat import embedded zero points,
and shrinking the astronomy video pruned its later passage without re-embedding
its unchanged first passage. A valid-line-boundary truncated file with its
original receipt was rejected. The actual API returned semantic video cards,
while `I`, explicit `published_asc`, and a default query issued lexical Raw
requests (the fallback transport was a local fixture, not production). All probe
processes were stopped afterward; temporary Qdrant storage stayed outside git.

The final real-caption trial and memory measurements are recorded in
[the report](../semantic-pilot-synthesis-2026-10-15.md) and
[its evidence](../semantic-pilot-evidence-2026-10-15.json).
