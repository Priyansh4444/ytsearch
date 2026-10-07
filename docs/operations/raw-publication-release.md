# Raw publication release

Proves a new Raw reader and detached builder on the isolated 2-GB canary. It
never promotes the release or touches the production reader, and it records
success only after confirming both Machines stopped.

## Fixed resources

[`config/fly-production-resources.json`](../../config/fly-production-resources.json)
is the only executable inventory of the production Machines and volumes. Inspect the two release roles with:

```bash
jq '.resources | {raw_reader_canary, raw_publication_builder}' \
  config/fly-production-resources.json
```

Both stay stopped with restart policy `no` and no services. The reader also
has no checks, skips DNS, and has exactly 2 shared CPUs and 2 GB. The builder is
an inert `sleep infinity` Machine: one performance CPU and 4 GB for a full schema
migration, two shared CPUs and 2 GB for the default incremental profile. The
scripts reject any drift.

## Build immutable overlays

Commit the release first; image labels derive from the full source commit and
base digest. Uploading source to Fly's builder is an external action: get
explicit approval for the exact commit before either build. Resolve the base
image from the production reader that `wrangler.jsonc` selects, never by typing
a tag or digest:

```bash
read -r app machine < <(
  bun -e 'import config from "./wrangler.jsonc"; console.log(config.vars.FLY_RAW_SEARCH_APP_NAME, config.vars.FLY_RAW_SEARCH_MACHINE_ID)'
)
export BASE_IMAGE="$(
  flyctl machine list --app "$app" --json | jq -er --arg app "$app" --arg id "$machine" '
    [.[] | select(
      .id == $id and .state == "started" and
      .config.env.RAW_SEARCH_STATE_DIRECTORY == "/data/mutable" and
      (.image_ref.digest | test("^sha256:[0-9a-f]{64}$"))
    )] |
    if length == 1 then
      "registry.fly.io/" + $app + "@" + .[0].image_ref.digest
    else
      error("checked-in production reader is absent, stopped, or incomplete")
    end
  '
)"
scripts/build-raw-overlay.sh reader | tee /tmp/raw-reader-image.json
scripts/build-raw-overlay.sh indexer | tee /tmp/raw-builder-image.json
```

Both archive only committed inputs, and their JSON binds the source commit,
base image and image name. The canary checks provenance from files baked into
each overlay, not Machine metadata.

## Stage exact stopped Machines

First inspect both changes without applying them:

```bash
BASE_IMAGE="$BASE_IMAGE" scripts/stage-raw-reader-canary.sh
BASE_IMAGE="$BASE_IMAGE" scripts/reconcile-raw-publication-builder.sh
```

Then apply the same checked inputs:

```bash
APPLY_RAW_READER_CANARY=stage-exact-isolated-canary \
  BASE_IMAGE="$BASE_IMAGE" scripts/stage-raw-reader-canary.sh \
  | tee /tmp/raw-reader-canary.json

APPLY_RAW_PUBLICATION_BUILDER=configure-exact-inert-builder \
  BASE_IMAGE="$BASE_IMAGE" scripts/reconcile-raw-publication-builder.sh \
  | tee /tmp/raw-publication-builder.json
```

Staging never starts either Machine. The reader's state directory is
`/data/mutable/trials/<source-commit-prefix>`, so each commit starts from the
immutable base index, apart from earlier canary state. Verify the base image's
`/data/index/manifest.json` first: as of 2026-10-04 it is schema 6 with
8,142,650 documents, 32,822 videos and 61 channels, and production serves
schema-9 generations published from it. A canary that never publishes proves
only that the base opens, not schema 9. Builder reconciliation writes one exact
Machine config, dropping stale queue settings while keeping the volume and
Machine identity.

The canary reads `RAW_SEARCH_API_KEY` and `RAW_SEARCH_ADMIN_API_KEY` from
`/run/secrets` and refuses to start without both. Fly base64-decodes secrets
written through `config.files`, so the entrypoint re-encodes them to match
environment injection, which production still uses. Staged secrets are not
visible through `config.files.secret_name`: run `flyctl secrets deploy` for any
reader/API rotation before staging. Verify a credential only by a one-way digest
of the `youtube-raw-search` process environment; `fly ssh console ... env` shows
the SSH process, not the server.

## Run the complete gate

- A cold start may take up to 300 one-second health probes: checksumming and
  opening the active generation took 193 s on the 2 GB canary (2026-08-31).
  Search cases still fail above 200 ms.
