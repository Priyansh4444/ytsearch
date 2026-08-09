import { createReadStream, createWriteStream } from "node:fs";
import { appendFile, readFile, rm } from "node:fs/promises";
import { once } from "node:events";
import { createInterface } from "node:readline";
import type { Readable } from "node:stream";
import { finished } from "node:stream/promises";
import { createGunzip, createGzip } from "node:zlib";
import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";

const BATCH_SIZE = 50;
const FETCH_CONCURRENCY = 1;
const FETCH_ATTEMPTS = 8;

const VideoStatsResponseSchema = Schema.Struct({
  items: Schema.optionalKey(
    Schema.Array(
      Schema.Struct({
        id: Schema.String,
        statistics: Schema.optionalKey(
          Schema.Struct({
            viewCount: Schema.optionalKey(Schema.String),
            likeCount: Schema.optionalKey(Schema.String),
            commentCount: Schema.optionalKey(Schema.String),
          }),
        ),
      }),
    ),
  ),
});

type Engagement = {
  view_count: number;
  like_count: number;
  comment_count: number;
};

class VideoStatsHttpError extends Error {
  constructor(
    readonly status: number,
    readonly retryAfterMs: number | undefined,
    detail: string,
  ) {
    super(`YouTube statistics request returned HTTP ${status}${detail ? `: ${detail}` : ""}`);
  }
}

const decodeVideoStats = Schema.decodeUnknownEffect(VideoStatsResponseSchema);

const [inputPath, outputPath] = process.argv.slice(2);
const apiKey = process.env.YOUTUBE_API_KEY?.trim();
const videoStatsApiUrl =
  process.env.YOUTUBE_VIDEO_STATS_API_URL?.trim() ||
  "https://www.googleapis.com/youtube/v3/videos:batchGetStats";
const engagementCachePath =
  process.env.ENGAGEMENT_CACHE_PATH?.trim() || `${outputPath}.stats.jsonl`;

const program = Effect.gen(function* () {
  if (!inputPath || !outputPath) {
    return yield* Effect.fail(
      new Error("Usage: bun scripts/enrich-search-engagement.ts INPUT.jsonl OUTPUT.jsonl"),
    );
  }
  if (inputPath === outputPath) {
    return yield* Effect.fail(new Error("Input and output paths must be different"));
  }
  if (!apiKey) {
    return yield* Effect.fail(new Error("YOUTUBE_API_KEY is required"));
  }

  const videoIds = yield* collectVideoIds(inputPath);
  const engagement = yield* loadEngagementCache(engagementCachePath);
  const cachedVideos = engagement.size;
  const batches = chunk(
    videoIds.filter((videoId) => !engagement.has(videoId)),
    BATCH_SIZE,
  );
  yield* Effect.forEach(
    batches,
    (batch) =>
      Effect.gen(function* () {
        const results = yield* fetchEngagementBatch(batch, apiKey, videoStatsApiUrl);
        yield* appendEngagementCache(engagementCachePath, results);
        for (const [videoId, metrics] of results) engagement.set(videoId, metrics);
      }),
    { concurrency: FETCH_CONCURRENCY, discard: true },
  );
  const written = yield* writeEnrichedCorpus(inputPath, outputPath, engagement);

  return {
    input: inputPath,
    output: outputPath,
    cache: engagementCachePath,
    requested_videos: videoIds.length,
    cached_videos: cachedVideos,
    enriched_videos: engagement.size,
    missing_videos: videoIds.length - engagement.size,
    written_documents: written.documents,
    metric_records_written: written.metricRecords,
    api_requests: batches.length,
  };
});

const startedAt = performance.now();
let peakRssBytes = process.memoryUsage().rss;
const memorySampler = setInterval(() => {
  peakRssBytes = Math.max(peakRssBytes, process.memoryUsage().rss);
}, 100);
memorySampler.unref();
try {
  const result = await Effect.runPromise(program);
  peakRssBytes = Math.max(peakRssBytes, process.memoryUsage().rss);
  process.stdout.write(
    `${JSON.stringify(
      {
        ...result,
        elapsed_ms: Math.round(performance.now() - startedAt),
        peak_rss_bytes: peakRssBytes,
      },
      null,
      2,
    )}\n`,
  );
} finally {
  clearInterval(memorySampler);
}

function collectVideoIds(path: string): Effect.Effect<string[], Error> {
  return Effect.tryPromise({
    try: async () => {
      const ids = new Set<string>();
      const lines = createInterface({ input: corpusInput(path), crlfDelay: Infinity });
      for await (const line of lines) {
        if (!line.trim()) continue;
        const value: unknown = JSON.parse(line);
        if (!isRecord(value) || typeof value.video_id !== "string" || !value.video_id) {
          throw new Error("Corpus document is missing video_id");
        }
        ids.add(value.video_id);
      }
      return [...ids];
    },
    catch: asError,
  });
}

