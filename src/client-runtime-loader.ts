let runtimePromise: Promise<typeof import("./client-runtime")> | undefined;

export function loadClientRuntime(): Promise<typeof import("./client-runtime")> {
  runtimePromise ??= import("./client-runtime").catch((error: unknown) => {
    runtimePromise = undefined;
    throw error;
  });
  return runtimePromise;
}
