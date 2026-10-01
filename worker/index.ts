import * as Effect from "effect/Effect";
import { shouldWakeIndexer } from "./wake-policy";
import { upstreamApiBase } from "./upstream-route";
import { requestOriginAllowed } from "./origin-policy";
import { evaluateSearchSchedule, type ScheduleEvaluation } from "../shared/service-schedule";
import {
  type FlyMachine,
  flyMachineStateEffect,
  flyMachinesEffect,
  startFlyMachineEffect,
  stopFlyMachineEffect,
  type FlyMachineState,
} from "./fly-machine";
import {
  scheduleEnforced,
  searchAlwaysOn,
  scheduledOfflineResponse,
  searchAvailabilityEffect,
  rawSearchHealthyEffect,
} from "./search-availability";

type WakeMessage = {
  reason: "index-request" | "scheduled-repair" | "availability-open";
};

const QUEUE_INDEXER_ROLE = "queue-indexer";

export default {
  fetch: (request, env): Promise<Response> =>
    Effect.runPromise(fetchEffect(request, env), { signal: request.signal }),
  queue: (batch, env): Promise<void> => Effect.runPromise(queueEffect(batch, env)),
  scheduled: (controller, env): Promise<void> =>
    Effect.runPromise(scheduledEffect(controller, env)),
} satisfies ExportedHandler<Env, WakeMessage>;

function fetchEffect(request: Request, env: Env): Effect.Effect<Response, Error> {
  return Effect.gen(function* () {
    const publicAppOrigin = (env as Env & { PUBLIC_APP_ORIGIN?: string }).PUBLIC_APP_ORIGIN;
    if (!requestOriginAllowed(request.url, publicAppOrigin)) {
      return new Response("Forbidden", {
        status: 403,
        headers: { "Cache-Control": "no-store" },
      });
    }
    const url = new URL(request.url);
    const isApiRequest = url.pathname === "/api" || url.pathname.startsWith("/api/");

    if (!isApiRequest) {
      return yield* fromPromise(() => env.ASSETS.fetch(request));
    }
    if (request.method === "GET" && url.pathname === "/api/availability") {
      const current = yield* searchAvailabilityEffect(env);
      return Response.json(current, { headers: { "Cache-Control": "no-store" } });
    }
    if (url.pathname === "/api/search" && scheduleEnforced(env)) {
      const schedule = evaluateSearchSchedule();
      if (schedule.phase !== "open") {
        yield* Effect.sync(() =>
          logEvent("scheduled_search_blocked", { schedule_phase: schedule.phase }),
        );
        return scheduledOfflineResponse();
      }
    }

    const upstreamUrl = new URL(
      url.pathname + url.search,
      upstreamApiBase(url.pathname, env.API_BASE_URL, env.INDEX_QUEUE_API_BASE_URL),
    );
    const upstreamRequest = new Request(upstreamUrl, request);
    const dynamicSearch = request.method === "GET" && url.pathname === "/api/search";
    const upstreamResponse = yield* fromPromise(() =>
      fetch(upstreamRequest, dynamicSearch ? { cache: "no-store" } : undefined),
    );
    if (shouldWakeIndexer(request.method, url.pathname, upstreamResponse.ok)) {
      yield* fromPromise(() => env.INDEX_WAKE_QUEUE.send({ reason: "index-request" }));
    }
    const response = new Response(upstreamResponse.body, upstreamResponse);
    if (dynamicSearch) {
      response.headers.set("Cache-Control", "no-store");
      response.headers.set("CDN-Cache-Control", "no-store");
      response.headers.set("Cloudflare-CDN-Cache-Control", "no-store");
    }
    return response;
  });
}

