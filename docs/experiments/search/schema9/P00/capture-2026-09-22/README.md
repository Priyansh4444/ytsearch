# Read-only production inventory capture

Source: `e5280936d9683ca7575298c6b7cd583f3af702dc`, 2026-09-22
22:57:56–22:58:02 UTC. The collector used the existing deployed admin key in
memory; no credential is retained in these artifacts. No machine, index, queue,
or deployment was changed.

[`receipt.json`](receipt.json) records seven successful probes, zero failures,
generation 2950, storage schema 6, 8,422,419 documents, 35,215 videos, and
61 channels. The generation pointer and its manifest—not the immutable base
index—supply the corpus counts. The manifest's raw SHA-256 is
`b97c8727ccabdbb5a45ac44010f337633849bf527e8d8a516f458d403411d7b5`.
The retained readiness, pointer, manifest, and availability responses permit
direct inspection of those claims. Fly's raw app/machine responses were not
committed because one embeds executable script and creator-list contents plus
private operational metadata. [`machine-sources.json`](machine-sources.json)
retains their SHA-256 digests and derived counts; the raw originals remain in
the temporary capture directory for this audit but are not durable repository
evidence. Recheck machine cardinality before the full P00 verdict.

This is a passing **inventory capture**, not a passing P00 program gate. The
reader still runs schema 6; observed grammar/rank versions are unknown, while
the target contract versions are 2. The 300-case matrix was structurally
validated, and the v2 contract file was present; neither proves production
query equivalence. The old public performance lane failed, and no new load or
latency run was performed here. Independent final P00 review remains pending.
