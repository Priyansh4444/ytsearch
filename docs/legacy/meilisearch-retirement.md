# Meilisearch Retirement Record

This record was committed before any Meilisearch Fly resource was destroyed.

- Recorded: 2026-07-11
- Engine app: `ytsearch-meili-staging`
- Engine machine: `2862d23f624098` (`shared-cpu-2x`, 4096 MB, SJC)
- Engine volume: `vol_vxm6pxw08endwz84`, `meili_data`, 10 GB
- API app: `ytsearch-api-meili-staging`
- API machine: `e826333f254408` (`shared-cpu-1x`, 512 MB, SJC)
- Frontend: `meili.ytsearch-755.pages.dev`

## Why It Was Retired

The search API used raw transcript hits and then made additional backend-page
requests to collect distinct video cards. Meilisearch returned many chunks from
the same video for broad queries such as `react`, so this grouping fan-out became
the dominant latency cost. During the final controlled sample its engine process
was about 2.8 GB RSS on a 4 GB VM. Typesense was about 1.9 GB RSS on the same VM.

The latency comparison was not used as a standalone ranking decision because the
backends had different indexed coverage and Typesense totals were still changing
while accepted writes drained. The retirement decision was operational: keeping
two full indexes was too expensive while Typesense was the selected active path.

This is a historical record only. Do not restore the volume or infer that an
export exists. The retirement is intentionally destructive; the volume contents
are deleted after this commit.