function queueEffect(batch: MessageBatch<WakeMessage>, env: Env): Effect.Effect<void> {
  return Effect.forEach(
    batch.messages,
    (message) =>
      Effect.gen(function* () {
        const schedule = evaluateSearchSchedule();
        if (scheduleEnforced(env) && schedule.phase !== "open") {
          yield* Effect.sync(() => {
            logEvent("indexer_wake_deferred", {
              reason: message.body.reason,
              schedule_phase: schedule.phase,
            });
            message.ack();
          });
          return;
        }
        yield* startIndexerEffect(env);
        yield* Effect.sync(() => message.ack());
      }).pipe(
        Effect.catch((error) =>
          Effect.sync(() => {
            // eslint-disable-next-line no-console -- Workers structured log event.
            console.error(
              JSON.stringify({
                event: "indexer_wake_failed",
                reason: message.body.reason,
                attempt: message.attempts,
                error: error.message,
              }),
            );
            message.retry({ delaySeconds: retryDelaySeconds(message.attempts) });
          }),
        ),
      ),
    { concurrency: 1, discard: true },
  );
}

function scheduledEffect(controller: ScheduledController, env: Env): Effect.Effect<void, Error> {
  if (controller.cron === "17 * * * *") {
    return Effect.gen(function* () {
      yield* fromPromise(() => env.INDEX_WAKE_QUEUE.send({ reason: "scheduled-repair" }));
      yield* Effect.sync(() =>
        logEvent("indexer_repair_wake_enqueued", {
          cron: controller.cron,
          scheduled_time: controller.scheduledTime,
        }),
      );
    });
  }
  return reconcileSearchMachineEffect(env, new Date(controller.scheduledTime));
}

function startIndexerEffect(env: Env): Effect.Effect<void, Error> {
  return Effect.gen(function* () {
    const machine = yield* queueIndexerMachineEffect(env);
    if (!machine)
      return yield* Effect.fail(new Error("No Raw queue indexer Machine is configured"));
    const state = machine.state ?? "unknown";
    if (state === "started" || state === "starting") return;
    if (!isStopped(state))
      return yield* Effect.fail(new Error(`Queue indexer cannot start from state: ${state}`));
    yield* startFlyMachineEffect(env, env.FLY_APP_NAME, machine.id);
    yield* Effect.sync(() => logEvent("indexer_woken", { machine_id: machine.id }));
  });
}

function retryDelaySeconds(attempts: number): number {
  return Math.min(30 * 2 ** Math.max(0, attempts - 1), 3_600);
}

function reconcileSearchMachineEffect(env: Env, now: Date): Effect.Effect<void> {
  const schedule = evaluateSearchSchedule(now);
  return Effect.gen(function* () {
    const state = yield* flyMachineStateEffect(
      env,
      env.FLY_RAW_SEARCH_APP_NAME,
      env.FLY_RAW_SEARCH_MACHINE_ID,
    );
    if (searchAlwaysOn(env)) {
      yield* Effect.sync(() =>
        logEvent("raw_search_always_on_reconciled", {
          schedule_phase: schedule.phase,
          machine_state: state,
          desired_state: "started",
        }),
      );
      yield* ensureSearchMachineStartedEffect(env, state, schedule.phase);
      return;
    }

    if (!scheduleEnforced(env)) {
      yield* Effect.sync(() =>
        logEvent("raw_search_schedule_mode_invalid", {
          configured_mode: env.SEARCH_SCHEDULE_MODE,
          schedule_phase: schedule.phase,
          machine_state: state,
        }),
      );
      return;
    }

    if (schedule.phase !== "closed") {
      yield* ensureSearchMachineStartedEffect(env, state, schedule.phase);
      if (
        schedule.phase === "open" &&
        schedule.localMinuteOfDay < 12 * 60 + 15 &&
        (yield* rawSearchHealthyEffect(env))
      ) {
        yield* fromPromise(() => env.INDEX_WAKE_QUEUE.send({ reason: "availability-open" }));
      }
      return;
    }

    if (state !== "started" && state !== "starting") return;
    const indexer = yield* queueIndexerMachineEffect(env);
    const indexerState = indexer?.state ?? "stopped";
    if (indexer && (indexerState === "started" || indexerState === "starting")) {
      yield* Effect.sync(() =>
        logEvent("indexer_schedule_stop_requested", {
          schedule_phase: schedule.phase,
          machine_id: indexer.id,
          indexer_state: indexerState,
        }),
      );
      yield* stopFlyMachineEffect(env, env.FLY_APP_NAME, indexer.id);
      yield* Effect.sync(() =>
        logEvent("indexer_schedule_stop_accepted", {
          schedule_phase: schedule.phase,
          machine_id: indexer.id,
        }),
      );
      return;
    }
    if (indexer && !isStopped(indexerState)) {
      yield* Effect.sync(() =>
        logEvent("raw_search_schedule_stop_deferred_indexer", {
          schedule_phase: schedule.phase,
          machine_state: state,
          indexer_state: indexerState,
        }),
      );
      return;
    }
    yield* Effect.sync(() =>
      logEvent("raw_search_schedule_stop_requested", {
        schedule_phase: schedule.phase,
        machine_state: state,
      }),
    );
    yield* stopFlyMachineEffect(env, env.FLY_RAW_SEARCH_APP_NAME, env.FLY_RAW_SEARCH_MACHINE_ID);
    yield* Effect.sync(() =>
      logEvent("raw_search_schedule_stopped", { schedule_phase: schedule.phase }),
    );
  }).pipe(
    Effect.catch((error) =>
      Effect.sync(() =>
        logEvent("raw_search_schedule_degraded", {
          schedule_phase: schedule.phase,
          error: safeError(error),
        }),
      ),
    ),
  );
}

