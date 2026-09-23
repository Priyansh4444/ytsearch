#!/usr/bin/env python3
"""Boundary fixtures for the read-only schema-6 lexicon analyzer."""

import importlib.util
import struct
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("analyze-raw-lexicon-df.py")
SPEC = importlib.util.spec_from_file_location("raw_lexicon_df", SCRIPT)
assert SPEC and SPEC.loader
ANALYZER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYZER)


def entry(term: bytes, text_df: int, title_df: int) -> bytes:
    return struct.pack("<H", len(term)) + term + struct.pack(
        "<QIIQII", 0, 1, text_df, 0, 1, title_df
    )


def lexicon(entries: list[bytes]) -> bytes:
    offsets = [0]
    for item in entries:
        offsets.append(offsets[-1] + len(item))
    return (
        b"RAWLEX01"
        + struct.pack("<II", len(entries), 1)
        + b"".join(struct.pack("<Q", offset) for offset in offsets)
        + b"".join(entries)
    )


class LexiconAnalyzerTests(unittest.TestCase):
    def analyze(self, contents: bytes, documents: int = 100, videos: int = 10) -> dict:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lexicon.bin"
            path.write_bytes(contents)
            return ANALYZER.parse_lexicon(path, documents, videos)

    def test_exact_frequencies_and_bitmap_cost(self) -> None:
        report = self.analyze(lexicon([entry(b"alpha", 100, 10), entry(b"beta", 0, 0)]))
        self.assertEqual(report["term_count"], 2)
        self.assertEqual(report["text"]["nonzero_terms"], 1)
        self.assertEqual(report["text"]["max_df"], 100)
        self.assertEqual(report["title"]["max_df"], 10)
        self.assertEqual(report["text"]["dense_bitmap_bytes_per_term"], 16)
        self.assertEqual(report["title"]["dense_bitmap_bytes_per_term"], 8)

    def test_empty_lexicon(self) -> None:
        report = self.analyze(lexicon([]))
        self.assertEqual(report["term_count"], 0)
        self.assertEqual(report["text"]["p99_df"], 0)

    def test_rejects_malformed_boundaries_and_frequencies(self) -> None:
        valid = bytearray(lexicon([entry(b"alpha", 2, 1)]))
        cases = []
        bad_magic = valid.copy()
        bad_magic[:8] = b"BADLEX01"
        cases.append(bad_magic)
        bad_offset = valid.copy()
        struct.pack_into("<Q", bad_offset, 16, 1)
        cases.append(bad_offset)
        bad_length = valid.copy()
        struct.pack_into("<H", bad_length, 32, 60)
        cases.append(bad_length)
        bad_frequency = valid.copy()
        struct.pack_into("<I", bad_frequency, 32 + 2 + 5 + 12, 101)
        cases.append(bad_frequency)
        bad_utf8 = valid.copy()
        bad_utf8[34] = 0xff
        cases.append(bad_utf8)
        for contents in cases:
            with self.subTest(contents=contents[:40]):
                with self.assertRaises((ValueError, UnicodeDecodeError)):
                    self.analyze(contents)


if __name__ == "__main__":
    unittest.main()
