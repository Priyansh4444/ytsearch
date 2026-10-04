import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";

export type FlyMachineState = string;
export type FlyMachine = {
  id: string;
  state?: FlyMachineState;
  config?: { env?: Record<string, string> };
};

type FlyEnvironment = {
  FLY_API_BASE_URL: string;
  FLYIO_TOKEN?: string;
};

const FlyMachineSchema = Schema.Struct({
  id: Schema.String,
  state: Schema.optionalKey(Schema.String),
  config: Schema.optionalKey(
    Schema.Struct({ env: Schema.optionalKey(Schema.Record(Schema.String, Schema.String)) }),
  ),
});
const FlyMachineStateSchema = Schema.Struct({ state: Schema.optionalKey(Schema.String) });
const FlyMachinesSchema = Schema.mutable(Schema.Array(FlyMachineSchema)) satisfies Schema.Schema<
  FlyMachine[]
>;
const decodeFlyMachineState = Schema.decodeUnknownEffect(FlyMachineStateSchema);
const decodeFlyMachines = Schema.decodeUnknownEffect(FlyMachinesSchema);

class FlyApiError extends Error {
  constructor(
    readonly operation: string,
    readonly status?: number,
    readonly cause?: unknown,
  ) {
    super(
      status === undefined
        ? `${operation} failed${cause instanceof Error ? `: ${cause.message}` : ""}`
        : `${operation} failed with HTTP ${status}`,
    );
    this.name = "FlyApiError";
  }
}

export const flyMachineStateEffect = (
  env: FlyEnvironment,
  appName: string,
  machineId: string,
): Effect.Effect<FlyMachineState, FlyApiError> =>
  Effect.gen(function* () {
    const response = yield* flyFetchEffect(
      machineUrl(env, appName, machineId),
      { headers: flyHeaders(env) },
      "Fly machine lookup",
    );
    const machine = yield* decodeResponseEffect(
      response,
      decodeFlyMachineState,
      "Fly machine lookup",
    );
    return machine.state ?? "unknown";
  });

export const flyMachinesEffect = (
  env: FlyEnvironment,
  appName: string,
): Effect.Effect<FlyMachine[], FlyApiError> =>
  Effect.gen(function* () {
    const response = yield* flyFetchEffect(
      `${appUrl(env, appName)}/machines`,
      { headers: flyHeaders(env) },
      "Fly machine list",
    );
    return yield* decodeResponseEffect(response, decodeFlyMachines, "Fly machine list");
  });

export const startFlyMachineEffect = (
  env: FlyEnvironment,
  appName: string,
  machineId: string,
): Effect.Effect<void, FlyApiError> =>
  flyFetchEffect(
    `${machineUrl(env, appName, machineId)}/start`,
    { method: "POST", headers: flyHeaders(env) },
    "Fly machine start",
  ).pipe(Effect.asVoid);

export const stopFlyMachineEffect = (
  env: FlyEnvironment,
  appName: string,
  machineId: string,
): Effect.Effect<void, FlyApiError> =>
  flyFetchEffect(
    `${machineUrl(env, appName, machineId)}/stop`,
    { method: "POST", headers: flyHeaders(env) },
    "Fly machine stop",
  ).pipe(Effect.asVoid);

function flyFetchEffect(
  input: RequestInfo | URL,
  init: RequestInit,
  operation: string,
): Effect.Effect<Response, FlyApiError> {
  return Effect.tryPromise({
    try: (signal) => fetch(input, { ...init, signal }),
    catch: (cause) => new FlyApiError(operation, undefined, cause),
  }).pipe(
    Effect.flatMap((response) =>
      response.ok
        ? Effect.succeed(response)
        : Effect.fail(new FlyApiError(operation, response.status)),
    ),
  );
}

function decodeResponseEffect<A, E>(
  response: Response,
  decode: (input: unknown) => Effect.Effect<A, E>,
  operation: string,
): Effect.Effect<A, FlyApiError> {
  return Effect.tryPromise({
    try: () => response.json(),
    catch: (cause) => new FlyApiError(`${operation} response decoding`, undefined, cause),
  }).pipe(
    Effect.flatMap(decode),
    Effect.mapError((cause) =>
      cause instanceof FlyApiError
        ? cause
        : new FlyApiError(`${operation} response validation`, undefined, cause),
    ),
  );
}

function machineUrl(env: FlyEnvironment, appName: string, machineId: string): string {
  return `${appUrl(env, appName)}/machines/${encodeURIComponent(machineId)}`;
}

function appUrl(env: FlyEnvironment, appName: string): string {
  return `${env.FLY_API_BASE_URL}/v1/apps/${encodeURIComponent(appName)}`;
}

function flyHeaders(env: FlyEnvironment): { Authorization: string } {
  if (!env.FLYIO_TOKEN) throw new FlyApiError("Fly API authentication");
  return { Authorization: `Bearer ${env.FLYIO_TOKEN}` };
}
