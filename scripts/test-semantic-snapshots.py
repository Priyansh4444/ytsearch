#!/usr/bin/env python3
"""Exercise snapshot exports through the command-line boundary."""

import datetime
import gzip
import hashlib
import json
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import unittest


EXPORTER = pathlib.Path(__file__).with_name("build-semantic-snapshots.py")


class SnapshotExports(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ytsearch-snapshot-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.archive = self.root / "chunks.jsonl.gz"
        self.output = self.root / "exports"
        self.video_ids = {f"video{i:06d}" for i in range(129)}
        with gzip.open(self.archive, "wt", encoding="utf-8") as source:
            for i in range(129):
                row = {
                    "id": f"chunk{i}",
                    "video_id": f"video{i:06d}",
                    "title": "Fixture",
                    "text": "A short complete transcript.",
                    "youtuber_handle": "@fixture",
                    "youtuber_name": "Fixture",
                    "t_start_ms": 0,
                    "t_end_ms": 1000,
                    "published_at": str(datetime.date(2026, 1, 1) - datetime.timedelta(days=i))
                    + "T00:00:00Z",
                }
                source.write(json.dumps(row) + "\n")

    def export(self, *arguments):
        return subprocess.run(
            [sys.executable, "-I", str(EXPORTER), str(self.archive), str(self.output),
             "--creator", "@fixture", *arguments],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )

    def add_mutations(self, rows):
        database = self.root / "mutations.sqlite3"
        connection = sqlite3.connect(database)
        connection.execute("CREATE TABLE video_mutations (video_id TEXT, chunks_json BLOB)")
        connection.executemany("INSERT INTO video_mutations VALUES (?, ?)", rows)
        connection.commit()
        connection.close()
        return database

    def exported(self):
        rows = []
        for path in sorted(self.output.glob("snapshot-*.jsonl")):
            rows += [json.loads(line) for line in path.read_text().splitlines()]
        return rows

    def test_mutations_replace_and_delete_archive_videos(self):
        replacement = {
            "id": "new", "video_id": "video000000", "title": "Replaced", "text": "Fresh words.",
            "youtuber_handle": "@fixture", "youtuber_name": "Fixture", "t_start_ms": 0,
            "t_end_ms": 1000,
        }
        added = {**replacement, "id": "added", "video_id": "videoadded", "title": "Added"}
        other = {**replacement, "id": "other", "video_id": "video000002", "youtuber_handle": "@else"}
        database = self.add_mutations([
            ("video000000", json.dumps([replacement]).encode()),
            ("video000001", None),
            ("videoadded", json.dumps([added]).encode()),
            ("video000002", json.dumps([other]).encode()),
        ])
        result = self.export("--mutations", str(database))
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = {row["video_id"]: row for row in self.exported()}
        self.assertEqual(rows["video000000"]["title"], "Replaced")
        self.assertEqual(rows["videoadded"]["title"], "Added")
        self.assertNotIn("video000001", rows)
        # Moved to a creator that was not selected: the archive copy is stale, not kept.
        self.assertNotIn("video000002", rows)
        self.assertEqual(len(rows), 128)

    def test_rename_files_old_chunks_under_the_new_handle_and_id(self):
        result = self.export("--rename", "@fixture=@renamed", "--newest", "2")
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = self.exported()
        self.assertEqual(len(rows), 2)
        self.assertEqual({row["youtuber_handle"] for row in rows}, {"@renamed"})
        self.assertEqual({row["youtuber_id"] for row in rows}, {"renamed"})
        self.assertEqual({row["uploader_id"] for row in rows}, {"@renamed"})

    def test_a_rename_selects_the_creator_by_either_handle(self):
        self.output = self.root / "by-new-handle"
        result = subprocess.run(
            [sys.executable, "-I", str(EXPORTER), str(self.archive), str(self.output),
             "--creator", "@renamed", "--rename", "@FIXTURE=@renamed", "--newest", "1"],
            capture_output=True, text=True, timeout=10, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.exported()), 1)

    def files(self):
        return {path.name: path.read_bytes() for path in self.output.iterdir()}

    def test_fresh_export_contains_complete_videos_and_matching_receipts(self):
        result = self.export()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["snapshots"], 2)
        observed = []
        batches = sorted(self.output.glob("snapshot-*.jsonl"))
        self.assertEqual(len(batches), 2)
        for path in batches:
            data = path.read_bytes()
            rows = [json.loads(line) for line in data.splitlines()]
            self.assertLessEqual(len(rows), 128)
            observed.extend(row["video_id"] for row in rows)
            receipt = json.loads(path.with_name(path.name + ".manifest.json").read_text())
            self.assertEqual(receipt["sha256"], hashlib.sha256(data).hexdigest())
            self.assertEqual(receipt["complete_videos"], {row["video_id"]: 1 for row in rows})
        self.assertEqual(set(observed), self.video_ids)
        self.assertEqual(len(observed), len(self.video_ids))

    def test_smaller_repeat_export_rejects_reuse_and_preserves_every_original_byte(self):
        first = self.export()
        self.assertEqual(first.returncode, 0, first.stderr)
        original = self.files()
        second = self.export("--newest", "1")
        self.assertNotEqual(second.returncode, 0, "a repeat export can leave stale batches")
        self.assertIn("fresh output directory", second.stderr)
        self.assertEqual(self.files(), original)

    def test_preexisting_directory_is_not_claimed_even_when_empty(self):
        self.output.mkdir()
        result = self.export("--newest", "1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("fresh output directory", result.stderr)
        self.assertEqual(self.files(), {})

    def test_preexisting_files_are_not_overwritten_or_deleted(self):
        self.output.mkdir()
        (self.output / "notes.txt").write_bytes(b"operator-owned\x00notes")
        original = self.files()
        result = self.export("--newest", "1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.files(), original)


if __name__ == "__main__":
    unittest.main()
