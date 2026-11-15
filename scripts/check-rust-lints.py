#!/usr/bin/env python3
"""Reject new Rust lint debt while retaining a reviewable, explicit baseline."""

import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BASELINE = ROOT / "config/rust-lint-baseline.json"


def cargo_command(manifest):
    rules = manifest["workspace"]["lints"]["clippy"]
    command = [
        "cargo", "clippy", "--workspace", "--all-targets", "--all-features",
        "--message-format=json", "--", "--deny", "warnings",
    ]
    for rule, setting in sorted(rules.items()):
        level = setting.get("level") if isinstance(setting, dict) else setting
        if level in ("warn", "deny", "forbid"):
            command.extend(["--force-warn", f"clippy::{rule}"])
    return command


def fingerprint(message):
    """Use source context rather than unstable line/column/byte positions."""
    locations = []
    for span in message.get("spans", []):
        if not span.get("is_primary"):
            continue
        filename = Path(span["file_name"])
        if filename.is_absolute():
            filename = filename.relative_to(ROOT)
        source = " ".join(
            " ".join(line["text"] for line in span.get("text", [])).split()
        )
        if not source:
            raise ValueError("lint has no primary source context")
        locations.append({"file": filename.as_posix(), "source": source})
    if not locations:
        raise ValueError("lint has no primary source location")
    return json.dumps(
        {"rule": message["code"]["code"], "message": message["message"],
         "locations": locations},
        sort_keys=True,
    )


def diagnostics(output):
    counts = Counter()
    failures = []
    for line in output.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            failures.append(f"Invalid Cargo JSON output: {line}")
            continue
        if event.get("reason") != "compiler-message":
            continue
        message = event["message"]
        if message["level"] not in ("warning", "error", "ice"):
            continue
        code = (message.get("code") or {}).get("code", "")
        if message["level"] != "warning" or not code.startswith("clippy::"):
            failures.append(message.get("rendered") or message["message"])
            continue
        try:
            counts[fingerprint(message)] += 1
        except (ValueError, KeyError) as error:
            failures.append(f"Cannot baseline diagnostic: {error}: {message['message']}")
    return counts, failures


def read_baseline(path):
    baseline = json.loads(path.read_text())
    if baseline["version"] != 1:
        raise ValueError("unsupported Rust lint baseline version")
    counts = Counter()
    for entry in baseline["diagnostics"]:
        count = entry["count"]
        if type(count) is not int or count <= 0:
            raise ValueError("baseline occurrence counts must be positive integers")
        key = json.dumps(entry["diagnostic"], sort_keys=True)
        if key in counts:
            raise ValueError("duplicate baseline diagnostic")
        counts[key] = count
    return counts


def write_baseline(path, counts):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({
        "version": 1,
        "diagnostics": [
            {"diagnostic": json.loads(key), "count": count}
            for key, count in sorted(counts.items())
        ],
    }, indent=2) + "\n")


def check(result, baseline_path, update=False):
    current, failures = diagnostics(result.stdout)
    if result.returncode:
        failures.append(f"Cargo Clippy failed (exit {result.returncode}).")
    if failures:
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1
    if update:
        write_baseline(baseline_path, current)
        print(f"Recorded {sum(current.values())} existing Rust lint occurrences.")
        return 0
    baseline = read_baseline(baseline_path)
    added = current - baseline
    resolved = baseline - current
    if added:
        for key, count in sorted(added.items()):
            entry = json.loads(key)
            location = entry["locations"][0]
            print(f"NEW ({count}): {location['file']}: {entry['rule']}: "
                  f"{entry['message']}\n  {location['source']}", file=sys.stderr)
        print(f"Rejected {sum(added.values())} new Rust lint occurrences.", file=sys.stderr)
        return 1
    print(f"Rust lint baseline passed: {sum(current.values())} existing occurrences; "
          f"{sum(resolved.values())} resolved."
          + (" Run --update-baseline to remove resolved entries." if resolved else ""))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--update-baseline", action="store_true",
                        help="explicitly replace the baseline with current diagnostics")
    args = parser.parse_args()
    try:
        manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
        # force-warn keeps every configured lint visible despite deny levels in
        # manifests/source. Ordinary compiler warnings and errors still fail.
        result = subprocess.run(cargo_command(manifest), cwd=ROOT, text=True,
                                stdout=subprocess.PIPE, check=False)
        return check(result, args.baseline, args.update_baseline)
    except (OSError, ValueError, KeyError) as error:
        print(f"Rust lint check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
