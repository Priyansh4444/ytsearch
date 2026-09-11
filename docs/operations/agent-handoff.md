# Resume Raw migration work

Start with `TODO.md`. Treat unchecked items as incomplete. Read
[schema-8 validation](raw-schema8-validation.md) for experiment evidence and
[production cost](raw-production-cost.md) for the budget assumptions.

Use the existing diagnostic commands instead of reconstructing Fly queries:

| Question | Command |
| --- | --- |
| Is search serving? | `rtk scripts/doctor.sh` |
| Is the queue worker configured correctly, and what failed recently? | `rtk scripts/doctor.sh --indexer` |
| Do all local release checks pass? | `rtk scripts/check-all.sh` |

For long commands, retain the full log in an ignored results directory and
return the exit status with a short failure excerpt. Do not repeatedly print
machine configurations, build logs, token values, or whole source files.
Poll the same live job after a timeout. Do not restart it because a log is quiet.

## Daily indexing

Cloudflare's hourly cron sends a wake message. The worker's persisted daily
timer owns the refresh. The reconciled configuration enables a 24-hour interval
with no first-run delay. A successful pass discovers recent uploads for the
62-creator list in one `index-youtubers` process, defers publication across
creators, and publishes once. One process also gives every discovery manifest
the same catalog-run ID required by the reconciliation checker.

`/var/lib/ytsearch/daily-update.last-completed` records successful completion.
A running Machine, enabled timer, or successful wake is not completion proof.
Verify the timestamp advances only after publication succeeds, then verify a
new video's transcript is searchable. Missing creator lists fail the pass.

On 2026-09-10, live inspection found daily refresh disabled, pending publication
retries, and an IPv4-only production reader bind. The worker could not reach
`/admin/readiness` over Fly's IPv6 private network. The local configuration fix
is not evidence of a repaired deployment.

## Release boundaries

Stable resource IDs live in `config/fly-production-resources.json`. Use the
staging and reconciliation scripts that validate those IDs and acquire leases.
Do not broaden a command to all Machines or delete retained trial artifacts.

Production promotion still requires the full-snapshot publication and strict
sub-200 ms correctness/workload gates. Keep Typesense resources until the final
retirement gates pass. Preserve unrelated user edits, including deleted docs.

For the one-time schema build, stage the existing stopped builder with
`RAW_PUBLICATION_BUILDER_PROFILE=performance-1x`. The profile keeps memory at
2 GB, raises storage throughput through Fly's performance CPU class, and gives
the build 10,800 seconds. The canary lease becomes 43,200 seconds so both
publication processes and cleanup fit. The scripts stop the builder after each
process. Do not resize the always-on reader.

Fly's scoped token may need renewal after 24 hours. Follow `docs/deploy.md`;
do not use deprecated `flyctl auth token`. Private credential files must remain
ignored and must never appear in handoff text or command output.
