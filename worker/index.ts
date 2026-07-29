import { shouldWakeIndexer } from "./wake-policy";
import { requestOriginAllowed } from "./origin-policy";
import { evaluateSearchSchedule } from "../shared/service-schedule";
import {
  flyMachineState,
  startFlyMachine,
  stopFlyMachine,
  type FlyMachineState,
} from "./fly-machine";
import {
  scheduleEnforced,
  scheduledOfflineResponse,
  searchAvailability,
  typesenseHealthy,
} from "./search-availability";

type WakeMessage = {
  reason: "index-request" | "scheduled-repair" | "availability-open";
};

export default {
  async fetch(request, env): Promise<Response> {
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
      return env.ASSETS.fetch(request);
    }
    if (request.method === "GET" && url.pathname === "/api/availability") {
      return Response.json(await searchAvailability(env), {
        headers: { "Cache-Control": "no-store" },
      });
    }
    if (
      url.pathname === "/api/search" &&
      scheduleEnforced(env) &&
      evaluateSearchSchedule().phase !== "open"
    ) {
      const schedule = evaluateSearchSchedule();
      logEvent("scheduled_search_blocked", {
        schedule_phase: schedule.phase,
      });
      return scheduledOfflineResponse();
    }

    const upstreamUrl = new URL(url.pathname + url.search, env.API_BASE_URL);
    const upstreamResponse = await fetch(new Request(upstreamUrl, request));
    if (shouldWakeIndexer(request.method, url.pathname, upstreamResponse.ok)) {
      await env.INDEX_WAKE_QUEUE.send({ reason: "index-request" });
    }
    return new Response(upstreamResponse.body, upstreamResponse);
  },

  async queue(batch, env): Promise<void> {
    for (const message of batch.messages) {
      try {
        const schedule = evaluateSearchSchedule();
        if (scheduleEnforced(env) && schedule.phase !== "open") {
          logEvent("indexer_wake_deferred", {
            reason: message.body.reason,
            schedule_phase: schedule.phase,
          });
          message.ack();
          continue;
        }
        await startIndexer(env);
        message.ack();
      } catch (error) {
        // eslint-disable-next-line no-console -- Workers structured log event.
        console.error(
          JSON.stringify({
            event: "indexer_wake_failed",
            reason: message.body.reason,
            attempt: message.attempts,
            error: error instanceof Error ? error.message : String(error),
          }),
        );
        message.retry({ delaySeconds: retryDelaySeconds(message.attempts) });
      }
    }
  },

  async scheduled(controller, env): Promise<void> {
    if (controller.cron === "17 */6 * * *") {
      await env.INDEX_WAKE_QUEUE.send({ reason: "scheduled-repair" });
      logEvent("indexer_repair_wake_enqueued", {
        cron: controller.cron,
        scheduled_time: controller.scheduledTime,
      });
      return;
    }
    await reconcileSearchMachine(env, new Date(controller.scheduledTime));
  },
} satisfies ExportedHandler<Env, WakeMessage>;

async function startIndexer(env: Env): Promise<void> {
  const state = await flyMachineState(env, env.FLY_APP_NAME, env.FLY_MACHINE_ID);
  if (state === "started" || state === "starting") return;
  await startFlyMachine(env, env.FLY_APP_NAME, env.FLY_MACHINE_ID);
  logEvent("indexer_woken", {});
}

function retryDelaySeconds(attempts: number): number {
  return Math.min(30 * 2 ** Math.max(0, attempts - 1), 3_600);
}

async function reconcileSearchMachine(env: Env, now: Date): Promise<void> {
  const schedule = evaluateSearchSchedule(now);
  try {
    const state = await flyMachineState(
      env,
      env.FLY_TYPESENSE_APP_NAME,
      env.FLY_TYPESENSE_MACHINE_ID,
    );
    if (!scheduleEnforced(env)) {
      logEvent("typesense_schedule_observed", {
        schedule_phase: schedule.phase,
        machine_state: state,
        desired_state: schedule.phase === "closed" ? "stopped" : "started",
      });
      return;
    }

    if (schedule.phase !== "closed") {
      await ensureSearchMachineStarted(env, state, schedule.phase);
      if (
        schedule.phase === "open" &&
        schedule.localMinuteOfDay < 12 * 60 + 15 &&
        (await typesenseHealthy(env))
      ) {
        await env.INDEX_WAKE_QUEUE.send({ reason: "availability-open" });
      }
      return;
    }

    if (state !== "started" && state !== "starting") return;
    const indexerState = await flyMachineState(env, env.FLY_APP_NAME, env.FLY_MACHINE_ID);
    if (indexerState === "started" || indexerState === "starting") {
      logEvent("typesense_schedule_stop_deferred_indexer", {
        schedule_phase: schedule.phase,
        machine_state: state,
        indexer_state: indexerState,
      });
      return;
    }
    logEvent("typesense_schedule_stop_requested", {
      schedule_phase: schedule.phase,
      machine_state: state,
    });
    await stopFlyMachine(env, env.FLY_TYPESENSE_APP_NAME, env.FLY_TYPESENSE_MACHINE_ID);
    logEvent("typesense_schedule_stopped", { schedule_phase: schedule.phase });
  } catch (error) {
    logEvent("typesense_schedule_degraded", {
      schedule_phase: schedule.phase,
      error: safeError(error),
    });
  }
}

async function ensureSearchMachineStarted(
  env: Env,
  state: FlyMachineState,
  schedulePhase: "prewarm" | "open",
): Promise<void> {
  if (state === "started" || state === "starting") {
    if (state === "started" && (await typesenseHealthy(env))) {
      logEvent("typesense_schedule_ready", { schedule_phase: schedulePhase });
    }
    return;
  }
  logEvent("typesense_schedule_start_requested", {
    schedule_phase: schedulePhase,
    machine_state: state,
  });
  await startFlyMachine(env, env.FLY_TYPESENSE_APP_NAME, env.FLY_TYPESENSE_MACHINE_ID);
  logEvent("typesense_schedule_started", { schedule_phase: schedulePhase });
}

function logEvent(event: string, fields: Record<string, unknown>): void {
  // eslint-disable-next-line no-console -- Workers structured operational event.
  console.log(JSON.stringify({ event, ...fields }));
}

function safeError(error: unknown): string {
  if (!(error instanceof Error)) return "unknown lifecycle error";
  return error.message.replaceAll(/(Bearer|token|key)\s+[^\s]+/gi, "$1 [redacted]");
}
