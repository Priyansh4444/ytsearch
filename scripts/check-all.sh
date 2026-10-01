#!/usr/bin/env sh
set -eu

bun run check:precommit
scripts/check-rust-packages.sh
cargo test --workspace
bun run test:e2e
scripts/check-rust-dependency-policy.sh
scripts/check-rust-public-surface.sh
scripts/test-setup-dev.sh
scripts/test-doctor.sh
bash scripts/test-check-markdown-links.sh
scripts/test-check-staging-machines.sh
scripts/test-check-raw-creator-reconciliation.sh
scripts/test-check-queue-indexers.sh
scripts/test-reconcile-raw-queue-worker.sh
scripts/test-reconcile-raw-publication-builder.sh
scripts/test-fly-publish-raw-generation.sh
scripts/test-fly-publication-job.sh
scripts/test-check-raw-publication-protocol.sh
scripts/test-fly-machine-resource-snapshot.sh
scripts/test-run-raw-publication-canary.sh
scripts/test-summarize-publication-load.sh
scripts/test-fly-machine-session.sh
scripts/test-raw-release-inventory.sh
scripts/sync-wrangler-production-resources.sh --check
scripts/test-sync-wrangler-production-resources.sh
scripts/test-build-raw-indexer-overlay.sh
scripts/test-build-raw-search-overlay.sh
scripts/test-raw-search-entrypoint.sh
scripts/test-stage-raw-reader-canary.sh
scripts/test-require-committed-rust-build-inputs.sh
scripts/test-cloud-index-queue-entrypoint.sh
scripts/check-fly-cost-config.sh
scripts/test-index-conference-playlists.sh
scripts/test-idle-gap-fill.sh
scripts/test-benchmark-search-tools.sh
scripts/test-refresh-query-log.sh
scripts/test-fly-set-indexer-secrets.sh

for script in scripts/*.sh; do
  case "$(head -n 1 "$script")" in
    *bash) bash -n "$script" ;;
    *) sh -n "$script" ;;
  esac
done

for script in scripts/lib/*.sh; do
  bash -n "$script"
done
