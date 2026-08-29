#!/usr/bin/env sh
set -eu

bun run check:precommit
scripts/check-rust-packages.sh
cargo test --workspace
bun run test:e2e
scripts/check-rust-dependency-policy.sh
scripts/test-setup-dev.sh
scripts/test-doctor.sh
scripts/test-check-queue-indexers.sh
scripts/test-reconcile-raw-queue-worker.sh
scripts/test-cloud-index-queue-entrypoint.sh
scripts/check-fly-cost-config.sh
scripts/test-destroy-deprecated-typesense-fly.sh
scripts/test-index-conference-playlists.sh
scripts/test-compare-autoresearch-search.sh
scripts/test-idle-gap-fill.sh
scripts/test-benchmark-search-tools.sh
scripts/test-refresh-query-log.sh

for script in scripts/*.sh; do
  case "$(head -n 1 "$script")" in
    *bash) bash -n "$script" ;;
    *) sh -n "$script" ;;
  esac
done
