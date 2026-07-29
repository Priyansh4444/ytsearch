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
  TYPESENSE_HEALTH_URL: string;
};

export function scheduleEnforced(env: AvailabilityEnvironment): boolean {
  return env.SEARCH_SCHEDULE_MODE === "enforce";
}

export async function searchAvailability(
  env: AvailabilityEnvironment,
  now = new Date(),
): Promise<SearchAvailability> {
  const schedule = evaluateSearchSchedule(now);
  const enforced = scheduleEnforced(env);
  if (enforced && schedule.phase !== "open") {
    return availability(
      schedule.phase === "prewarm" ? "prewarming" : "scheduled_offline",
      false,
      enforced,
      schedule,
    );
  }

  const healthy = await typesenseHealthy(env);
  const status = healthy ? "ready" : schedule.phase === "open" ? "starting" : "degraded";
  return availability(status, healthy, enforced, schedule);
}

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

export async function typesenseHealthy(env: AvailabilityEnvironment): Promise<boolean> {
  try {
    const response = await fetch(env.TYPESENSE_HEALTH_URL, {
      headers: { Accept: "application/json" },
      signal: AbortSignal.timeout(2_000),
    });
    return response.ok;
  } catch {
    return false;
  }
}

function availability(
  status: SearchAvailabilityStatus,
  searchEnabled: boolean,
  enforced: boolean,
  schedule: ScheduleEvaluation,
): SearchAvailability {
  return {
    status,
    search_enabled: searchEnabled,
    enforced,
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
