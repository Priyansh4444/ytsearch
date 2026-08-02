import { createReadStream, createWriteStream } from "node:fs";
import { once } from "node:events";
import { createInterface } from "node:readline";
import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";

const BATCH_SIZE = 50;
const FETCH_CONCURRENCY = 2;
const FETCH_ATTEMPTS = 3;

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

const decodeVideoStats = Schema.decodeUnknownEffect(VideoStatsResponseSchema);

const [inputPath, outputPath] = process.argv.slice(2);
const apiKey = process.env.YOUTUBE_API_KEY?.trim();

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
  const batches = chunk(videoIds, BATCH_SIZE);
  const batchResults = yield* Effect.forEach(
    batches,
    (batch) => fetchEngagementBatch(batch, apiKey),
    { concurrency: FETCH_CONCURRENCY },
  );
  const engagement = new Map(batchResults.flatMap((batch) => batch));
  const written = yield* writeEnrichedCorpus(inputPath, outputPath, engagement);

  return {
    input: inputPath,
    output: outputPath,
    requested_videos: videoIds.length,
    enriched_videos: engagement.size,
    missing_videos: videoIds.length - engagement.size,
    written_documents: written,
    api_requests: batches.length,
  };
});

const result = await Effect.runPromise(program);
process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);

function collectVideoIds(path: string): Effect.Effect<string[], Error> {
  return Effect.tryPromise({
    try: async () => {
      const ids = new Set<string>();
      const lines = createInterface({ input: createReadStream(path), crlfDelay: Infinity });
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
  attempt = 1,
): Effect.Effect<Array<[string, Engagement]>, Error> {
  const request = Effect.gen(function* () {
    const url = new URL("https://www.googleapis.com/youtube/v3/videos:batchGetStats");
    url.searchParams.set("part", "statistics");
    url.searchParams.set("id", videoIds.join(","));
    url.searchParams.set("key", key);
    const response = yield* Effect.tryPromise({
      try: (signal) => fetch(url, { signal }),
      catch: asError,
    });
    if (!response.ok) {
      return yield* Effect.fail(
        new Error(`YouTube statistics request returned HTTP ${response.status}`),
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
    Effect.catch(() =>
      Effect.sleep(250 * 2 ** (attempt - 1)).pipe(
        Effect.andThen(fetchEngagementBatch(videoIds, key, attempt + 1)),
      ),
    ),
  );
}

function writeEnrichedCorpus(
  input: string,
  output: string,
  engagement: ReadonlyMap<string, Engagement>,
): Effect.Effect<number, Error> {
  return Effect.tryPromise({
    try: async () => {
      const lines = createInterface({ input: createReadStream(input), crlfDelay: Infinity });
      const destination = createWriteStream(output, { flags: "wx" });
      let documents = 0;
      try {
        for await (const line of lines) {
          if (!line.trim()) continue;
          const value: unknown = JSON.parse(line);
          if (!isRecord(value) || typeof value.video_id !== "string") {
            throw new Error("Corpus document is missing video_id");
          }
          const metrics = engagement.get(value.video_id) ?? {
            view_count: 0,
            like_count: 0,
            comment_count: 0,
          };
          const writable = destination.write(`${JSON.stringify({ ...value, ...metrics })}\n`);
          if (!writable) await once(destination, "drain");
          documents += 1;
        }
        destination.end();
        await once(destination, "finish");
        return documents;
      } catch (error) {
        destination.destroy();
        throw error;
      }
    },
    catch: asError,
  });
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asError(cause: unknown): Error {
  return cause instanceof Error ? cause : new Error(String(cause));
}
