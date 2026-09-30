import { ApiDecodeError, ApiResponseError, ApiTransportError } from "./api-error";
import type { IndexedChannel, IndexStatus, SearchAvailability } from "./types";

type JsonRecord = Record<string, unknown>;

export async function fetchBootstrapAvailability(
  apiBaseUrl: string,
  signal?: AbortSignal,
): Promise<SearchAvailability> {
  return requestWireJson(
    `${apiBaseUrl}/api/availability`,
    "Could not load search availability",
    "Search availability returned an invalid response",
    decodeAvailability,
    signal,
  );
}

export async function fetchBootstrapChannels(
  apiBaseUrl: string,
  signal?: AbortSignal,
): Promise<IndexedChannel[]> {
  return requestWireJson(
    `${apiBaseUrl}/api/youtubers`,
    "Could not load indexed channels",
    "Channel list returned an invalid response",
    (input) => array(record(input).channels, decodeChannel),
    signal,
  );
}

export function fetchBootstrapIndexStatus(
  apiBaseUrl: string,
  handle?: string,
  signal?: AbortSignal,
): Promise<IndexStatus> {
  const params = handle ? `?handle=${encodeURIComponent(handle)}` : "";
  return requestWireJson(
    `${apiBaseUrl}/api/index-status${params}`,
    "Could not load indexing status",
    "Index status returned an invalid response",
    decodeIndexStatus,
    signal,
  );
}

export async function requestWireJson<T>(
  url: string,
  operation: string,
  decodeOperation: string,
  decode: (input: unknown) => T,
  signal?: AbortSignal,
  init?: RequestInit,
): Promise<T> {
  let response: Response;
  try {
    response = await fetch(url, { ...init, signal });
  } catch (cause) {
    throw new ApiTransportError(operation, cause);
  }
  if (!response.ok) {
    const body = await response.json().catch(() => undefined);
    const error = isRecord(body) ? body : undefined;
    throw new ApiResponseError(
      typeof error?.error === "string" ? error.error : operation,
      response.status,
      typeof error?.code === "string" ? error.code : undefined,
      typeof error?.available_at === "string" ? error.available_at : undefined,
    );
  }
  try {
    return decode(await response.json());
  } catch (cause) {
    throw new ApiDecodeError(decodeOperation, response.status, cause);
  }
}

function decodeAvailability(input: unknown): SearchAvailability {
  const value = record(input);
  const dailyWindow = record(value.daily_window);
  return {
    status: literal(value.status, [
      "scheduled_offline",
      "prewarming",
      "ready",
      "starting",
      "degraded",
    ]),
    search_enabled: boolean(value.search_enabled),
    enforced: boolean(value.enforced),
    schedule_policy: literal(value.schedule_policy, ["always_on", "scheduled", "observe"]),
    schedule_phase: literal(value.schedule_phase, ["closed", "prewarm", "open"]),
    time_zone: literal(value.time_zone, ["America/Los_Angeles"]),
    daily_window: {
      opens: literal(dailyWindow.opens, ["12:00"]),
      closes: literal(dailyWindow.closes, ["00:00"]),
      label: literal(dailyWindow.label, ["12:00 PM–12:00 AM Pacific Time"]),
    },
    next_transition_at: string(value.next_transition_at),
  };
}

function decodeChannel(input: unknown): IndexedChannel {
  const value = record(input);
  return {
    id: string(value.id),
    handle: string(value.handle),
    name: string(value.name),
    channel_id: string(value.channel_id),
    uploads_playlist: optionalString(value.uploads_playlist),
    public_video_count: number(value.public_video_count),
    indexed_video_count: number(value.indexed_video_count),
    indexed_chunk_count: number(value.indexed_chunk_count),
    avatar_url: optionalString(value.avatar_url),
  };
}

function decodeIndexStatus(input: unknown): IndexStatus {
  const value = record(input);
  return {
    handle: string(value.handle),
    status: literal(value.status, ["pending", "queued", "running", "failed", "rejected", "idle"]),
    channel:
      value.channel === undefined || value.channel === null
        ? undefined
        : decodeChannel(value.channel),
    request_id: optionalString(value.request_id),
    target_backend: optionalString(value.target_backend),
  };
}

function isRecord(input: unknown): input is JsonRecord {
  return typeof input === "object" && input !== null && !Array.isArray(input);
}

function record(input: unknown): JsonRecord {
  if (!isRecord(input)) throw new TypeError("Expected an object");
  return input;
}

function string(input: unknown): string {
  if (typeof input !== "string") throw new TypeError("Expected a string");
  return input;
}

// The API serializes absent optional fields as JSON null (Rust `Option`
// without `skip_serializing_if`), so null and undefined both mean absent.
function optionalString(input: unknown): string | undefined {
  return input === undefined || input === null ? undefined : string(input);
}

function number(input: unknown): number {
  if (typeof input !== "number" || !Number.isFinite(input))
    throw new TypeError("Expected a number");
  return input;
}

function boolean(input: unknown): boolean {
  if (typeof input !== "boolean") throw new TypeError("Expected a boolean");
  return input;
}

function array<T>(input: unknown, decode: (value: unknown) => T): T[] {
  if (!Array.isArray(input)) throw new TypeError("Expected an array");
  return input.map(decode);
}

function literal<const T extends string>(input: unknown, values: readonly T[]): T {
  if (typeof input !== "string" || !values.includes(input as T)) {
    throw new TypeError("Expected a known literal");
  }
  return input as T;
}
