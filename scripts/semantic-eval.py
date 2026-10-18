#!/usr/bin/env python3
"""Compare lexical, semantic and hybrid (RRF) ranking on a judged query set.

  semantic-eval.py fetch    OUT.json      query both engines, save raw rankings
  semantic-eval.py pool     OUT.json      print pooled top-k results of topic queries for grading
  semantic-eval.py score    OUT.json      print metrics (needs topic grades for nDCG)

Hybrids are fused offline from the two rankings with weighted reciprocal rank
fusion (RRF, k=60): `rrf` weights both 1:1, `rrf_sem2` weights semantic 2:1, and
`gated` weights lexical 3:1 when lexical's top video matched at least 75% of the
query's content words (counted from its highlights, which include fuzzy matches)
and semantic 3:1 otherwise. These weights and the threshold were fixed before
scoring; do not tune them on this same query set.

Lexical rankings come from the production API and are restricted to the semantic
corpus (corpus-*.txt), so both engines compete over the same videos. Known-item
queries name a target moment; a moment hit means one of the top video's first
three hits overlaps the target window by +-30 s. Topic queries are graded 0/1/2
per video in grades-*.tsv. Raw output holds transcript text: keep it out of git.
"""
import argparse
import json
import math
import pathlib
import re
import statistics
import sys
import time
import urllib.parse
import urllib.request

HERE = pathlib.Path(__file__).resolve().parents[1] / "docs/experiments/semantic"
QUERIES, TARGETS = HERE / "queries-2026-10-17.tsv", HERE / "targets-2026-10-17.json"
CORPUS, GRADES = HERE / "corpus-2026-10-17.txt", HERE / "grades-2026-10-17.tsv"
LEXICAL = "https://ytsearch-api-raw-production.fly.dev/api/search"
SEMANTIC = "https://ytsearch-semantic-experimental.fly.dev/api/search"
CREATOR, RRF_K, SLACK_MS, POOL_DEPTH = "@t3dotgg", 60, 30_000, 5
KNOWN_ITEM = ["quote", "paraphrase", "question", "typo", "vague"]
ENGINES = ["lexical", "semantic", "rrf", "rrf_sem2", "gated"]
GATE_COVERAGE = 0.75
STOPWORDS = set("""a an and are as at be but by can do does did for from had has have how i if in
into is it its it's me my no not of on or our so than that the their them then there these they this
to too up us was we were what when where which who why will with would you your""".split())


def load_queries():
    rows = []
    for line in QUERIES.read_text().splitlines():
        if line and not line.startswith("#"):
            category, target, query = line.split("\t")
            rows.append({"category": category, "target": None if target == "-" else target, "query": query})
    return rows


def search(base, query, semantic):
    params = {"q": query, "youtuber": CREATOR, "per_page": 100, "sort": "relevance"}
    if semantic:
        params["mode"] = "semantic"
    url = base + "?" + urllib.parse.urlencode(params)
    for attempt in range(4):
        started = time.monotonic()
        try:
            with urllib.request.urlopen(url, timeout=60) as response:
                body = json.load(response)
            break
        except urllib.error.HTTPError as error:
            if error.code != 429 or attempt == 3:
                raise
            time.sleep(15)
    wall_ms = (time.monotonic() - started) * 1000
    videos = [{"video_id": v["video_id"], "title": v["title"],
               "hits": [{"t_start_ms": h["t_start_ms"], "t_end_ms": h["t_end_ms"], "text": h["text"],
                         "text_highlight": h.get("text_highlight")}
                        for h in v["hits"][:3]]} for v in body["videos"]]
    return {"mode": body["mode"], "server_ms": body["time_to_first_result_ms"], "wall_ms": wall_ms, "videos": videos}


def fetch(out):
    results = []
    queries = load_queries()
    for i, q in enumerate(queries):
        row = dict(q, lexical=search(LEXICAL, q["query"], False), semantic=search(SEMANTIC, q["query"], True))
        results.append(row)
        print(f"{i + 1}/{len(queries)} {q['category']}: {q['query'][:60]} "
              f"(semantic mode={row['semantic']['mode']})", file=sys.stderr)
        time.sleep(1.2)  # stay well under both APIs' per-IP search limits
    out.write_text(json.dumps(results))


def content_words(text):
    return {w for w in re.findall(r"[a-z0-9']+", text.lower()) if w not in STOPWORDS}


def lexical_coverage(query, lexical):
    """Share of the query's content words that lexical's top video matched."""
    wanted = content_words(query)
    if not lexical or not wanted:
        return 0.0
    marked = set()
    for hit in lexical[0]["hits"]:
        marked |= content_words(" ".join(re.findall(r"<mark>([^<]+)</mark>", hit["text_highlight"] or "")))
    return min(1.0, len(marked) / len(wanted))


def fuse(lexical, semantic, lexical_weight, semantic_weight):
    scores = {}
    for ranked, weight in ((lexical, lexical_weight), (semantic, semantic_weight)):
        for rank, video in enumerate(ranked, 1):
            scores[video["video_id"]] = scores.get(video["video_id"], 0.0) + weight / (RRF_K + rank)
    # Fused moments: prefer the semantic engine's hits for a video when it has any.
    by_id = {v["video_id"]: v for v in lexical}
    by_id.update({v["video_id"]: v for v in semantic})
    return [by_id[vid] for vid in sorted(scores, key=lambda vid: -scores[vid])]


