#!/usr/bin/env python3
"""Explain captured creator gaps without claiming a fresh reconciliation.

Input: {catalog: /admin/active/catalog, discoveries: [manifest, ...],
        failures: ytsearch failure-report, state?: selected worker state files}.
No network calls or production writes. Output includes every catalog creator.
"""

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path


def report_video_gaps(capture):
    catalog = capture["catalog"]
    channels = catalog["channels"]
    indexed = catalog["video_ids_by_creator"]
    failures = capture["failures"]
    manifests = defaultdict(list)
    records = defaultdict(list)
    for manifest in capture["discoveries"]:
        manifests[manifest["creator_id"]].append(manifest)
    for record in failures["records"]:
        records[record["video_id"]].append(record)

    active_run = capture.get("state", {}).get("active-run-id", "").strip() or None
    run_ids = sorted({m.get("run_id", "") for ms in manifests.values() for m in ms})
    rows = []
    missing_categories = Counter()
    missing_retry_states = Counter()
    all_indexed = {video for ids in indexed.values() for video in ids}
    all_discovered = set()
    for channel in sorted(channels, key=lambda row: row["handle"].lower()):
        creator = channel["id"]
        indexed_ids = set(indexed.get(creator, []))
        candidates = manifests[creator]
        row = {
            "creator_id": creator,
            "handle": channel["handle"],
            "indexed_video_count": len(indexed_ids),
            "profile_indexed_video_count": channel["indexed_video_count"],
            "profile_public_video_count": channel["public_video_count"],
            "profile_gap": channel["public_video_count"] - channel["indexed_video_count"],
            "manifest_status": "missing",
            "manifest_run_id": None,
            "discovered_video_count": None,
            "missing_video_count": None,
            "missing_by_category": {},
            "missing_by_retry_state": {},
            "unclassified_video_ids": [],
            "ambiguous_failure_video_ids": [],
            "unattributed_failure_video_ids": [],
            "ownership_conflict_video_ids": [],
            "missing_videos": [],
            "indexed_failure_video_ids": sorted(
                video for video in indexed_ids if video in records
            ),
        }
        if len(candidates) > 1:
            row["manifest_status"] = "duplicate"
        elif candidates:
            manifest = candidates[0]
            row["manifest_run_id"] = manifest.get("run_id")
            video_ids = manifest.get("video_ids", [])
            valid = (
                manifest.get("schema_version") == 1
                and manifest.get("status") == "complete"
                and manifest.get("handle", "").lower() == channel["handle"].lower()
                and isinstance(video_ids, list)
                and all(isinstance(video, str) and video for video in video_ids)
                and len(video_ids) == len(set(video_ids))
            )
            if not valid:
                row["manifest_status"] = "invalid"
            else:
                row["manifest_status"] = (
                    "run_unverified" if active_run is None else
                    "active_run" if manifest["run_id"] == active_run else "older_run"
                )
                discovered_ids = set(video_ids)
                all_discovered.update(discovered_ids)
                missing_ids = discovered_ids - indexed_ids
                categories = Counter()
                retry_states = Counter()
                row["discovered_video_count"] = len(discovered_ids)
                row["missing_video_count"] = len(missing_ids)
                row["indexed_not_discovered_video_ids"] = sorted(indexed_ids - discovered_ids)
                for video in sorted(missing_ids):
                    evidence = records[video]
                    outcome = {"video_id": video, "category": None, "retry_state": None}
                    if not evidence:
                        row["unclassified_video_ids"].append(video)
                    elif len(evidence) != 1:
                        row["ambiguous_failure_video_ids"].append(video)
                    else:
                        record = evidence[0]
                        owner = record.get("creator_id")
                        if owner not in (None, creator):
                            row["ownership_conflict_video_ids"].append(video)
                        else:
                            if owner is None:
                                row["unattributed_failure_video_ids"].append(video)
                            outcome.update(category=record["category"], retry_state=record["retry_state"])
                            categories[record["category"]] += 1
                            retry_states[record["retry_state"]] += 1
                    row["missing_videos"].append(outcome)
                row["missing_by_category"] = dict(sorted(categories.items()))
                row["missing_by_retry_state"] = dict(sorted(retry_states.items()))
                missing_categories.update(categories)
                missing_retry_states.update(retry_states)
        rows.append(row)

    def total_ids(field):
        return sum(len(row[field]) for row in rows)

    return {
        "kind": "captured_video_gap_diagnostic",
        "reconciliation_proven": False,
        "limitations": [
            "Discovery manifests may come from different or older runs; this report does not fetch YouTube.",
            "Categories and retry states describe captured failures; they are not reclassified from raw reasons.",
            "Unattributed failures can explain a video but do not prove creator ownership.",
            "Use check-raw-creator-reconciliation.sh after a complete published catalog pass for proof.",
        ],
        "evidence": {
            "generation": catalog["generation"],
            "last_sequence_before": catalog["last_sequence_before"],
            "last_sequence_after": catalog["last_sequence_after"],
            "snapshot_stable": catalog["last_sequence_before"] == catalog["last_sequence_after"],
            "fully_published": catalog["generation"] == catalog["last_sequence_before"] == catalog["last_sequence_after"],
            "active_run_id": active_run,
            "discovery_run_ids": run_ids,
            "manifest_status_counts": dict(sorted(Counter(row["manifest_status"] for row in rows).items())),
        },
        "summary": {
            "creators": len(rows),
            "indexed_videos": sum(row["indexed_video_count"] for row in rows),
            "profile_gap": sum(row["profile_gap"] for row in rows),
            "observed_missing_videos": sum(row["missing_video_count"] or 0 for row in rows),
            "missing_by_category": dict(sorted(missing_categories.items())),
            "missing_by_retry_state": dict(sorted(missing_retry_states.items())),
            "unclassified_videos": total_ids("unclassified_video_ids"),
            "ambiguous_failure_videos": total_ids("ambiguous_failure_video_ids"),
            "unattributed_missing_failures": total_ids("unattributed_failure_video_ids"),
            "ownership_conflicts": total_ids("ownership_conflict_video_ids"),
            "failure_video_ids_already_indexed": len(set(records) & all_indexed),
            "failure_video_ids_outside_captured_discovery": len(set(records) - all_discovered),
            "legacy_failure_records": failures.get("legacy_records", 0),
        },
        "creators": rows,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    print(json.dumps(report_video_gaps(json.loads(args.capture.read_text())), indent=2))


if __name__ == "__main__":
    main()
