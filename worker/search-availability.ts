import * as Effect from "effect/Effect";
import {
  evaluateSearchSchedule,
  nextPublicOpening,
  type ScheduleEvaluation,
} from "../shared/service-schedule";

export type SearchAvailabilityStatus =
  | "scheduled_offline"
  | "prewarming"
  | "ready"
  | "starting"
  | "degraded";

export type SearchAvailability = {
  status: SearchAvailabilityStatus;
  search_enabled: boolean;
  enforced: boolean;
  schedule_policy: "always_on" | "scheduled" | "observe";
  schedule_phase: ScheduleEvaluation["phase"];
  time_zone: "America/Los_Angeles";
  daily_window: {
    opens: "12:00";
    closes: "00:00";
    label: "12:00 PM–12:00 AM Pacific Time";
  };
  next_transition_at: string;
};

type AvailabilityEnvironment = {
  SEARCH_SCHEDULE_MODE?: string;
  RAW_SEARCH_HEALTH_URL: string;
};

export function scheduleEnforced(env: AvailabilityEnvironment): boolean {
  return env.SEARCH_SCHEDULE_MODE === "enforce";
}

export function searchAlwaysOn(env: AvailabilityEnvironment): boolean {
  return env.SEARCH_SCHEDULE_MODE === "always_on";
}

function schedulePolicy(env: AvailabilityEnvironment): SearchAvailability["schedule_policy"] {
  if (scheduleEnforced(env)) return "scheduled";
  if (searchAlwaysOn(env)) return "always_on";
  return "observe";
}

export const searchAvailabilityEffect = (
  env: AvailabilityEnvironment,
  now = new Date(),
): Effect.Effect<SearchAvailability> =>
  Effect.gen(function* () {
    const schedule = evaluateSearchSchedule(now);
    const enforced = scheduleEnforced(env);
    const policy = schedulePolicy(env);
    if (enforced && schedule.phase !== "open") {
      return availability(
        schedule.phase === "prewarm" ? "prewarming" : "scheduled_offline",
        false,
        enforced,
        policy,
        schedule,
      );
    }

    const healthy = yield* rawSearchHealthyEffect(env);
    const status = healthy ? "ready" : schedule.phase === "open" ? "starting" : "degraded";
    return availability(status, healthy, enforced, policy, schedule);
  });

export const searchAvailability = (
  env: AvailabilityEnvironment,
  now = new Date(),
): Promise<SearchAvailability> => Effect.runPromise(searchAvailabilityEffect(env, now));

export function scheduledOfflineResponse(now = new Date()): Response {
  const availableAt = nextPublicOpening(now);
  const retryAfter = Math.max(
    1,
    Math.ceil((new Date(availableAt).getTime() - now.getTime()) / 1_000),
  );
  return Response.json(
    {
      code: "scheduled_offline",
      error: "Search is available daily from 12:00 PM to 12:00 AM Pacific Time.",
      available_at: availableAt,
    },
    {
      status: 503,
      headers: {
        "Cache-Control": "no-store",
        "Retry-After": String(retryAfter),
      },
    },
  );
}

export const rawSearchHealthyEffect = (env: AvailabilityEnvironment): Effect.Effect<boolean> =>
  Effect.tryPromise({
    try: (signal) =>
      fetch(env.RAW_SEARCH_HEALTH_URL, {
        headers: { Accept: "application/json" },
        signal,
      }),
    catch: () => false,
  }).pipe(
    Effect.timeout("2 seconds"),
    Effect.map((response) => response.ok),
    Effect.catch(() => Effect.succeed(false)),
  );

export const rawSearchHealthy = (env: AvailabilityEnvironment): Promise<boolean> =>
  Effect.runPromise(rawSearchHealthyEffect(env));

function availability(
  status: SearchAvailabilityStatus,
  searchEnabled: boolean,
  enforced: boolean,
  policy: SearchAvailability["schedule_policy"],
  schedule: ScheduleEvaluation,
): SearchAvailability {
  return {
    status,
    search_enabled: searchEnabled,
    enforced,
    schedule_policy: policy,
    schedule_phase: schedule.phase,
    time_zone: "America/Los_Angeles",
    daily_window: {
      opens: "12:00",
      closes: "00:00",
      label: "12:00 PM–12:00 AM Pacific Time",
    },
    next_transition_at: schedule.nextTransitionAt,
  };
}
