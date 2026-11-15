#!/usr/bin/env python3
"""Build semantic-pilot import snapshots from a Raw chunk backup archive.

Reads youtube_transcript_chunks.jsonl.gz once, keeps the chosen creators, and
writes snapshot-NNN.jsonl files (at most 128 complete videos, sorted by video_id
then timestamp) each with the SNAPSHOT.jsonl.manifest.json receipt the importer
requires. Videos the pilot importer would reject are skipped and counted.
OUT_DIR must be a fresh path; existing directories are never reused.

usage: build-semantic-snapshots.py ARCHIVE OUT_DIR --creator @handle [...]
       [--newest N]        keep only each creator's N most recently published videos
       [--mutations DB]    a copy of the reader's mutations.sqlite3: its videos
                           replace or delete the archive's, so the snapshots match
                           the published corpus rather than the archive's date
       [--rename @old=@new] file a renamed creator's videos under the new handle
"""
import argparse
import collections
import gzip
import hashlib
import json
import pathlib
import re
import sqlite3

MAX_VIDEOS, MAX_SNAPSHOT_BYTES, MAX_LINE_BYTES = 128, 60 * 1024 * 1024, 64 * 1024
MAX_VIDEO_CHUNKS, MAX_VIDEO_BYTES, MAX_CHUNK_MS = 4096, 4 * 1024 * 1024, 60_000
# A chunk this long can exceed the 512-token cap once the title prompt is added.
MAX_CHUNK_CHARS = 1600
DISPLAY = ["thumbnail", "published_at", "view_count", "like_count", "comment_count", "duration"]


def video_lines(rows):
    """Return the video's JSONL lines, or the reason the importer would reject it."""
    rows.sort(key=lambda r: (r["t_start_ms"], r["t_end_ms"]))
    # Engagement counts were backfilled onto only some chunks of a video, but the
    # importer requires one display value per video: use the first non-null one.
    for key in DISPLAY:
        value = next((r[key] for r in rows if r.get(key) is not None), None)
        for r in rows:
            if value is None:
                r.pop(key, None)
            else:
                r[key] = value
    # Channel renames leave old creator names on older chunks; the importer wants one.
    for r in rows:
        r["youtuber_name"] = rows[-1]["youtuber_name"]
    if any(r["title"] != rows[0]["title"] for r in rows):
        return None, "inconsistent_title"
    if any(r["youtuber_handle"].lower() != rows[0]["youtuber_handle"].lower() for r in rows):
        return None, "inconsistent_handle"
    if any(not 0 < r["t_end_ms"] - r["t_start_ms"] <= MAX_CHUNK_MS for r in rows):
        return None, "bad_time_range"
    if any(not r["text"].strip() or len(r["text"]) > MAX_CHUNK_CHARS for r in rows):
        return None, "long_or_empty_chunk"
    lines = [json.dumps(r, ensure_ascii=False) + "\n" for r in rows]
    if any(len(line.encode()) > MAX_LINE_BYTES for line in lines):
        return None, "long_line"
    if len(lines) > MAX_VIDEO_CHUNKS or sum(len(line.encode()) for line in lines) > MAX_VIDEO_BYTES:
        return None, "video_too_large"
    return lines, None


def mutation_rows(database):
    """Yield (video_id, chunks or None for a deletion) from a reader mutation database."""
    connection = sqlite3.connect(f"file:{database}?mode=ro&immutable=1", uri=True)
    for video_id, blob in connection.execute("SELECT video_id, chunks_json FROM video_mutations"):
        yield video_id, json.loads(bytes(blob)) if blob is not None else None


def canonical_handle(handle, renames):
    return renames.get(handle.lower(), handle)


def filed_under(rows, renames):
    """Move a video's chunks to its creator's current handle."""
    for row in rows:
        handle = canonical_handle(row["youtuber_handle"], renames)
        if handle != row["youtuber_handle"]:
            row["youtuber_handle"] = handle
            row["uploader_id"] = handle
            row["youtuber_id"] = re.sub(r"[^a-z0-9]+", "-", handle[1:].lower()).strip("-")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("archive", type=pathlib.Path)
    parser.add_argument("out", type=pathlib.Path)
    parser.add_argument("--creator", action="append", required=True)
    parser.add_argument("--newest", type=int)
    parser.add_argument("--mutations", type=pathlib.Path)
    parser.add_argument("--rename", action="append", default=[], metavar="@OLD=@NEW")
    args = parser.parse_args()
    renames = {}
    for pair in args.rename:
        old, _, new = pair.partition("=")
        if not old or not new:
            parser.error(f"--rename needs @old=@new, got {pair!r}")
        renames[old.lower()] = new
    wanted = {canonical_handle(handle, renames).lower() for handle in args.creator}
    selected = lambda row: canonical_handle(row["youtuber_handle"], renames).lower() in wanted
    mutated = {}
    if args.mutations:
        for video_id, chunks in mutation_rows(args.mutations):
            mutated[video_id] = filed_under(chunks, renames) if chunks and selected(chunks[0]) else None
    videos = collections.defaultdict(list)
    with gzip.open(args.archive, "rt") as source:
        for line in source:
            row = json.loads(line)
            if row["video_id"] not in mutated and selected(row):
                videos[row["video_id"]].append(row)
    for rows in videos.values():
        filed_under(rows, renames)
    for video_id, chunks in mutated.items():
        if chunks:
            videos[video_id] = chunks

    skipped = collections.Counter()
    by_creator = collections.defaultdict(list)
    for video_id, rows in videos.items():
        lines, reason = video_lines(rows)
        if reason:
            skipped[reason] += 1
            continue
        by_creator[rows[0]["youtuber_handle"].lower()].append(
            (rows[0].get("published_at") or "", video_id, lines))
    keep = []
    for items in by_creator.values():
        items.sort(key=lambda item: item[0], reverse=True)
        keep.extend(items[: args.newest] if args.newest else items)
    keep.sort(key=lambda item: item[1])  # the importer requires video_id order

    try:
        args.out.mkdir(parents=True)
    except FileExistsError:
        parser.error("choose a fresh output directory; existing exports are never overwritten")
    batches, batch, batch_bytes = [], [], 0
    for item in keep:
        size = sum(len(line.encode()) for line in item[2])
        if batch and (len(batch) == MAX_VIDEOS or batch_bytes + size > MAX_SNAPSHOT_BYTES):
            batches.append(batch)
            batch, batch_bytes = [], 0
        batch.append(item)
        batch_bytes += size
    if batch:
        batches.append(batch)
    for index, batch in enumerate(batches):
        path = args.out / f"snapshot-{index:03d}.jsonl"
        data = "".join(line for _, _, lines in batch for line in lines).encode()
        path.write_bytes(data)
        receipt = {"schema": 1, "sha256": hashlib.sha256(data).hexdigest(),
                   "complete_videos": {video_id: len(lines) for _, video_id, lines in batch}}
        pathlib.Path(str(path) + ".manifest.json").write_text(json.dumps(receipt))
    print(json.dumps({"videos": len(keep), "chunks": sum(len(item[2]) for item in keep),
                      "snapshots": len(batches), "per_creator": {h: len(v) for h, v in by_creator.items()},
                      "skipped": skipped}))


if __name__ == "__main__":
    main()