def rankings(row, corpus):
    lexical = [v for v in row["lexical"]["videos"] if v["video_id"] in corpus]
    semantic = [v for v in row["semantic"]["videos"] if v["video_id"] in corpus]
    confident = lexical_coverage(row["query"], lexical) >= GATE_COVERAGE
    return {"lexical": lexical, "semantic": semantic,
            "rrf": fuse(lexical, semantic, 1, 1),
            "rrf_sem2": fuse(lexical, semantic, 1, 2),
            "gated": fuse(lexical, semantic, 3, 1) if confident else fuse(lexical, semantic, 1, 3)}


def rank_of(ranked, video_id):
    return next((i for i, v in enumerate(ranked, 1) if v["video_id"] == video_id), None)


def moment_hit(video, target):
    lo, hi = target["t_start_ms"] - SLACK_MS, target["t_end_ms"] + SLACK_MS
    return any(h["t_start_ms"] <= hi and h["t_end_ms"] >= lo for h in video["hits"])


def pool(out):
    corpus = set(CORPUS.read_text().split())
    for row in json.loads(out.read_text()):
        if row["category"] != "topic":
            continue
        ranked = rankings(row, corpus)
        print(f"\n### {row['query']}")
        seen = {}
        for engine in ENGINES:
            for video in ranked[engine][:POOL_DEPTH]:
                seen.setdefault(video["video_id"], video)
        for vid, video in seen.items():
            snippets = " | ".join(h["text"][:150] for h in video["hits"][:2])
            print(f"{vid}\t{video['title'][:80]}\t{snippets}")


def dcg(grades):
    return sum((2 ** g - 1) / math.log2(i + 2) for i, g in enumerate(grades))


def score(out):
    corpus = set(CORPUS.read_text().split())
    targets = {t["tid"]: t for t in json.loads(TARGETS.read_text())}
    grades = {}
    if GRADES.exists():
        for line in GRADES.read_text().splitlines():
            if line and not line.startswith("#"):
                query, video_id, grade = line.split("\t")
                grades[(query, video_id)] = int(grade)
    rows = json.loads(out.read_text())
    report = {"known_item": {}, "topic": {}, "latency_ms": {}, "fallbacks": 0}
    for category in KNOWN_ITEM:
        cat_rows = [r for r in rows if r["category"] == category]
        stats = {}
        for engine in ENGINES:
            rr, r1, r5, r10, m1 = [], 0, 0, 0, 0
            for row in cat_rows:
                target = targets[row["target"]]
                ranked = rankings(row, corpus)[engine]
                rank = rank_of(ranked, target["video_id"])
                rr.append(1 / rank if rank and rank <= 10 else 0)
                r1 += rank == 1
                r5 += bool(rank and rank <= 5)
                r10 += bool(rank and rank <= 10)
                m1 += rank == 1 and moment_hit(ranked[0], target)
            n = len(cat_rows)
            stats[engine] = {"n": n, "MRR@10": round(sum(rr) / n, 3), "R@1": round(r1 / n, 3),
                             "R@5": round(r5 / n, 3), "R@10": round(r10 / n, 3),
                             "moment@1": None if category == "vague" else round(m1 / n, 3)}
        report["known_item"][category] = stats
    topic_rows = [r for r in rows if r["category"] == "topic"]
    for engine in ENGINES:
        ndcgs, p5 = [], []
        for row in topic_rows:
            ranked = rankings(row, corpus)
            got = [grades.get((row["query"], v["video_id"]), 0) for v in ranked[engine][:POOL_DEPTH]]
            judged = {vid: g for (q, vid), g in grades.items() if q == row["query"]}
            ideal = sorted(judged.values(), reverse=True)[:POOL_DEPTH]
            ndcgs.append(dcg(got) / dcg(ideal) if dcg(ideal) else 0)
            p5.append(sum(g > 0 for g in got) / POOL_DEPTH)
        report["topic"][engine] = {"n": len(topic_rows), "nDCG@5": round(statistics.mean(ndcgs), 3),
                                   "P@5": round(statistics.mean(p5), 3)}
    for engine in ("lexical", "semantic"):
        server = sorted(r[engine]["server_ms"] for r in rows)
        wall = sorted(r[engine]["wall_ms"] for r in rows)
        report["latency_ms"][engine] = {"server_p50": server[len(server) // 2],
                                        "server_p95": server[int(len(server) * 0.95)],
                                        "wall_p50": round(wall[len(wall) // 2]),
                                        "wall_p95": round(wall[int(len(wall) * 0.95)])}
    report["fallbacks"] = sum(r["semantic"]["mode"] != "semantic" for r in rows)
    print(json.dumps(report, indent=1))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["fetch", "pool", "score"])
    parser.add_argument("out", type=pathlib.Path)
    args = parser.parse_args()
    {"fetch": fetch, "pool": pool, "score": score}[args.command](args.out)


if __name__ == "__main__":
    main()
