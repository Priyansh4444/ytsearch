# Raw publication release

Use this runbook to prove a new Raw reader and detached builder on the isolated
2-GB canary. It does not promote the release or touch the production reader.
The runner confirms that the canary and builder are stopped before it records
success. If it cannot confirm a stop, it fails and retains the Machine lease and
recovery session instead of claiming success.

## Fixed resources

[`config/fly-production-resources.json`](../../config/fly-production-resources.json)
is the only executable inventory of the isolated reader, detached builder, and
deprecated Typesense preservation set. Inspect the two release roles with:

```bash
jq '.resources | {raw_reader_canary, raw_publication_builder}' \
  config/fly-production-resources.json
```

The reader must remain stopped, have no services or checks, skip DNS
registration, use restart policy `no`, and have exactly 2 shared CPUs and 2 GB
of memory. The builder must remain an inert `sleep infinity` Machine with no
services and restart policy `no`. The scripts reject any drift from these
identities and shapes.

## Build immutable overlays

Commit the release first. Choose the exact tested corpus image, including its
full registry digest. The image labels are derived from the full source commit
and base digest; operators cannot override them.

Uploading the committed source to Fly's private remote builder and registry is
an external release action. Obtain explicit approval for the exact commit
before running either build.

Resolve the tested base corpus from the exact production reader selected by the
checked-in Worker configuration. Fly supplies the immutable image digest. Do
not type a tag, commit prefix, or digest:

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
scripts/build-raw-search-overlay.sh | tee /tmp/raw-reader-image.json
scripts/build-raw-indexer-overlay.sh | tee /tmp/raw-builder-image.json
```

Both commands archive only the committed build inputs. Their JSON output binds
the source commit, base image, and derived image name.
The canary also reads that source commit and base-image digest from files baked
into each running overlay. Machine metadata alone is not accepted as image
provenance.

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

Staging never starts either Machine.
Builder reconciliation replaces the obsolete repair configuration as one exact
Machine config; stale queue environment variables, embedded scripts, file
mappings, services, and checks are not carried into the publication role. The
volume and Machine identity are preserved.

## Run the complete gate

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
   images, private target, isolation, and 2-GB shapes.
2. Starts only the isolated reader and connects to its exact private IPv6
   address through a local Fly proxy.
3. Requires publication protocol v1 and the exact compiled source commit.
4. Proves missing and mismatched publication versions are rejected without
   consuming a mutation sequence.
5. Writes the same valid channel mutation twice and requires one durable
   operation and one sequence advance.
6. Publishes with the detached builder while continuously running the canonical
   mixed-query workload at concurrency four.
7. Requires every request to complete in at most 200 ms with zero HTTP, JSON,
   metrics, or search errors.
8. Requires reader process RSS at or below 1.5 GiB. For both 2-GB Machines it
   also requires observed cgroup memory at or below 1.85 GiB, observed swap of
   zero, no OOM event, and volume utilization below 80%. Cgroup v2 uses kernel
   peak counters. Fly's current cgroup v1 reports an unusable zero peak, so the
   gate records that limitation, requires v1 memory-plus-swap accounting, and
   fails if consecutive accepted samples are more than six seconds apart. It
   never labels current-usage samples as a kernel peak.
9. Runs the warmed new generation for 60 seconds, verifies the operation is
   published, restarts the reader, and verifies it again.
10. Deletes the synthetic channel, publishes the deletion, restarts again, and
    requires the channel to remain absent.

The six-hour reader lease is a worst-case mutual-exclusion and recovery
envelope derived from both complete builder runs, restarts, health checks, and
cleanup. It does not keep a stopped Machine running for six hours; normal
cleanup stops the Machines and releases both leases immediately.

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

Typesense retirement, production promotion, queue-worker restoration, and
creator reconciliation remain separate gates. See
[Raw mutable migration](raw-mutable-migration.md) and
[Typesense retirement](typesense-retirement.md).
