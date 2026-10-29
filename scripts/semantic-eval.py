#!/usr/bin/env python3
"""Compare lexical, semantic and hybrid (RRF) ranking on a judged query set.

  semantic-eval.py [--set dev|heldout] [--base URL] fetch OUT.json   query, save raw rankings
  semantic-eval.py [--set dev|heldout] pool  OUT.json   print pooled topic results for grading
  semantic-eval.py [--set dev|heldout] score OUT.json   print metrics (needs topic grades for nDCG)
  semantic-eval.py [--set dev|heldout] judge OUT.json GRADES.tsv   grade the topic pool with a Workers AI model
  semantic-eval.py [--grades GRADES.tsv] score OUT.json  score against other topic grades
  semantic-eval.py rewrite QUERY...                     show the question rewrite

`--base` is one API that answers both `mode=lexical` and `mode=semantic`: the
local stack in docs/playbooks/semantic-pilot.md, or the experimental site. The
first dev run (2026-10-17) used production for lexical, restricted to the corpus.

Hybrids are fused offline from the two rankings with weighted reciprocal rank
fusion (RRF, k=60): `rrf` weights both 1:1, `rrf_sem2` weights semantic 2:1, and
`gated` weights lexical 3:1 when lexical's top video matched at least 75% of the
query's content words (counted from its highlights, which include fuzzy matches)
and semantic 3:1 otherwise. These weights and the threshold were fixed before
scoring; do not tune them on this same query set.

Question rewriting (`rewrite`) turns a question-shaped query into a keyword
query for lexical search: it drops question words, auxiliaries, pronouns, asking
verbs and filler, keeps quoted phrases, and leaves non-questions untouched.
`lexical_rw` and `gated_rw` use the rewritten lexical query; `question_routed`
fuses it semantic-led (1:3) for questions and is `gated` otherwise. The rules
were fixed before the held-out set was run. youtube_api implements the same
rewrite; its tests compare against this one.

`judge` grades the same pool `pool` prints with JUDGE_MODEL on Workers AI and
writes a grades file in the hand-graded format, so new query sets can be scored
without grading by hand; it also prints its agreement with the hand grades for
the queries both cover. It needs CLOUDFLARE_ACCOUNT_ID and either
CLOUDFLARE_API_TOKEN (Workers AI read) or a `wrangler login` session. Against
the dev hand grades it agreed exactly on 66% of 120 results and within one grade
on 98%, grading more leniently; it kept lexical (~0.5 nDCG@5) far below semantic
and hybrid (~0.8) but reordered engines within 0.08 of each other. Use it to
catch large gains and regressions on new query sets, not to separate close
engines. A run costs about one neuron per query, inside the free allocation.

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
import os
import urllib.error
import urllib.parse
import urllib.request

HERE = pathlib.Path(__file__).resolve().parents[1] / "docs/experiments/semantic"
SETS = {
    "dev": (HERE / "queries-2026-10-17.tsv", HERE / "targets-2026-10-17.json"),
    "heldout": (HERE / "heldout-queries-2026-10-18.tsv", HERE / "heldout-targets-2026-10-18.json"),
}
QUERIES, TARGETS = SETS["dev"]
CORPUS, GRADES = HERE / "corpus-2026-10-17.txt", HERE / "grades-2026-10-17.tsv"
BASE = "http://127.0.0.1:18787/api/search"
CREATOR, RRF_K, SLACK_MS, POOL_DEPTH = "@t3dotgg", 60, 30_000, 5
KNOWN_ITEM = ["quote", "paraphrase", "question", "typo", "vague"]
ENGINES = ["lexical", "semantic", "rrf", "rrf_sem2", "gated", "lexical_rw", "gated_rw", "question_routed",
           "hybrid"]
GATE_COVERAGE = 0.75
JUDGE_MODEL = "@cf/google/gemma-4-26b-a4b-it"
JUDGE_RUBRIC = (
    "You grade search results for a YouTube transcript search engine. For the query, grade each result: "
    "2 = on topic (answers the query or is centrally about it), 1 = partly relevant (related, or mentions it "
    "meaningfully), 0 = off-topic or only an accidental word match. Judge from the title and the transcript "
    'snippets. Reply with only JSON: {"grades": {"<id>": <0|1|2>, ...}} covering every id.')
STOPWORDS = set("""a an and are as at be but by can do does did for from had has have how i if in
into is it its it's me my no not of on or our so than that the their them then there these they this
to too up us was we were what when where which who why will with would you your""".split())
QUESTION_WORDS = set("what whats what's why how hows how's when where who whose whom which".split())
AUXILIARIES = set("""do does did is are was were am be been being can could should would will shall may
might must has have had don't doesn't didn't isn't aren't wasn't weren't can't couldn't won't wouldn't
shouldn't""".split())
# Words a person uses to ask about a video that its transcript will not contain.
REWRITE_DROP = STOPWORDS | QUESTION_WORDS | AUXILIARIES | set("""theo theos he his him video videos
talk talks talked talking say says said saying think thinks thought opinion take explain explains
explained mean means meant tell tells told about really actually please someone something""".split())


