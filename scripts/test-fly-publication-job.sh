#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/fly-publication-job.sh"
temporary_directory=$(mktemp -d)
job_directory="/tmp/ytsearch-publication-${temporary_directory##*.}"
trap 'rm -rf "$temporary_directory" "$job_directory"' EXIT
mkdir "$temporary_directory/bin"
export PATH="$temporary_directory/bin:$PATH"
export PUBLICATION_TEST_CALLS="$temporary_directory/calls"
export PUBLICATION_TEST_EXIT=0 PUBLICATION_TEST_SLEEP=0
cat > "$temporary_directory/bin/ytsearch" <<'SH'
#!/usr/bin/env sh
printf 'publish\n' >> "$PUBLICATION_TEST_CALLS"
sleep "$PUBLICATION_TEST_SLEEP"
exit "$PUBLICATION_TEST_EXIT"
SH
chmod +x "$temporary_directory/bin/ytsearch"

RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS=12
RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS=1
RAW_BUILDER_COMMAND_TIMEOUT_SECONDS=2
RAW_BUILDER_APP=fixture
RAW_BUILDER_MACHINE_ID=33333333333333

bounded_flyctl() {
  while [[ "$1" != --command ]]; do shift; done
  local status
  status=$(sh -c "$2")
  # Lose the first acknowledgement after the remote job has been accepted.
  if [[ ! -e "$temporary_directory/disconnected" ]]; then
    touch "$temporary_directory/disconnected"
    return 255
  fi
  printf '%s\n' "$status"
}

fly_publication_job_wait "$job_directory" "$temporary_directory/diagnostics" \
  2>"$temporary_directory/wait.log"
[[ $(wc -l < "$PUBLICATION_TEST_CALLS") == 1 ]]
grep -q 'checking the same job' "$temporary_directory/wait.log"
[[ $(cat "$job_directory/result") == 0 ]]

command=$(fly_publication_job_command "$job_directory" 12 1)
[[ $(sh -c "$command") == complete ]]
[[ $(wc -l < "$PUBLICATION_TEST_CALLS") == 1 ]]

rm -rf "$job_directory"
export PUBLICATION_TEST_EXIT=7
if fly_publication_job_wait "$job_directory" "$temporary_directory/diagnostics" \
  2>"$temporary_directory/failure.log"; then
  echo 'remote failure must not report success' >&2
  exit 1
fi
grep -q 'failed:7' "$temporary_directory/failure.log"
[[ $(wc -l < "$PUBLICATION_TEST_CALLS") == 2 ]]

rm -rf "$job_directory"
export PUBLICATION_TEST_EXIT=0 PUBLICATION_TEST_SLEEP=5
command=$(fly_publication_job_command "$job_directory" 1 1)
[[ $(sh -c "$command") == pending ]]
for ((attempt=0; attempt<40; attempt++)); do
  [[ ! -f "$job_directory/result" ]] || break
  sleep 0.1
done
[[ $(sh -c "$command") == failed:124 ]]
[[ $(wc -l < "$PUBLICATION_TEST_CALLS") == 3 ]]
if fly_publication_job_command '/tmp/invalid;command' 1 1 >/dev/null; then
  echo 'unsafe job path accepted' >&2
  exit 1
fi

rm -rf "$job_directory"
mkdir "$job_directory"
touch -d '1 minute ago' "$job_directory"
[[ $(sh -c "$command" 2>"$temporary_directory/launch.log") == failed:launch-unconfirmed ]]
[[ $(wc -l < "$PUBLICATION_TEST_CALLS") == 3 ]]

bounded_flyctl() { printf 'unrecognized\n'; }
if fly_publication_job_wait "$job_directory" "$temporary_directory/diagnostics" \
  2>"$temporary_directory/malformed.log"; then
  echo 'malformed completion must fail' >&2
  exit 1
fi
grep -q 'Unrecognized publication job status' "$temporary_directory/malformed.log"

bounded_flyctl() { printf 'fixture transport unavailable\n' >&2; return 255; }
RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS=1
status=0
fly_publication_job_wait "$job_directory" "$temporary_directory/diagnostics" \
  2>"$temporary_directory/deadline.log" || status=$?
[[ "$status" == 124 ]]
grep -q 'completion unconfirmed' "$temporary_directory/deadline.log"
grep -q 'fixture transport unavailable' "$temporary_directory/deadline.log"
printf 'bounded publication job tests passed\n'
