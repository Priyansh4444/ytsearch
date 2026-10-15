#!/usr/bin/env python3
"""Exercise real local Gemma -> Qdrant -> API, using disposable synthetic data."""

import argparse
import collections
import hashlib
import http.server
import json
import os
import pathlib
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request


ROOT = pathlib.Path(__file__).resolve().parents[1]


def free_port():
    with socket.socket() as connection:
        connection.bind(("127.0.0.1", 0))
        return connection.getsockname()[1]


def request(url, body=None):
    data = None if body is None else json.dumps(body).encode()
    query = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(query, timeout=40) as response:
        content = response.read()
        return json.loads(content) if content else None


def snapshot(path, rows):
    content = "".join(json.dumps(row) + "\n" for row in rows).encode()
    path.write_bytes(content)
    receipt = {
        "schema": 1,
        "sha256": hashlib.sha256(content).hexdigest(),
        "complete_videos": dict(collections.Counter(row["video_id"] for row in rows)),
    }
    pathlib.Path(str(path) + ".manifest.json").write_text(json.dumps(receipt))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=pathlib.Path, default=ROOT / "data/semantic")
    parser.add_argument("--binary-dir", type=pathlib.Path, default=ROOT / "target/debug")
    args = parser.parse_args()
    assets = args.assets.resolve()
    binary = args.binary_dir.resolve() / "youtube-semantic"
    api_binary = args.binary_dir.resolve() / "youtube-api"
    # Deliberately do not inherit service credentials or a repository .env.
    clean_env = {key: os.environ[key] for key in ["PATH", "HOME"] if key in os.environ}
    processes, logs = [], []
    raw_requests = []

    class RawFixture(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            raw_requests.append(self.path)
            body = json.dumps({"hits": [], "found": 0, "out_of": 0, "search_time_ms": 0}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    raw = http.server.ThreadingHTTPServer(("127.0.0.1", 0), RawFixture)
    threading.Thread(target=raw.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="ytsearch-semantic-smoke-") as work:
        work = pathlib.Path(work)

        def launch(name, command, environment):
            log = (work / (name + ".log")).open("w+")
            logs.append(log)
            process = subprocess.Popen(command, env=environment, cwd=work, stdout=log, stderr=log)
            processes.append(process)
            return process

        def ready(url, process):
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline and process.poll() is None:
                try:
                    request(url)
                    return
                except (OSError, urllib.error.HTTPError):
                    time.sleep(0.05)
            raise RuntimeError("local smoke service did not become ready")

        try:
            qport, gport, sport, aport = [free_port() for _ in range(4)]
            qbase, service, api = [f"http://127.0.0.1:{port}" for port in [qport, sport, aport]]
            qenv = dict(clean_env, QDRANT__SERVICE__HTTP_PORT=str(qport),
                        QDRANT__SERVICE__GRPC_PORT=str(gport),
                        QDRANT__STORAGE__STORAGE_PATH=str(work / "qdrant"),
                        QDRANT__STORAGE__SNAPSHOTS_PATH=str(work / "snapshots"))
            qdrant = launch("qdrant", [str(assets / "qdrant/qdrant"), "--config-path",
                                      str(ROOT / "config/qdrant-semantic.local.yaml")], qenv)
            ready(qbase + "/collections", qdrant)
            collection = "ytsearch_semantic_pilot_smoke"
            env = dict(clean_env, SEMANTIC_MODEL_DIR=str(assets / "model"),
                       ORT_DYLIB_PATH=str(assets / "runtime/onnxruntime-linux-x64-1.31.0/lib/libonnxruntime.so.1.31.0"),
                       SEMANTIC_QDRANT_URL=qbase, SEMANTIC_COLLECTION=collection,
                       SEMANTIC_BIND=f"127.0.0.1:{sport}")
            source = work / "snapshot.jsonl"
            rows = [
                {"id": "a", "video_id": "astronomy", "title": "Black holes", "text": "A collapsed star forms a black hole. Light cannot escape its event horizon.", "youtuber_handle": "@Space", "youtuber_name": "Space", "t_start_ms": 0, "t_end_ms": 30000},
                {"id": "b", "video_id": "astronomy", "title": "Black holes", "text": "Hawking radiation can make black holes slowly evaporate.", "youtuber_handle": "@Space", "youtuber_name": "Space", "t_start_ms": 70000, "t_end_ms": 90000},
                {"id": "c", "video_id": "typescript", "title": "Compiler checks", "text": "TypeScript detects type errors before JavaScript executes.", "youtuber_handle": "@Code", "youtuber_name": "Code", "t_start_ms": 0, "t_end_ms": 30000},
            ]
            snapshot(source, rows)

            def index():
                result = subprocess.run([str(binary), "index", str(source)], env=env, cwd=work,
                                        capture_output=True, text=True, timeout=120, check=True)
                return json.loads(result.stdout)

            initial, repeat = index(), index()
            assert initial["embedded_passages"] == 3 and repeat["embedded_passages"] == 0
            server = launch("semantic", [str(binary), "serve"], env)
            ready(service + "/ready", server)
            response = request(service + "/search", {"q": "Why can light not escape a collapsed star?", "limit": 10})
            assert response["hits"][0]["document"]["video_id"] == "astronomy"
            filtered = request(service + "/search", {"q": "collapsed star", "filter": {"creator": "@SPACE", "video_id": "astronomy"}, "limit": 10})
            assert len(filtered["hits"]) == 2
            assert all(hit["document"]["youtuber_handle"] == "@space" for hit in filtered["hits"])
            assert not request(service + "/search", {"q": "star", "filter": {"creator": "@absent"}, "limit": 10})["hits"]
            apienv = dict(clean_env, RAW_SEARCH_API_URL=f"http://127.0.0.1:{raw.server_port}",
                          RAW_SEARCH_API_KEY="local-fixture-only", YTSEARCH_API_ADDR=f"127.0.0.1:{aport}",
                          SEMANTIC_SEARCH_URL=service, YTSEARCH_INDEX_QUEUE_PATH=str(work / "queue.jsonl"))
            api_process = launch("api", [str(api_binary)], apienv)
            ready(api + "/api/health", api_process)
            def search(params):
                return request(api + "/api/search?" + urllib.parse.urlencode(params))
            cards = search({"q": "collapsed star", "mode": "semantic", "youtuber": "@SPACE"})
            assert cards["mode"] == "semantic" and cards["videos"][0]["video_id"] == "astronomy"
            assert cards["videos"][0]["hits"][0]["text"] and cards["totals_approximate"]
            low = search({"q": "I", "mode": "semantic", "youtuber": "@t3dotgg"})
            assert low["mode"] == "lexical" and low["sort"] == "published_desc"
            assert search({"q": "fable on high", "youtuber": "@t3dotgg"})["mode"] == "lexical"
            assert search({"q": "stars", "mode": "semantic", "sort": "published_asc"})["mode"] == "lexical"
            # Keep the original receipt while truncating at a valid line boundary.
            source.write_text("".join(json.dumps(row) + "\n" for row in rows[:-1]))
            truncated = subprocess.run([str(binary), "index", str(source)], env=env, cwd=work,
                                       capture_output=True, text=True, timeout=30)
            assert truncated.returncode and "receipt mismatch" in truncated.stderr
            snapshot(source, [rows[0], rows[2]])
            assert index()["embedded_passages"] == 0
            assert index()["embedded_passages"] == 0
            remaining = request(service + "/search", {"q": "star", "filter": {"video_id": "astronomy"}, "limit": 10})["hits"]
            assert len(remaining) == 1 and remaining[0]["document"]["t_end_ms"] == 30000
            params = request(qbase + "/collections/" + collection)["result"]["config"]["params"]
            assert params["vectors"]["datatype"] == "float16" and params["vectors"]["on_disk"]
            assert params["on_disk_payload"]
            print(json.dumps({"ok": True, "initial_passages": 3, "repeat_embedded": 0,
                              "embedding_ms": response["timings"]["embedding_ms"],
                              "checks": ["real inference", "creator/video filters", "mixed-case handles", "API cards", "lexical routes", "truncation rejection", "shrink cleanup", "on-disk float16 storage"],
                              "raw_fixture_requests": len(raw_requests)}))
        finally:
            for process in reversed(processes):
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            raw.shutdown()
            for log in logs:
                log.close()


if __name__ == "__main__":
    main()
