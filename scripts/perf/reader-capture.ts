// Records the reader requests the API makes for every benchmark case, sort
// and page. Called by reader-capture-requests.sh, which starts the reader.
//
//   bun reader-capture.ts <cases.tsv> <api-binary> <reader-port> <output>
//
// A logging proxy sits between a real API process and the reader, so the
// captured URLs are exactly what production's request construction emits.
/* oxlint-disable no-console -- a CLI whose output is its result. */
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { createServer, request as httpRequest } from "node:http";
import { setTimeout as sleep } from "node:timers/promises";

import { searchCaseParams } from "./search-cases";

const [casesFile, apiBinary, readerPort, output] = process.argv.slice(2);
if (!casesFile || !apiBinary || !readerPort || !output) {
  console.error("usage: bun reader-capture.ts <cases.tsv> <api-binary> <reader-port> <output>");
  process.exit(2);
}
const PROXY_PORT = 18131;
const API_PORT = 18132;

const captured = new Set<string>();
const proxy = createServer((incoming, outgoing) => {
  const { pathname, search } = new URL(incoming.url ?? "/", "http://reader");
  // The API asks the reader to cache; replays must measure uncached work.
  const recorded = new URLSearchParams(search);
  recorded.delete("use_cache");
  recorded.delete("cache_ttl");
  captured.add(`${pathname}?${recorded}`);
  const forwarded = httpRequest(
    {
      host: "::1",
      port: Number(readerPort),
      path: incoming.url,
      method: incoming.method,
      headers: incoming.headers,
    },
    (response) => {
      outgoing.writeHead(response.statusCode ?? 502, response.headers);
      response.pipe(outgoing);
    },
  );
  incoming.pipe(forwarded);
});
await new Promise<void>((resolve) => proxy.listen(PROXY_PORT, "::1", resolve));

const api = spawn(apiBinary, [], {
  env: {
    ...process.env,
    RAW_SEARCH_API_URL: `http://[::1]:${PROXY_PORT}`,
    RAW_SEARCH_API_KEY: "perf-local",
    YTSEARCH_API_ADDR: `127.0.0.1:${API_PORT}`,
    YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE: "0",
    YTSEARCH_ENGAGEMENT_SORTS: "true",
  },
  stdio: "ignore",
});

try {
  const deadline = Date.now() + 60_000;
  for (;;) {
    const healthy = await fetch(`http://127.0.0.1:${API_PORT}/api/health`).then(
      (response) => response.ok,
      () => false,
    );
    if (healthy) break;
    if (Date.now() > deadline) throw new Error("API did not become healthy");
    await sleep(300);
  }

  for (const params of searchCaseParams(casesFile)) {
    const response = await fetch(`http://127.0.0.1:${API_PORT}/api/search?${params}`);
    if (!response.ok) {
      throw new Error(`${response.status} for ${params}: ${await response.text()}`);
    }
    await response.arrayBuffer();
  }
  writeFileSync(output, [...captured].sort().join("\n") + "\n");
  console.log(`captured ${captured.size} reader requests -> ${output}`);
} finally {
  api.kill();
  proxy.close();
}
