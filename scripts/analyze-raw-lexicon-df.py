#!/usr/bin/env python3
"""Summarize schema-6 Raw lexicon document frequencies without exporting terms."""

import argparse
import hashlib
import json
import mmap
import struct
from pathlib import Path

MAGIC = b"RAWLEX01"
HEADER_BYTES = 16
ENTRY_FIELDS_BYTES = 32
DENSITY_PPM = (1_000, 10_000, 100_000, 500_000, 900_000)
BUDGET_MIB = (16, 32, 64, 128)


def bitmap_bytes(universe: int) -> int:
    return ((universe + 63) // 64) * 8


def percentile(sorted_values: list[int], numerator: int, denominator: int) -> int:
    if not sorted_values:
        return 0
    index = ((len(sorted_values) - 1) * numerator) // denominator
    return sorted_values[index]


def summarize(values: list[int], universe: int) -> dict:
    values.sort()
    one_bitmap = bitmap_bytes(universe)
    nonzero = sum(value > 0 for value in values)
    return {
        "universe": universe,
        "terms": len(values),
        "nonzero_terms": nonzero,
        "max_df": values[-1] if values else 0,
        "p50_df": percentile(values, 50, 100),
        "p90_df": percentile(values, 90, 100),
        "p99_df": percentile(values, 99, 100),
        "density_bands": [
            {
                "minimum_ppm": ppm,
                "minimum_df": (universe * ppm + 999_999) // 1_000_000,
                "terms_at_or_above": sum(
                    value * 1_000_000 >= universe * ppm for value in values
                ),
            }
            for ppm in DENSITY_PPM
        ],
        "dense_bitmap_bytes_per_term": one_bitmap,
        "bitmap_budget_capacity": [
            {
                "budget_mib": mib,
                "whole_bitmaps": (mib * 1024 * 1024) // one_bitmap,
            }
            for mib in BUDGET_MIB
        ],
    }


def parse_lexicon(path: Path, documents: int, videos: int) -> dict:
    with path.open("rb") as source, mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as data:
        if len(data) < HEADER_BYTES or data[:8] != MAGIC:
            raise ValueError("invalid Raw lexicon header")
        term_count, version = struct.unpack_from("<II", data, 8)
        if version != 1:
            raise ValueError(f"unsupported Raw lexicon version {version}")
        entries_start = HEADER_BYTES + (term_count + 1) * 8
        if entries_start > len(data):
            raise ValueError("truncated lexicon offset table")
        last_offset = struct.unpack_from("<Q", data, HEADER_BYTES + term_count * 8)[0]
        if entries_start + last_offset != len(data):
            raise ValueError("lexicon final offset does not match file length")
        text_df = []
        title_df = []
        prior = 0
        for index in range(term_count):
            relative = struct.unpack_from("<Q", data, HEADER_BYTES + index * 8)[0]
            next_relative = struct.unpack_from("<Q", data, HEADER_BYTES + (index + 1) * 8)[0]
            if relative != prior or next_relative < relative or next_relative > last_offset:
                raise ValueError(f"invalid lexicon offsets at entry {index}")
            start = entries_start + relative
            end = entries_start + next_relative
            if end - start < 2 + ENTRY_FIELDS_BYTES:
                raise ValueError(f"truncated lexicon entry {index}")
            term_bytes = struct.unpack_from("<H", data, start)[0]
            fields = start + 2 + term_bytes
            if fields + ENTRY_FIELDS_BYTES != end:
                raise ValueError(f"invalid lexicon entry length {index}")
            # Validate the format without retaining or reporting search terms.
            data[start + 2 : fields].decode("utf-8")
            text_frequency = struct.unpack_from("<I", data, fields + 12)[0]
            title_frequency = struct.unpack_from("<I", data, fields + 28)[0]
            if text_frequency > documents or title_frequency > videos:
                raise ValueError(f"out-of-universe frequency at entry {index}")
            text_df.append(text_frequency)
            title_df.append(title_frequency)
            prior = next_relative
        return {
            "lexicon_version": version,
            "term_count": term_count,
            "text": summarize(text_df, documents),
            "title": summarize(title_df, videos),
        }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lexicon", type=Path)
    parser.add_argument("--documents", type=int, required=True)
    parser.add_argument("--videos", type=int, required=True)
    parser.add_argument("--generation", type=int, required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--expect-sha256", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.documents <= 0 or args.videos <= 0 or args.generation < 0:
        parser.error("documents and videos must be positive; generation must be nonnegative")
    if args.output.exists():
        parser.error("refusing to overwrite existing output")
    digest = hashlib.sha256(args.lexicon.read_bytes()).hexdigest()
    if digest != args.expect_sha256:
        parser.error("lexicon SHA-256 differs from expected remote digest")
    if len(args.manifest_sha256) != 64 or any(
        digit not in "0123456789abcdef" for digit in args.manifest_sha256
    ):
        parser.error("manifest SHA-256 must be lowercase hexadecimal")
    if len(args.source_commit) != 40 or any(
        digit not in "0123456789abcdef" for digit in args.source_commit
    ):
        parser.error("source commit must be a full lowercase Git SHA")
    report = {
        "schema_version": 1,
        "analyzer_source_commit": args.source_commit,
        "source": {
            "generation": args.generation,
            "manifest_sha256": args.manifest_sha256,
            "lexicon_sha256": digest,
            "lexicon_bytes": args.lexicon.stat().st_size,
            "documents": args.documents,
            "videos": args.videos,
        },
        "analysis": parse_lexicon(args.lexicon, args.documents, args.videos),
        "limits": [
            "Schema-6 active lexicon frequencies; schema-9 format and bitmap choice remain open.",
            "No term bytes or posting payloads are exported.",
            "Bitmap capacity is payload only; directories, allocation, and query concurrency are excluded.",
        ],
    }
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