- With a short-lived `flyctl auth token` as `FLY_API_TOKEN`, set
  `FLY_MACHINE_SESSION_ALLOW_CLI_TOKEN_REFRESH=1`: after a 401/403 the helper
  refreshes the token and retries reads once; a rejected mutation still needs an
  authoritative state check. A scoped token keeps the fail-closed default.
- The queue worker must target the exact production Machine,
  `d89303da029328.vm.ytsearch-raw-production.internal`, never the app-wide name,
  which can route to a started canary. The runner checks this first.
- Broad single-token queries once OOM-killed the 2 GB reader (2026-08-31); they
  now merge posting cursors into bounded per-video slots, and the HTTP service
  runs one backend search at a time. The gate must include broad terms such as
  `react` and `fire`, keep every request below 200 ms, and see no new OOM.

Every gate request travels from the operator's machine through a local Fly
proxy, so the 200 ms limit includes that round trip. On 2026-10-07 a gate run
from a laptop measured a 47 ms floor (fastest request) and failed the baseline
at 235 ms on the stopword-heavy `and_the_spaced` case; the reader alone was
about 185 ms. Run the gate from a Machine in `sjc` or another host with a
small round trip to it, and treat a failure close to the limit as unproven
until it is.

```bash
SEARCH_API_KEY="$RAW_CANARY_SEARCH_API_KEY" \
RAW_ADMIN_API_KEY="$RAW_SEARCH_ADMIN_API_KEY" \
EXPECTED_RAW_READER_IMAGE="$(jq -r .resolved_image /tmp/raw-reader-canary.json)" \
EXPECTED_RAW_BUILDER_IMAGE="$(jq -r .resolved_image /tmp/raw-publication-builder.json)" \
ALLOW_RAW_CANARY_GATE=run-exact-isolated-canary \
  scripts/run-raw-publication-canary.sh
```

The one command performs these checks in order:

1. Revalidates the exact stopped reader, builder, volumes, source commit,
   images, private target, isolation, and profile-specific shapes.
2. Starts only the isolated reader and connects to its exact private IPv6
   address through a local Fly proxy.
3. Requires publication protocol v1 and the exact compiled source commit.
4. Proves missing and mismatched publication versions are rejected without
   consuming a mutation sequence.
5. Writes the same valid channel mutation twice and requires one durable
   operation and one sequence advance.
6. Runs the canonical mixed-query workload at concurrency four for 120 seconds
   during the build. Before activation, an explicit handshake starts a second
   ten-second load window and blocks the swap until all workers are ready.
   The builder records the exact remote activation start and finish timestamps;
   the gate requires concurrency-four requests across that interval.
7. Requires every direct Raw request on the private Fly path to complete in
   under 200 ms with zero HTTP, JSON, metrics, or search errors. Browser and
   Cloudflare transit time remain a separate production check.
8. Requires reader process RSS at or below 1.5 GiB. It requires reader cgroup
   memory at or below 1.6 GiB and temporary 4-GB builder cgroup memory at or
   below 3.2 GiB. It also requires observed swap of zero, no OOM event, and
   volume utilization below 80%. These gates measure memory and disk, not CPU
   utilization. Cgroup v2 uses kernel
   peak counters. Fly's current cgroup v1 reports an unusable zero peak, so the
   gate records that limitation, requires v1 memory-plus-swap accounting, and
   fails if consecutive accepted samples are more than six seconds apart. It
   never labels current-usage samples as a kernel peak.
9. Runs the warmed new generation for 60 seconds, verifies the operation is
   published, restarts the reader, and verifies it again.
10. Deletes the synthetic channel, publishes the deletion, restarts again, and
    requires the channel to remain absent.

The six-hour reader lease is a worst-case envelope for two builder runs,
restarts and cleanup; normal cleanup stops both Machines and releases the leases
immediately.

Evidence is written under
`.search-experiment/results/raw-canary-<commit>-<timestamp>/`. A successful run
ends with `result.json`, which attests the full source commit, corpus digest,
and resolved reader and builder image digests. The runner passes the admin key
through a mode-600 temporary header file instead of a command argument.

## Failure handling

Do not promote a partial or failed run. Publication mutations are durable, so a
failed run may be retried with a new run ID after correcting the cause. The
builder wrapper exclusively leases its exact Machine and stops it before
releasing the lease. The canary runner stops the exact reader during cleanup.
If an authoritative stop or lease release cannot be confirmed, the command
fails without a success attestation and preserves its mode-700 session under
`${XDG_STATE_HOME:-${TMPDIR:-/tmp}}/ytsearch/fly-machine-recovery/`. The printed
path contains the nonce needed to retry cleanup and must not be committed.
