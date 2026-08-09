let runtimePromise: Promise<typeof import("./effect-runtime")> | undefined;

export function loadEffectRuntime(): Promise<typeof import("./effect-runtime")> {
  runtimePromise ??= import("./effect-runtime");
  return runtimePromise;
}