function fetchEngagementBatch(
  videoIds: readonly string[],
  key: string,
  apiUrl: string,
  attempt = 1,
): Effect.Effect<Array<[string, Engagement]>, Error> {
  const request = Effect.gen(function* () {
    const url = new URL(apiUrl);
    url.searchParams.set("part", "statistics");
    url.searchParams.set("id", videoIds.join(","));
    url.searchParams.set("key", key);
    const response = yield* Effect.tryPromise({
      try: (signal) => fetch(url, { signal }),
      catch: asError,
    });
    if (!response.ok) {
      const detail = yield* Effect.tryPromise({
        try: () => response.text(),
        catch: asError,
      });
      return yield* Effect.fail(
        new VideoStatsHttpError(
          response.status,
          retryAfterMilliseconds(response.headers.get("Retry-After")),
          detail.replace(/\s+/g, " ").trim().slice(0, 180),
        ),
      );
    }
    const body = yield* Effect.tryPromise({
      try: () => response.json(),
      catch: asError,
    });
    const decoded = yield* decodeVideoStats(body).pipe(Effect.mapError(asError));
    return (decoded.items ?? []).map((item): [string, Engagement] => [
      item.id,
      {
        view_count: count(item.statistics?.viewCount),
        like_count: count(item.statistics?.likeCount),
        comment_count: count(item.statistics?.commentCount),
      },
    ]);
  });

  if (attempt >= FETCH_ATTEMPTS) return request;
  return request.pipe(
    Effect.catch((error) => {
      if (!retryableStatsError(error)) return Effect.fail(error);
      return Effect.sleep(retryDelayMilliseconds(error, attempt)).pipe(
        Effect.andThen(fetchEngagementBatch(videoIds, key, apiUrl, attempt + 1)),
      );
    }),
  );
}

function loadEngagementCache(path: string): Effect.Effect<Map<string, Engagement>, Error> {
  return Effect.tryPromise({
    try: async () => {
      const text = await readFile(path, "utf8").catch((error: unknown) => {
        if (isNodeError(error) && error.code === "ENOENT") return "";
        throw error;
      });
      const cache = new Map<string, Engagement>();
      for (const line of text.split("\n")) {
        if (!line.trim()) continue;
        const value: unknown = JSON.parse(line);
        if (!isRecord(value) || typeof value.video_id !== "string") {
          throw new Error("Engagement cache contains an invalid record");
        }
        cache.set(value.video_id, {
          view_count: countNumber(value.view_count),
          like_count: countNumber(value.like_count),
          comment_count: countNumber(value.comment_count),
        });
      }
      return cache;
    },
    catch: asError,
  });
}

function appendEngagementCache(
  path: string,
  results: ReadonlyArray<readonly [string, Engagement]>,
): Effect.Effect<void, Error> {
  if (results.length === 0) return Effect.void;
  const records = results
    .map(([videoId, metrics]) => JSON.stringify({ video_id: videoId, ...metrics }))
    .join("\n");
  return Effect.tryPromise({
    try: () => appendFile(path, `${records}\n`, { encoding: "utf8", mode: 0o600 }),
    catch: asError,
  });
}

function writeEnrichedCorpus(
  input: string,
  output: string,
  engagement: ReadonlyMap<string, Engagement>,
): Effect.Effect<{ documents: number; metricRecords: number }, Error> {
  return Effect.tryPromise({
    try: async () => {
      const lines = createInterface({ input: corpusInput(input), crlfDelay: Infinity });
      const outputFile = createWriteStream(output, { flags: "wx" });
      const destination = output.endsWith(".gz") ? createGzip({ level: 1 }) : outputFile;
      if (destination !== outputFile) destination.pipe(outputFile);
      const destinationDone = finished(destination);
      const outputDone = destination === outputFile ? destinationDone : finished(outputFile);
      const enrichedVideos = new Set<string>();
      let documents = 0;
      let metricRecords = 0;
      try {
        for await (const line of lines) {
          if (!line.trim()) continue;
          const value: unknown = JSON.parse(line);
          if (!isRecord(value) || typeof value.video_id !== "string") {
            throw new Error("Corpus document is missing video_id");
          }
          const metrics = engagement.get(value.video_id);
          const shouldWriteMetrics = metrics !== undefined && !enrichedVideos.has(value.video_id);
          if (shouldWriteMetrics) {
            enrichedVideos.add(value.video_id);
            metricRecords += 1;
          }
          const outputLine = shouldWriteMetrics ? JSON.stringify({ ...value, ...metrics }) : line;
          const writable = destination.write(`${outputLine}\n`);
          if (!writable) await once(destination, "drain");
          documents += 1;
        }
        destination.end();
        await Promise.all([destinationDone, outputDone]);
        return { documents, metricRecords };
      } catch (error) {
        destination.destroy();
        outputFile.destroy();
        await Promise.allSettled([destinationDone, outputDone]);
        await rm(output, { force: true });
        throw error;
      }
    },
    catch: asError,
  });
}

function corpusInput(path: string): Readable {
  const input = createReadStream(path);
  return path.endsWith(".gz") ? input.pipe(createGunzip()) : input;
}

function chunk<T>(values: readonly T[], size: number): T[][] {
  const chunks: T[][] = [];
  for (let index = 0; index < values.length; index += size) {
    chunks.push(values.slice(index, index + size));
  }
  return chunks;
}

function count(value: string | undefined): number {
  if (!value) return 0;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : 0;
}

function countNumber(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new Error("Engagement cache contains an invalid counter");
  }
  return value;
}

function retryableStatsError(error: Error): boolean {
  return !(error instanceof VideoStatsHttpError) || error.status === 429 || error.status >= 500;
}

function retryDelayMilliseconds(error: Error, attempt: number): number {
  const exponential = Math.min(2_000 * 2 ** (attempt - 1), 60_000);
  const retryAfter = error instanceof VideoStatsHttpError ? (error.retryAfterMs ?? 0) : 0;
  const jitter = (attempt * 137) % 251;
  return Math.min(Math.max(exponential, retryAfter) + jitter, 300_000);
}

function retryAfterMilliseconds(value: string | null): number | undefined {
  if (!value) return undefined;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.round(seconds * 1_000);
  const date = Date.parse(value);
  if (!Number.isFinite(date)) return undefined;
  return Math.max(0, date - Date.now());
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && "code" in error;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asError(cause: unknown): Error {
  return cause instanceof Error ? cause : new Error(String(cause));
}