function queueIndexerMachineEffect(env: Env): Effect.Effect<FlyMachine | undefined, Error> {
  return Effect.gen(function* () {
    const machines = (yield* flyMachinesEffect(env, env.FLY_APP_NAME)).filter(
      (machine) =>
        machine.config?.env?.YTSEARCH_MACHINE_ROLE === QUEUE_INDEXER_ROLE &&
        machine.config.env.QUEUE_TARGET_BACKEND === "raw",
    );
    if (machines.length > 1) {
      return yield* Effect.fail(
        new Error(`Expected one Raw queue indexer Machine, found ${machines.length}`),
      );
    }
    return machines[0];
  });
}

function isStopped(state: FlyMachineState): boolean {
  return state === "stopped" || state === "suspended" || state === "destroyed";
}

function ensureSearchMachineStartedEffect(
  env: Env,
  state: FlyMachineState,
  schedulePhase: ScheduleEvaluation["phase"],
): Effect.Effect<void, Error> {
  return Effect.gen(function* () {
    if (state === "started" || state === "starting") {
      if (state === "started" && (yield* rawSearchHealthyEffect(env))) {
        yield* Effect.sync(() =>
          logEvent("raw_search_schedule_ready", { schedule_phase: schedulePhase }),
        );
      }
      return;
    }
    yield* Effect.sync(() =>
      logEvent("raw_search_schedule_start_requested", {
        schedule_phase: schedulePhase,
        machine_state: state,
      }),
    );
    yield* startFlyMachineEffect(env, env.FLY_RAW_SEARCH_APP_NAME, env.FLY_RAW_SEARCH_MACHINE_ID);
    yield* Effect.sync(() =>
      logEvent("raw_search_schedule_started", { schedule_phase: schedulePhase }),
    );
  });
}

function fromPromise<A>(operation: () => PromiseLike<A>): Effect.Effect<A, Error> {
  return Effect.tryPromise({
    try: operation,
    catch: asError,
  });
}

function logEvent(event: string, fields: Record<string, unknown>): void {
  // eslint-disable-next-line no-console -- Workers structured operational event.
  console.log(JSON.stringify({ event, ...fields }));
}

function safeError(error: unknown): string {
  if (!(error instanceof Error)) return "unknown lifecycle error";
  return error.message.replaceAll(/(Bearer|token|key)\s+[^\s]+/gi, "$1 [redacted]");
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
