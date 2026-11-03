#!/usr/bin/env python3
"""Move already-indexed videos to a creator's current handle after a YouTube rename.

Raw filters on each chunk's handle and groups creators by the ID derived from it,
so a rename leaves old videos under the old handle. This replaces those videos
with identical chunks whose handle, uploader and creator ID are the new ones, then
removes the old creator record once nothing is filed under it. Run it with one
publisher active; the next publication makes the change visible.

The chunks come from a copy of the reader's mutation database, which needs no
credential to read (`flyctl ssh sftp get /data/mutable/mutations.sqlite3`). Writes
use RAW_SEARCH_API_URL and RAW_SEARCH_ADMIN_API_KEY from the environment.
It prints the plan and changes nothing without --apply.

usage: rehome-raw-creator.py MUTATIONS.sqlite3 --from @old --to @new [--apply]
"""
import argparse
import json
import os
import re
import sqlite3
import urllib.error
import urllib.parse
import urllib.request

PUBLICATION_API_VERSION = 1
ADMIN_KEY_HEADER = "x-raw-admin-key"


def stable_id(handle):
    """Mirror of youtube_ingest::stable_youtuber_id for a nonempty handle."""
    return re.sub(r"[^a-z0-9]+", "-", handle.removeprefix("@").lower()).strip("-")


def rehomed(chunks, old, new):
    """The chunks moved to the new handle, or None when they already belong there."""
    new_id = stable_id(new)
    first = chunks[0]
    claimed = first["youtuber_handle"].lower() == old.lower() or stable_id(
        first["youtuber_id"]
    ) == stable_id(old)
    if not claimed or (first["youtuber_handle"] == new and first["youtuber_id"] == new_id):
        return None
    return [
        {**chunk, "youtuber_handle": new, "uploader_id": new, "youtuber_id": new_id}
        for chunk in chunks
    ]


def plan(database, old, new):
    connection = sqlite3.connect(f"file:{database}?mode=ro&immutable=1", uri=True)
    rows = connection.execute(
        "SELECT video_id, chunks_json FROM video_mutations WHERE chunks_json IS NOT NULL"
    )
    for video_id, blob in rows:
        replacement = rehomed(json.loads(bytes(blob)), old, new)
        if replacement:
            yield video_id, replacement


def admin(method, path, body=None):
    url = os.environ["RAW_SEARCH_API_URL"].rstrip("/") + path
    data = None if body is None else json.dumps(body, ensure_ascii=False).encode()
    request = urllib.request.Request(url, data=data, method=method)
    request.add_header(ADMIN_KEY_HEADER, os.environ["RAW_SEARCH_ADMIN_API_KEY"])
    request.add_header("content-type", "application/json")
    with urllib.request.urlopen(request, timeout=120) as response:
        raw = response.read()
    return json.loads(raw) if raw else None


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("database")
    parser.add_argument("--from", dest="old", required=True)
    parser.add_argument("--to", dest="new", required=True)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    moves = list(plan(args.database, args.old, args.new))
    chunk_total = sum(len(chunks) for _, chunks in moves)
    print(json.dumps({"videos": len(moves), "chunks": chunk_total, "applied": args.apply}))
    if not args.apply:
        return
    key_prefix = f"rehome-{stable_id(args.old)}-{stable_id(args.new)}"
    for video_id, chunks in moves:
        before = admin("GET", f"/admin/videos/{urllib.parse.quote(video_id)}/chunk-count")
        admin(
            "PUT",
            f"/admin/videos/{urllib.parse.quote(video_id)}",
            {
                "publication_api_version": PUBLICATION_API_VERSION,
                "idempotency_key": f"{key_prefix}-{video_id}",
                "chunks": chunks,
            },
        )
        after = admin("GET", f"/admin/videos/{urllib.parse.quote(video_id)}/chunk-count")
        if before != after or after.get("chunk_count", len(chunks)) != len(chunks):
            raise SystemExit(f"{video_id}: chunk count changed from {before} to {after}")
    old_id, new_id = stable_id(args.old), stable_id(args.new)
    if old_id == new_id:
        return
    stats = admin("GET", f"/admin/channels/{urllib.parse.quote(old_id)}/stats")
    if stats["indexed_videos"] or stats["indexed_chunks"]:
        raise SystemExit(f"{old_id} still holds {stats}; keeping its creator record")
    query = urllib.parse.urlencode(
        {
            "publication_api_version": PUBLICATION_API_VERSION,
            "idempotency_key": f"{key_prefix}-channel",
            "cascade": "false",
        }
    )
    admin("DELETE", f"/admin/channels/{urllib.parse.quote(old_id)}?{query}")
    print(json.dumps({"removed_creator": old_id}))


if __name__ == "__main__":
    try:
        main()
    except urllib.error.HTTPError as error:
        raise SystemExit(f"Raw admin answered HTTP {error.code}") from None
