# Disaster Recovery

Last verified: 2026-07-15.

## Recovery Layers

Production uses two independent recovery layers:

1. Fly takes daily encrypted snapshots of the Typesense and queue volumes. Both
   critical volumes retain 30 days, and an on-demand snapshot was scheduled on
   2026-07-15.
2. `scripts/backup-typesense.sh` exports provider-neutral JSONL archives. These
   archives can rebuild the collections with `scripts/restore-typesense.sh`, so
   recovery does not depend on Fly's volume format.

Fly snapshots protect against a Machine or volume-host failure. JSONL exports
must be copied off Fly to protect against account, provider, or regional loss.

## Create and Verify an Export

Do not export the full collection from the serving Typesense process. A live
test on 2026-07-15 pushed a health-check search from 99 ms to 5.7 seconds and
was aborted. Restore the newest Fly snapshot into an isolated Typesense Machine
first, then point the export command at that instance.

```sh
TYPESENSE_BACKUP_API_URL=http://127.0.0.1:18108 \
TYPESENSE_BACKUP_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
BACKUP_ROOT=.search-experiment/backups \
scripts/backup-typesense.sh
cd backups/<timestamp>
sha256sum -c SHA256SUMS
```

The dedicated backup variables intentionally take precedence over production
defaults loaded from `.env`. The local URL must be a tunnel to an isolated
snapshot clone with no public service routes. The command refuses the known
serving Typesense URL by default and never writes the key into the archive.
The export contains `youtube_transcript_chunks` and `youtube_youtubers` as
compressed JSONL plus checksum and document-count manifests.

Copy the completed timestamp directory to independent object storage. Keep at
least seven daily and three monthly copies. Do not expose the bucket publicly.

Cloudflare R2 is the intended target because its Standard storage free tier
currently includes 10 GB-month and egress is free. R2 is not yet enabled on the
Cloudflare account; Wrangler currently returns API error `10042`. Enable R2 in
the dashboard once, create a private `ytsearch-backups` bucket, then upload each
completed backup directory. Pricing and limits can change, so verify the
[current R2 pricing](https://developers.cloudflare.com/r2/pricing/) first.

## Restore Drill

Create an empty isolated Typesense instance with the same collection schemas.
Never test a restore against production. Then run:

```sh
TYPESENSE_SELFHOST_API_URL=http://127.0.0.1:8108 \
TYPESENSE_SELFHOST_API_KEY='<isolated-admin-key>' \
scripts/restore-typesense.sh backups/<timestamp>
```

After import, verify:

- both collection document counts match `COUNTS.tsv`;
- `scripts/check-indexed-channel-status.sh` reports expected creator counts;
- the fixed search suite returns stable video and moment totals;
- a browser search opens the expected timestamp.

Run this drill quarterly. An untested archive is not a reliable backup.

## Fly Snapshot Restore

Fly automatically takes daily snapshots. The current volume IDs are:

- Typesense: `vol_v8e6yy17jo63nplv`
- API queue: `vol_vwnydd578odzzo8v`

List snapshots with `fly volumes snapshots list <volume-id> --app <app>`. Restore
into a new volume or Machine first; do not overwrite the only production copy.
Fly documents the current commands in its
[snapshot guide](https://fly.io/docs/volumes/snapshots/).

## Recovery Objectives

- Machine crash: automatic restart, no data restore expected.
- Volume-host loss: restore the newest Fly snapshot; recovery-point objective is
  under 24 hours.
- Fly account or regional loss: restore the newest off-provider JSONL export;
  recovery-point objective is under 24 hours after daily uploads are automated.
- Lost queue entries: restore the 1 GB queue-volume snapshot, then reconcile
  indexed/public creator counts before restarting the single writer.
