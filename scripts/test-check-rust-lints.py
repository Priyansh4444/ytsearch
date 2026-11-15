#!/usr/bin/env python3
"""Independent Cargo diagnostic fixtures for the Rust lint baseline gate."""

from contextlib import redirect_stderr, redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "rust_lints", Path(__file__).with_name("check-rust-lints.py")
)
lints = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lints)


def fixture(source="let first = values[0];", rule="clippy::indexing_slicing",
            line=7, level="warning"):
    return {"reason": "compiler-message", "message": {
        "level": level, "code": {"code": rule}, "message": "indexing may panic",
        "spans": [{"file_name": "crates/example/src/lib.rs", "is_primary": True,
                   "line_start": line, "line_end": line, "column_start": 13,
                   "byte_start": line * 20, "text": [{"text": source}]}],
    }}


def result(*events, returncode=0):
    return subprocess.CompletedProcess([], returncode,
                                       "\n".join(json.dumps(e) for e in events))


class BaselineTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.baseline = Path(self.temp.name) / "baseline.json"
        self.assertEqual(self.run_check(result(fixture()), update=True), 0)

    def run_check(self, cargo_result, update=False):
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            return lints.check(cargo_result, self.baseline, update)

    def test_line_shifts_and_whitespace_do_not_create_debt(self):
        shifted = fixture("    let   first = values[0];", line=400)
        self.assertEqual(self.run_check(result(shifted)), 0)

    def test_new_rule_rejected_despite_same_total(self):
        self.assertEqual(self.run_check(result(fixture(rule="clippy::string_slice"))), 1)

    def test_new_source_rejected_despite_same_total(self):
        self.assertEqual(self.run_check(result(fixture("let second = other[0];"))), 1)

    def test_duplicates_are_counted(self):
        self.assertEqual(self.run_check(result(fixture(), fixture(line=30))), 1)

    def test_resolved_occurrences_allowed_without_mutating_baseline(self):
        before = self.baseline.read_text()
        self.assertEqual(self.run_check(result()), 0)
        self.assertEqual(self.baseline.read_text(), before)

    def test_compiler_error_never_baselined(self):
        before = self.baseline.read_text()
        self.assertEqual(self.run_check(result(fixture(rule="E0308", level="error")),
                                        update=True), 1)
        self.assertEqual(self.baseline.read_text(), before)

    def test_failed_cargo_never_baselined(self):
        before = self.baseline.read_text()
        self.assertEqual(self.run_check(result(fixture(), returncode=101), update=True), 1)
        self.assertEqual(self.baseline.read_text(), before)

    def test_non_clippy_warning_never_baselined(self):
        self.assertEqual(self.run_check(result(fixture(rule="unused_imports")), update=True), 1)

    def test_missing_source_context_rejected(self):
        event = fixture()
        event["message"]["spans"][0]["text"] = []
        self.assertEqual(self.run_check(result(event), update=True), 1)

    def test_all_configured_groups_and_rules_forced_visible(self):
        manifest = {"workspace": {"lints": {"clippy": {
            "pedantic": {"level": "deny", "priority": -1},
            "panic": "deny", "unwrap_used": "warn", "style": "allow",
        }}}}
        command = lints.cargo_command(manifest)
        self.assertIn("--all-features", command)
        self.assertIn("warnings", command)
        self.assertIn("clippy::pedantic", command)
        self.assertIn("clippy::panic", command)
        self.assertIn("clippy::unwrap_used", command)
        self.assertNotIn("clippy::style", command)
        self.assertNotIn("--cap-lints", command)


if __name__ == "__main__":
    unittest.main()
