let runtimePromise: Promise<typeof import("./effect-runtime")> | undefined;

export function loadEffectRuntime(): Promise<typeof import("./effect-runtime")> {
  runtimePromise ??= import("./effect-runtime").catch((error: unknown) => {
    // A transient chunk/network failure must not poison every later search in
    // this tab. Let the next interaction retry the lazy import.
    runtimePromise = undefined;
    throw error;
  });
  return runtimePromise;
}
