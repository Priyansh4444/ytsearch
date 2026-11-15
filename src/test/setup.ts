import * as matchers from "@testing-library/jest-dom/matchers";
import type { TestingLibraryMatchers } from "@testing-library/jest-dom/matchers";
import { cleanup, configure } from "@solidjs/testing-library";
import { afterEach, expect } from "vitest";

// jest-dom's /vitest entry still augments Vitest 4's Assertion<T>. Vitest 5
// exposes Matchers<return type, received type> for custom matcher declarations.
declare module "vitest" {
  interface Matchers<R extends void | Promise<void>, T> extends TestingLibraryMatchers<T, R> {}
}

expect.extend(matchers);

// findBy*/waitFor default to 1 s, which parallel check:all runs on one machine
// exceed: "opens a creator from the home swarm" timed out under load.
configure({ asyncUtilTimeout: 5_000 });

const storageValues = new Map<string, string>();
const testStorage: Storage = {
  get length() {
    return storageValues.size;
  },
  clear: () => storageValues.clear(),
  getItem: (key) => storageValues.get(key) ?? null,
  key: (index) => [...storageValues.keys()][index] ?? null,
  removeItem: (key) => storageValues.delete(key),
  setItem: (key, value) => storageValues.set(key, String(value)),
};

Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: testStorage,
});

Object.defineProperty(window, "matchMedia", {
  configurable: true,
  value: (query: string): MediaQueryList => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => true,
  }),
});

afterEach(() => {
  cleanup();
  testStorage.clear();
});
