#!/usr/bin/env python3
"""Small independent fixtures for the Raw posting overlap survey."""

import importlib.util
import mmap
import struct
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("analyze-raw-posting-overlap.py")
SPEC = importlib.util.spec_from_file_location("raw_posting_overlap", SCRIPT)
assert SPEC and SPEC.loader
ANALYZER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYZER)


def lexicon_entry(term: bytes, offset: int, length: int, df: int) -> bytes:
    return struct.pack("<H", len(term)) + term + struct.pack(
        "<QIIQII", offset, length, df, offset + length, 0, 0
    )


class PostingOverlapTests(unittest.TestCase):
    def test_complete_pair_and_missing_terms(self) -> None:
        and_bytes = bytes([0, 1, 2, 1])
        the_bytes = bytes([2, 1, 2, 1])
        entries = [
            lexicon_entry(b"and", 16, len(and_bytes), 2),
            lexicon_entry(b"the", 20, len(the_bytes), 2),
        ]
        lexicon = (
            b"RAWLEX01"
            + struct.pack("<II", 2, 1)
            + b"".join(
                struct.pack("<Q", offset)
                for offset in (0, len(entries[0]), sum(map(len, entries)))
            )
            + b"".join(entries)
        )
        postings = b"RAWPOS01" + struct.pack("<II", 2, 1) + and_bytes + the_bytes
        with tempfile.TemporaryDirectory() as directory:
            lexicon_path = Path(directory) / "lexicon.bin"
            posting_path = Path(directory) / "postings.bin"
            lexicon_path.write_bytes(lexicon)
            posting_path.write_bytes(postings)
            result = ANALYZER.analyze(lexicon_path, posting_path, 5, 2)
        pair = next(
            item
            for item in result["pairs"]
            if item["left"] == "the" and item["right"] == "and"
        )
        self.assertEqual(pair["text"]["intersection"], 1)
        self.assertEqual(pair["text"]["union"], 3)
        self.assertEqual(pair["title"]["intersection"], 0)
        react = next(item for item in result["terms"] if item["term"] == "react")
        self.assertFalse(react["present"])

    def test_varint_rejects_truncation_and_overflow(self) -> None:
        for data in (
            bytes([0x80]),
            bytes([0x80, 0x80, 0x80, 0x80, 0x10]),
            bytes([0x81, 0]),
        ):
            with self.subTest(data=data), tempfile.TemporaryFile() as file:
                file.write(data)
                file.flush()
                with mmap.mmap(file.fileno(), 0, access=mmap.ACCESS_READ) as mapped:
                    with self.assertRaises(ValueError):
                        ANALYZER.read_varint(mapped, 0, len(data))


if __name__ == "__main__":
    unittest.main()
