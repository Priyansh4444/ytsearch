import { describe, expect, test } from "vitest";
import { CHANNEL_CACHE_KEY, cacheChannels, loadCachedChannels } from "./channel-cache";

const channel = {
  id: "one",
  handle: "@one",
  name: "One",
  channel_id: "UCone",
  public_video_count: 2,
  indexed_video_count: 1,
  indexed_chunk_count: 10,
};

describe("channel cache", () => {
  test("loads a fresh cache and rejects expired or corrupt data", () => {
    const storage = new MemoryStorage();
    cacheChannels([channel], "2026-07-26T20:00:00Z", storage);
    expect(loadCachedChannels(new Date("2026-07-26T20:29:59Z").getTime(), storage)).toEqual([
      channel,
    ]);
    expect(loadCachedChannels(new Date("2026-07-26T20:30:01Z").getTime(), storage)).toBeUndefined();
    storage.setItem(CHANNEL_CACHE_KEY, "{");
    expect(loadCachedChannels(Date.now(), storage)).toBeUndefined();
  });
});

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();
  get length() {
    return this.values.size;
  }
  clear() {
    this.values.clear();
  }
  getItem(key: string) {
    return this.values.get(key) ?? null;
  }
  key(index: number) {
    return [...this.values.keys()][index] ?? null;
  }
  removeItem(key: string) {
    this.values.delete(key);
  }
  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}
