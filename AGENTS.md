# ytsearch

Rust ingestion/API and Raw index, Solid frontend, Cloudflare Worker.
`raw` is the only backend. [Architecture](docs/architecture.md) owns boundaries;
[production inventory](config/fly-production-resources.json) owns resource identities.

## Doctrine

- **Monomorphism:** prefer concrete domain types and consistent record shapes.
  Avoid speculative generics, dynamic registries and parallel abstractions.
- **Purity:** deterministic domain functions, readonly inputs, explicit outputs.
  Isolate I/O, clocks, randomness and mutation at effect boundaries.
- **Totality:** validate external data once; represent absence and failure explicitly;
  use exhaustive variants instead of contradictory flags or unchecked casts.
- **Encapsulation:** extend `SearchBackend` and `youtube_ingest` contracts before
  introducing alternatives. Preserve the project's existing stack.
- **Proof obligations:** test observable contracts with independent fixtures,
  differential checks or algebraic properties. A test must detect a plausible
  defect; copying the implementation into its expected result proves nothing.

## Invariants

- Best-match ranking; low-signal `I` uses newest-for-filter ordering. Stopwords
  must not dominate snippets. Group timestamp hits by video; sort in the API.
- Prefer `update`; `rewipe` needs explicit acknowledgement. Preserve chunks and
  creator counts. One queue consumer and one publisher; `JOBS=1 CHANNEL_JOBS=1`.
  Retain retryable work without blocking later requests.
- Public API uses search-only credentials; keep indexing/admin logic outside it.
  Never print secrets or expose them through `VITE_*`. Use existing env loaders;
  append to `.env`, never rewrite it.
- Preserve unrelated edits. Fix recurring mistakes in types, tests or scripts;
  record operational discoveries in the relevant playbook.

## Task routing

Read only the relevant material. Use the available `workflow` skill for substantial work.

| Task | Read |
| --- | --- |
| Live operations, indexing, deploys, session recovery | [Operations skill](.agents/skills/ytsearch-operations/SKILL.md) |
| Search behavior or relevance | [Search contract](docs/playbooks/search-quality.md) |
| Optional semantic pilot and experimental site | [Semantic pilot](docs/playbooks/semantic-pilot.md) |
| Dependencies, frontend, API, performance | [Playbook index](docs/playbooks/README.md) |

## Verification

Run `bun run check:precommit` and `bun run check` before declaring code complete.
Run the task's live or differential checks where applicable. Report implementation,
deployment and background repair separately; compilation cannot prove production recovery.
