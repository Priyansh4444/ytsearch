import { createEffect, createSignal, onCleanup, type Component } from "solid-js";
import { nextPublicOpening } from "../../shared/service-schedule";
import type { SearchAvailability } from "../types";

type Timer = {
  atLabel: string;
  countdown: string;
};

export const AvailabilityNotice: Component<{ availability: SearchAvailability }> = (props) => {
  const [now, setNow] = createSignal(Date.now());

  createEffect(() => {
    const interval = window.setInterval(() => setNow(Date.now()), 1_000);
    onCleanup(() => window.clearInterval(interval));
  });

  const timer = (): Timer => {
    const availability = props.availability;
    if (availability.schedule_phase === "open") {
      const closeAt = new Date(availability.next_transition_at);
      return {
        atLabel: formatLocalTime(closeAt),
        countdown: formatCountdown(Math.max(0, closeAt.getTime() - now())),
      };
    }
    const opensAt = new Date(nextPublicOpening(new Date(now())));
    return {
      atLabel: formatLocalTime(opensAt),
      countdown: formatCountdown(Math.max(0, opensAt.getTime() - now())),
    };
  };

  return (
    <div
      class="relative z-10 mx-auto flex w-full flex-wrap items-center justify-center gap-x-2 gap-y-1 px-2 text-center text-xs text-vesper-dim md:w-4/5 xl:w-2/3"
      role="status"
    >
      <span class="text-vesper-muted">{availabilityMessage(props.availability, timer)}</span>
    </div>
  );
};

function availabilityMessage(availability: SearchAvailability, timer: () => Timer): string {
  switch (availability.status) {
    case "scheduled_offline": {
      const { atLabel, countdown } = timer();
      return `Search returns at ${atLabel} in ${countdown}.`;
    }
    case "prewarming": {
      const { atLabel, countdown } = timer();
      return `Search is warming up and will be available at ${atLabel} in ${countdown}.`;
    }
    case "ready": {
      const { atLabel, countdown } = timer();
      return `Search is available now. Expires at ${atLabel} in ${countdown}.`;
    }
    case "starting":
      return "Search is starting. Please check back shortly.";
    case "degraded":
      return "Search should be available, but the search service is currently having trouble.";
  }
}

function formatLocalTime(at: Date): string {
  return new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(at);
}

function formatCountdown(ms: number): string {
  const totalSeconds = Math.floor(ms / 1_000);
  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}
