export const SEARCH_TIME_ZONE = "America/Los_Angeles";
export const SEARCH_OPEN_MINUTE = 12 * 60;
export const SEARCH_PREWARM_MINUTE = SEARCH_OPEN_MINUTE - 15;

export type SchedulePhase = "closed" | "prewarm" | "open";

export type ScheduleEvaluation = {
  phase: SchedulePhase;
  timeZone: typeof SEARCH_TIME_ZONE;
  localMinuteOfDay: number;
  nextTransitionAt: string;
};

type ZonedParts = {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
};

const formatter = new Intl.DateTimeFormat("en-US", {
  timeZone: SEARCH_TIME_ZONE,
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

export function evaluateSearchSchedule(now = new Date()): ScheduleEvaluation {
  const local = zonedParts(now);
  const localMinuteOfDay = local.hour * 60 + local.minute;
  const phase =
    localMinuteOfDay >= SEARCH_OPEN_MINUTE
      ? "open"
      : localMinuteOfDay >= SEARCH_PREWARM_MINUTE
        ? "prewarm"
        : "closed";
  const target =
    phase === "closed"
      ? { ...local, hour: 11, minute: 45 }
      : phase === "prewarm"
        ? { ...local, hour: 12, minute: 0 }
        : { ...addLocalDays(local, 1), hour: 0, minute: 0 };

  return {
    phase,
    timeZone: SEARCH_TIME_ZONE,
    localMinuteOfDay,
    nextTransitionAt: zonedDateTimeToUtc(target).toISOString(),
  };
}

export function nextPublicOpening(now = new Date()): string {
  const local = zonedParts(now);
  const localMinuteOfDay = local.hour * 60 + local.minute;
  const openingDay = localMinuteOfDay < SEARCH_OPEN_MINUTE ? local : addLocalDays(local, 1);
  return zonedDateTimeToUtc({ ...openingDay, hour: 12, minute: 0 }).toISOString();
}

function zonedParts(date: Date): ZonedParts {
  const parts = Object.fromEntries(
    formatter
      .formatToParts(date)
      .filter((part) => part.type !== "literal")
      .map((part) => [part.type, Number(part.value)]),
  );
  return {
    year: parts.year,
    month: parts.month,
    day: parts.day,
    hour: parts.hour,
    minute: parts.minute,
  };
}

function addLocalDays(parts: ZonedParts, days: number): ZonedParts {
  const date = new Date(Date.UTC(parts.year, parts.month - 1, parts.day + days));
  return {
    ...parts,
    year: date.getUTCFullYear(),
    month: date.getUTCMonth() + 1,
    day: date.getUTCDate(),
  };
}

function zonedDateTimeToUtc(target: ZonedParts): Date {
  const desiredAsUtc = Date.UTC(
    target.year,
    target.month - 1,
    target.day,
    target.hour,
    target.minute,
  );
  let candidate = desiredAsUtc;
  for (let attempt = 0; attempt < 4; attempt += 1) {
    const actual = zonedParts(new Date(candidate));
    const actualAsUtc = Date.UTC(
      actual.year,
      actual.month - 1,
      actual.day,
      actual.hour,
      actual.minute,
    );
    const correction = desiredAsUtc - actualAsUtc;
    candidate += correction;
    if (correction === 0) break;
  }
  return new Date(candidate);
}