def load_queries():
    rows = []
    for line in QUERIES.read_text().splitlines():
        if line and not line.startswith("#"):
            category, target, query = line.split("\t")
            rows.append({"category": category, "target": None if target == "-" else target, "query": query})
    return rows


def is_question(query):
    words = query.lower().replace("\u2019", "'").split()
    return bool(words) and (words[0] in QUESTION_WORDS or words[0] in AUXILIARIES or query.rstrip().endswith("?"))


def rewrite(query):
    """Keyword query for lexical search; non-questions are returned unchanged."""
    if not is_question(query):
        return query
    kept = []
    for token in re.findall(r'"[^"]+"|\S+', query.replace("\u2019", "'")):
        if token.startswith('"'):
            kept.append(token)
            continue
        word = token.strip("?!.,;:()[]")
        if word.lower().endswith("'s"):
            word = word[:-2]
        if word and word.lower() not in REWRITE_DROP:
            kept.append(word)
    return " ".join(kept) or query


def search(base, query, mode):
    params = {"q": query, "youtuber": CREATOR, "per_page": 100, "sort": "relevance"}
    if mode:
        params["mode"] = mode
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


def fetch(out, base, delay):
    results = []
    queries = load_queries()
    for i, q in enumerate(queries):
        rewritten = rewrite(q["query"])
        row = dict(q, rewritten=rewritten, lexical=search(base, q["query"], None),
                   semantic=search(base, q["query"], "semantic"), hybrid=search(base, q["query"], "hybrid"))
        row["lexical_rw"] = row["lexical"] if rewritten == q["query"] else search(base, rewritten, None)
        results.append(row)
        print(f"{i + 1}/{len(queries)} {q['category']}: {q['query'][:60]} "
              f"(semantic mode={row['semantic']['mode']})", file=sys.stderr)
        time.sleep(delay)  # stay under the API's per-IP search limit
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


def gated(query, lexical, semantic):
    confident = lexical_coverage(query, lexical) >= GATE_COVERAGE
    return fuse(lexical, semantic, 3, 1) if confident else fuse(lexical, semantic, 1, 3)


def rankings(row, corpus):
    lexical = [v for v in row["lexical"]["videos"] if v["video_id"] in corpus]
    semantic = [v for v in row["semantic"]["videos"] if v["video_id"] in corpus]
    rewritten = row.get("rewritten", row["query"])
    lexical_rw = [v for v in row.get("lexical_rw", row["lexical"])["videos"] if v["video_id"] in corpus]
    return {"lexical": lexical, "semantic": semantic,
            "rrf": fuse(lexical, semantic, 1, 1),
            "rrf_sem2": fuse(lexical, semantic, 1, 2),
            "gated": gated(row["query"], lexical, semantic),
            "lexical_rw": lexical_rw,
            "gated_rw": gated(rewritten, lexical_rw, semantic),
            "question_routed": fuse(lexical_rw, semantic, 1, 3) if is_question(row["query"])
            else gated(row["query"], lexical, semantic),
            # youtube_api's mode=hybrid as served; it should equal gated_rw.
            "hybrid": [v for v in row.get("hybrid", {"videos": []})["videos"] if v["video_id"] in corpus]}


