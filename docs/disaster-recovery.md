# Disaster recovery

Last updated: 2026-08-09.

## Recovery assets

Keep independent copies of:

1. the immutable base transcript/channel gzip archives;
2. the Raw 20 GB mutable volume, including SQLite WAL and generation manifests;
3. the API 1 GB queue volume;
4. the indexer transcript-cache volume;
5. checked-in source/configuration at the deployed commit.

Fly volume snapshots protect against Machine and volume-host failure. Copy
checksummed base archives and periodic queue/mutation exports to private
off-provider object storage to cover account, provider, and regional loss.

## Snapshot inventory

Resolve live IDs instead of copying IDs from old reports:

```bash
flyctl volumes list --app ytsearch-raw-production
flyctl volumes list --app ytsearch-api-raw-production
flyctl volumes list --app ytsearch-indexer-raw-production
```

List snapshots with:

```bash
flyctl volumes snapshots list VOLUME_ID --app APP_NAME
```

Never restore over the only production copy. Create a new isolated volume and
Machine first.

## Isolated restore drill

1. Restore the Raw mutable snapshot and the exact base archives into an isolated
   app with no public routes.
2. Start `serve-mutable` with new temporary public/admin keys.
3. Require health/readiness success, active-generation checksum validation, and
   no failed/pending recovery operation.
4. Compare document, video, and channel counts with the recorded manifest.
5. Run global/channel `blackhole`, typo, quoted phrase, negative term,
   pagination, engagement, and date-sort cases.
6. Apply an idempotent test-video replacement, publish, and verify visibility.
7. Restart the Machine and repeat the search and operation-status checks.
8. Delete the test video, publish, restart, and verify absence.
9. Restore the queue snapshot separately and compare every request ID/status,
   not only the number of rows.

Run the drill quarterly and after a schema/publication change. An archive that
has not passed open, mutation, publication, and restart tests is not a recovery
asset.

## Failure handling

- Machine crash: restart on the same volume; Raw opens the manifest-selected
  generation and resumes pending publication.
- Candidate corruption or partial build: keep serving the previous verified
  generation; remove only the failed candidate after evidence is captured.
- Mutable-volume loss: restore the latest snapshot, then replay independently
  retained mutation operations newer than the snapshot.
- Queue-volume loss: restore its snapshot, reconcile request IDs against
  published channel/video state, then start the single writer.
- Base archive loss: restore the checksummed off-provider copy before attempting
  another compaction.
- Region/provider loss: provision all three apps in an isolated replacement
  region, pass the full cutover gate, then change the Worker origin.

## Objectives

- Machine crash: no acknowledged-write loss; recovery within one restart.
- Volume-host loss: recovery-point objective under 24 hours until more frequent
  snapshots/exports are configured.
- Queue loss: no request is acknowledged complete until its Raw operation and
  resulting published generation are verifiable.
- Search recovery: the previous verified generation remains the immediate
  rollback target.

Cloudflare R2 is the intended private off-provider target once enabled. Verify
current [R2 pricing](https://developers.cloudflare.com/r2/pricing/) and enable
retention/lifecycle rules before relying on it.
