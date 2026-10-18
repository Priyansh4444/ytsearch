---
name: ytsearch-operations
description: Resume or operate ytsearch indexing, queue repair, transcription fallback and production deployments. Use for live maintenance, not ordinary frontend or domain refactors.
---

# ytsearch operations

Work from the existing session's authorization; this skill does not authorize
additional production writes.

## Establish state

Read the inherited handoff and `git status` before repeating work. Record exact
refs, deployed image digests, queue state and active publication. Use
[production inventory](../../../config/fly-production-resources.json) and
`scripts/agent-status.sh`; verify Fly state before and after mutations.

Keep long-running scripts, logs and resume notes in a durable task directory:
`/tmp` can disappear between sessions. Keep credentials out of handoffs.
An active publisher owns the writer; do not interrupt it to redeploy or reconcile.

## Select the operation

| Operation | Authoritative procedure |
| --- | --- |
| Queue, creator discovery, classification, ASR | [Indexing guide](../../../docs/indexing-operations-guide.md), [gap diagnostics](../../../docs/playbooks/indexing-gaps.md) |
| Deploys, immutable images, credentials | [Deploy guide](../../../docs/deploy.md) |
| Publication, space and resource limits | [Release procedure](../../../docs/operations/raw-publication-release.md), [capacity](../../../docs/capacity.md), `scripts/check-fly-cost-config.sh` |
| Production health | [Observability](../../../docs/staging-observability.md) |

## Recovery obligations

- Keep the canonical queue entrypoint and one consumer. A stopped on-demand
  worker can be healthy; inspect persisted schedules before waking or resetting it.
- Discovery must include videos, streams and Shorts. Join IDs against a stable
  published catalog; aggregate count differences and historical failure totals
  cannot establish missing videos.
- Keep the shared ASR budget, resumable checkpoints and portable final caches.
  Classify access restrictions and no speech; never erase failures to imply recovery.
- Retain publication's disk reserve beside transcript/ASR caches. Diagnose actual
  space use before changing storage; keep measured resource/cadence limits.
- Verify recovery through publication and public search. Use
  `scripts/check-raw-creator-reconciliation.sh` after a complete published sweep;
  mixed or stale manifests are not proof. Record unfinished work and its next check.
- Use `bunx wrangler`; consult current Wrangler docs before Worker config changes.
  For classic Fly VMs, `flyctl ssh console --no-container --machine <id>` avoids
  container-selection failures. Read selected state, never environment dumps.
