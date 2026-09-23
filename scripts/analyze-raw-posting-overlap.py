#!/usr/bin/env python3
"""Read-only overlap survey of named Raw benchmark terms in one generation."""

import argparse
from array import array
import hashlib
import json
import mmap
import struct
from pathlib import Path

PAIRS = (
    ("the", "and"),
    ("in", "the"),
    ("this", "is"),
    ("is", "the"),
    ("coding", "dead"),
    ("css", "dead"),
    ("create", "effect"),
    ("black", "hole"),
    ("one", "fire"),
)
SINGLES = ("react", "convex", "the", "and")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_varint(data: mmap.mmap, cursor: int, end: int) -> tuple[int, int]:
    value = 0
    for shift in range(0, 35, 7):
        if cursor >= end:
            raise ValueError("truncated posting varint")
        byte = data[cursor]
        cursor += 1
        if shift == 28 and byte > 0x0f:
            raise ValueError("posting varint overflows u32")
        value |= (byte & 0x7f) << shift
        if not byte & 0x80:
            if shift and byte == 0:
                raise ValueError("noncanonical posting varint")
            return value, cursor
    raise ValueError("posting varint exceeds five bytes")


def read_postings(data: mmap.mmap, offset: int, length: int, count: int, universe: int) -> array:
    end = offset + length
    if offset < 16 or end > len(data):
        raise ValueError("posting range outside file")
    output = array("I")
    previous = 0
    cursor = offset
    while cursor < end:
        delta, cursor = read_varint(data, cursor, end)
        frequency, cursor = read_varint(data, cursor, end)
        if output and delta == 0:
            raise ValueError("duplicate posting ID")
        if frequency == 0:
            raise ValueError("zero posting frequency")
        previous += delta
        if previous >= universe or previous > 0xffffffff:
            raise ValueError("posting ID outside universe")
        output.append(previous)
        if len(output) > count:
            raise ValueError("posting count exceeds lexicon DF")
    if len(output) != count:
        raise ValueError("posting count differs from lexicon DF")
    return output


def overlap(left: array, right: array) -> int:
    a = b = matches = 0
    while a < len(left) and b < len(right):
        if left[a] < right[b]:
            a += 1
        elif left[a] > right[b]:
            b += 1
        else:
            matches += 1
            a += 1
            b += 1
    return matches


def lookup(data: mmap.mmap, term: str) -> dict | None:
    if len(data) < 16 or data[:8] != b"RAWLEX01":
        raise ValueError("invalid lexicon header")
    count, version = struct.unpack_from("<II", data, 8)
    if version != 1:
        raise ValueError("unsupported lexicon version")
    entries_start = 16 + (count + 1) * 8
    if entries_start > len(data):
        raise ValueError("truncated lexicon table")
    last = struct.unpack_from("<Q", data, 16 + count * 8)[0]
    if entries_start + last != len(data):
        raise ValueError("invalid lexicon final offset")
    wanted = term.encode("utf-8")
    low, high = 0, count
    while low < high:
        middle = low + (high - low) // 2
        start = entries_start + struct.unpack_from("<Q", data, 16 + middle * 8)[0]
        end = entries_start + struct.unpack_from("<Q", data, 16 + (middle + 1) * 8)[0]
        if end > len(data) or end - start < 34:
            raise ValueError("invalid lexicon entry")
        term_length = struct.unpack_from("<H", data, start)[0]
        fields = start + 2 + term_length
        if fields + 32 != end:
            raise ValueError("invalid lexicon entry length")
        candidate = data[start + 2 : fields]
        if candidate < wanted:
            low = middle + 1
        elif candidate > wanted:
            high = middle
        else:
            text_offset, text_length, text_df, title_offset, title_length, title_df = (
                struct.unpack_from("<QIIQII", data, fields)
            )
            return {
                "text": (text_offset, text_length, text_df),
                "title": (title_offset, title_length, title_df),
            }
    return None


def analyze(lexicon: Path, postings: Path, documents: int, videos: int) -> dict:
    terms = sorted(set(SINGLES) | {term for pair in PAIRS for term in pair})
    with lexicon.open("rb") as lexicon_file, postings.open("rb") as posting_file:
        with mmap.mmap(lexicon_file.fileno(), 0, access=mmap.ACCESS_READ) as lex:
            with mmap.mmap(posting_file.fileno(), 0, access=mmap.ACCESS_READ) as data:
                if len(data) < 16 or data[:8] != b"RAWPOS01":
                    raise ValueError("invalid posting file header")
                if struct.unpack_from("<I", data, 12)[0] != 1:
                    raise ValueError("unsupported posting version")
                descriptors = {term: lookup(lex, term) for term in terms}
                decoded = {}
                single_reports = []
                for term in terms:
                    descriptor = descriptors[term]
                    row = {"term": term, "present": descriptor is not None}
                    if descriptor:
                        for field, universe in (("text", documents), ("title", videos)):
                            offset, length, count = descriptor[field]
                            decoded[term, field] = read_postings(data, offset, length, count, universe)
                            row[field] = {"df": count, "encoded_bytes": length}
                    single_reports.append(row)
                pair_reports = []
                for left, right in PAIRS:
                    row = {"left": left, "right": right}
                    for field, universe in (("text", documents), ("title", videos)):
                        a = decoded.get((left, field))
                        b = decoded.get((right, field))
                        if a is None or b is None:
                            row[field] = {"status": "missing_term"}
                            continue
                        matches = overlap(a, b)
                        row[field] = {
                            "intersection": matches,
                            "union": len(a) + len(b) - matches,
                            "dense_bitmap_bytes_per_term": ((universe + 63) // 64) * 8,
                        }
                    pair_reports.append(row)
                return {"terms": single_reports, "pairs": pair_reports}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lexicon", type=Path)
    parser.add_argument("postings", type=Path)
    parser.add_argument("--documents", type=int, required=True)
    parser.add_argument("--videos", type=int, required=True)
    parser.add_argument("--generation", type=int, required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--expect-lexicon-sha256", required=True)
    parser.add_argument("--expect-postings-sha256", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.documents <= 0 or args.videos <= 0 or args.output.exists():
        parser.error("invalid universes or output already exists")
    lex_digest = sha256(args.lexicon)
    posting_digest = sha256(args.postings)
    if lex_digest != args.expect_lexicon_sha256 or posting_digest != args.expect_postings_sha256:
        parser.error("input SHA-256 differs from expected remote digest")
    if len(args.source_commit) != 40 or len(args.manifest_sha256) != 64:
        parser.error("source or manifest digest has invalid length")
    report = {
        "schema_version": 1,
        "analyzer_source_commit": args.source_commit,
        "source": {
            "generation": args.generation,
            "manifest_sha256": args.manifest_sha256,
            "lexicon_sha256": lex_digest,
            "postings_sha256": posting_digest,
            "documents": args.documents,
            "videos": args.videos,
        },
        "analysis": analyze(args.lexicon, args.postings, args.documents, args.videos),
        "limits": [
            "Exact schema-6 raw posting overlap for named terms only; no phrase or expansion semantics.",
            "No timing or schema-9 bitmap threshold is inferred from these intersections.",
        ],
    }
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
