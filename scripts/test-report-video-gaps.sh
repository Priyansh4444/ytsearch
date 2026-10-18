#!/usr/bin/env bash
set -euo pipefail

python3 - "$(dirname "$0")/report-video-gaps.py" <<'PY'
import copy
import importlib.util
import sys
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("gap_report", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class GapReportTests(unittest.TestCase):
    def setUp(self):
        self.capture = {
            "catalog": {
                "generation": 7, "last_sequence_before": 9, "last_sequence_after": 9,
                "channels": [
                    {"id": "a", "handle": "@a", "indexed_video_count": 2, "public_video_count": 5},
                    {"id": "b", "handle": "@b", "indexed_video_count": 1, "public_video_count": 1},
                ],
                "video_ids_by_creator": {"a": ["existing", "removed"], "b": ["b-existing"]},
            },
            "discoveries": [{"schema_version": 1, "status": "complete", "run_id": "older-run", "creator_id": "a", "handle": "@a", "video_ids": ["existing", "captionless", "retry", "unclassified", "unattributed"]}],
            "failures": {"legacy_records": 1, "records": [
                {"video_id": "existing", "creator_id": "a", "category": "unknown", "retry_state": "ready"},
                {"video_id": "captionless", "creator_id": "a", "category": "no_captions", "retry_state": "terminal"},
                {"video_id": "retry", "creator_id": "a", "category": "timeout", "retry_state": "ready"},
                {"video_id": "unattributed", "creator_id": None, "category": "private", "retry_state": "terminal"},
                {"video_id": "old-failure", "creator_id": "a", "category": "unknown", "retry_state": "ready"},
            ]},
            "state": {"active-run-id": "current-run\n"},
        }

    def test_missing_ids_use_sets_and_do_not_subtract_aggregate_failures(self):
        report = module.report_video_gaps(self.capture)
        self.assertEqual(report["summary"]["creators"], 2)
        self.assertEqual(report["summary"]["observed_missing_videos"], 4)
        self.assertEqual(report["summary"]["missing_by_category"], {"no_captions": 1, "private": 1, "timeout": 1})
        self.assertEqual(report["summary"]["failure_video_ids_already_indexed"], 1)
        self.assertEqual(report["summary"]["failure_video_ids_outside_captured_discovery"], 1)
        self.assertEqual(report["creators"][0]["unclassified_video_ids"], ["unclassified"])
        self.assertEqual(report["creators"][0]["unattributed_failure_video_ids"], ["unattributed"])
        self.assertEqual(report["creators"][1]["manifest_status"], "missing")
        self.assertIsNone(report["creators"][1]["missing_video_count"])
        self.assertFalse(report["reconciliation_proven"])
        self.assertTrue(report["evidence"]["snapshot_stable"])
        self.assertFalse(report["evidence"]["fully_published"])
        self.assertEqual(report["creators"][0]["manifest_status"], "older_run")

    def test_ownership_conflicts_and_duplicate_records_are_not_classified(self):
        capture = copy.deepcopy(self.capture)
        capture["failures"]["records"][1]["creator_id"] = "other"
        capture["failures"]["records"].append(capture["failures"]["records"][2].copy())
        report = module.report_video_gaps(capture)
        self.assertEqual(report["summary"]["ownership_conflicts"], 1)
        self.assertEqual(report["summary"]["ambiguous_failure_videos"], 1)
        self.assertEqual(report["summary"]["missing_by_category"], {"private": 1})

    def test_ambiguous_or_invalid_discovery_does_not_report_zero_gaps(self):
        for mode in ("duplicate", "invalid"):
            capture = copy.deepcopy(self.capture)
            if mode == "duplicate":
                capture["discoveries"].append(capture["discoveries"][0].copy())
            else:
                capture["discoveries"][0]["video_ids"].append("existing")
            report = module.report_video_gaps(capture)
            self.assertEqual(report["creators"][0]["manifest_status"], mode)
            self.assertIsNone(report["creators"][0]["missing_video_count"])


unittest.main(argv=[sys.argv[0]])
PY