def rank_of(ranked, video_id):
    return next((i for i, v in enumerate(ranked, 1) if v["video_id"] == video_id), None)


def moment_hit(video, target):
    lo, hi = target["t_start_ms"] - SLACK_MS, target["t_end_ms"] + SLACK_MS
    return any(h["t_start_ms"] <= hi and h["t_end_ms"] >= lo for h in video["hits"])


def pooled(out):
    """(query, [(video_id, title, snippets)]) for each topic query: every engine's top 5."""
    corpus = set(CORPUS.read_text().split())
    for row in json.loads(out.read_text()):
        if row["category"] != "topic":
            continue
        ranked = rankings(row, corpus)
        seen = {}
        for engine in ENGINES:
            for video in ranked[engine][:POOL_DEPTH]:
                seen.setdefault(video["video_id"], video)
        yield row["query"], [(vid, v["title"][:80], " | ".join(h["text"][:150] for h in v["hits"][:2]))
                             for vid, v in seen.items()]


def pool(out):
    for query, videos in pooled(out):
        print(f"\n### {query}")
        for vid, title, snippets in videos:
            print(f"{vid}\t{title}\t{snippets}")


def read_grades(path):
    grades = {}
    for line in path.read_text().splitlines() if path.exists() else []:
        if line and not line.startswith("#"):
            query, video_id, grade = line.split("\t")
            grades[(query, video_id)] = int(grade)
    return grades


def parse_judgement(text, count):
    """Grades 0-2 for ids 0..count-1 from a judge reply; models wrap the JSON in fences and
    write keys as "0" or "[0]". None unless every id is graded exactly once."""
    match = re.search(r"\{.*\}", text or "", re.S)
    try:
        graded = json.loads(match.group(0))["grades"] if match else {}
    except (ValueError, KeyError, TypeError):
        return None
    grades = {}
    for key, grade in graded.items() if isinstance(graded, dict) else []:
        digits = re.sub(r"\D", "", str(key))
        if digits and int(digits) < count and grade in (0, 1, 2) and int(digits) not in grades:
            grades[int(digits)] = grade
    return [grades[i] for i in range(count)] if len(grades) == count else None


def workers_ai(model, body):
    account = os.environ.get("CLOUDFLARE_ACCOUNT_ID")
    token = os.environ.get("CLOUDFLARE_API_TOKEN")
    if not token:
        config = pathlib.Path.home() / ".config/.wrangler/config/default.toml"
        found = re.search(r'^oauth_token\s*=\s*"([^"]+)"', config.read_text(), re.M) if config.exists() else None
        token = found and found.group(1)
    if not account or not token:
        sys.exit("judge needs CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN or a wrangler login")
    request = urllib.request.Request(
        f"https://api.cloudflare.com/client/v4/accounts/{account}/ai/run/{model}",
        data=json.dumps(body).encode(), method="POST",
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
    for attempt in range(3):
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                return json.load(response)["result"]
        except urllib.error.HTTPError as error:
            if error.code == 401:
                sys.exit("Workers AI rejected the credentials; for a wrangler login run `bunx wrangler whoami`")
            if error.code not in (429, 500, 502, 503) or attempt == 2:
                raise
            time.sleep(2 * (attempt + 1))


def judge(out, grades_out):
    hand = read_grades(GRADES)
    lines, pairs, neurons = [f"# query\tvideo_id\tgrade (0 off-topic, 1 partly relevant, 2 on topic); judged by {JUDGE_MODEL}"
                             " from title + top snippets"], [], 0.0
    for query, videos in pooled(out):
        prompt = "\n".join([f"Query: {query}"] + [f"[{i}] title: {title} - snippets: {snippets}"
                                                    for i, (_, title, snippets) in enumerate(videos)])
        for _ in range(2):
            result = workers_ai(JUDGE_MODEL, {
                "messages": [{"role": "system", "content": JUDGE_RUBRIC}, {"role": "user", "content": prompt}],
                "max_tokens": 1500, "temperature": 0, "chat_template_kwargs": {"enable_thinking": False}})
            neurons += (result.get("usage") or {}).get("neurons") or 0
            grades = parse_judgement(result["choices"][0]["message"].get("content"), len(videos))
            if grades:
                break
        else:
            sys.exit(f"judge reply for {query!r} did not grade all {len(videos)} results twice; nothing written")
        for (vid, _, _), grade in zip(videos, grades):
            lines.append(f"{query}\t{vid}\t{grade}")
            if (query, vid) in hand:
                pairs.append((hand[(query, vid)], grade))
    pathlib.Path(grades_out).write_text("\n".join(lines) + "\n")
    print(f"wrote {len(lines) - 1} grades to {grades_out} ({neurons:.0f} neurons)")
    if pairs:
        exact = sum(h == j for h, j in pairs) / len(pairs)
        within = sum(abs(h - j) <= 1 for h, j in pairs) / len(pairs)
        print(f"agreement with {GRADES.name} on {len(pairs)} results: exact {exact:.2f}, within one grade {within:.2f}")


def dcg(grades):
    return sum((2 ** g - 1) / math.log2(i + 2) for i, g in enumerate(grades))


def score(out):
    corpus = set(CORPUS.read_text().split())
    targets = {t["tid"]: t for t in json.loads(TARGETS.read_text())}
    grades = read_grades(GRADES)
    rows = json.loads(out.read_text())
    report = {"known_item": {}, "topic": {}, "latency_ms": {}, "fallbacks": 0}
    for category in KNOWN_ITEM:
        cat_rows = [r for r in rows if r["category"] == category]
        if not cat_rows:
            continue
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
    for engine in ENGINES if topic_rows else []:
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
    for engine in [e for e in ("lexical", "semantic", "hybrid") if all(e in r for r in rows)]:
        server = sorted(r[engine]["server_ms"] for r in rows)
        wall = sorted(r[engine]["wall_ms"] for r in rows)
        report["latency_ms"][engine] = {"server_p50": server[len(server) // 2],
                                        "server_p95": server[int(len(server) * 0.95)],
                                        "wall_p50": round(wall[len(wall) // 2]),
                                        "wall_p95": round(wall[int(len(wall) * 0.95)])}
    report["fallbacks"] = sum(r["semantic"]["mode"] != "semantic" for r in rows)
    print(json.dumps(report, indent=1))


def main():
    global QUERIES, TARGETS, GRADES
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--set", choices=sorted(SETS), default="dev")
    parser.add_argument("--base", default=BASE)
    parser.add_argument("--delay", type=float, default=1.2)
    parser.add_argument("--grades", type=pathlib.Path, default=GRADES, help="topic grades read by score")
    parser.add_argument("command", choices=["fetch", "pool", "score", "judge", "rewrite"])
    parser.add_argument("args", nargs="+")
    args = parser.parse_args()
    QUERIES, TARGETS = SETS[args.set]
    if args.command == "judge":
        if len(args.args) != 2:
            parser.error("judge needs OUT.json GRADES.tsv")
        return judge(pathlib.Path(args.args[0]), args.args[1])
    GRADES = args.grades
    if args.command == "rewrite":
        for query in args.args:
            print(f"{query}\t{rewrite(query)}")
        return
    out = pathlib.Path(args.args[0])
    if args.command == "fetch":
        fetch(out, args.base, args.delay)
    else:
        {"pool": pool, "score": score}[args.command](out)


if __name__ == "__main__":
    main()
