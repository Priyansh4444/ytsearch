import type { Component } from "solid-js";
import type { SearchAvailability } from "../types";

export const AvailabilityNotice: Component<{ availability: SearchAvailability }> = (props) => (
  <div
    class="relative z-10 mx-auto flex w-full flex-wrap items-center justify-center gap-x-2 gap-y-1 px-2 text-center text-xs text-vesper-dim md:w-4/5 xl:w-2/3"
    role="status"
  >
    <span>Search hours: 12:00 PM–12:00 AM Pacific</span>
    <span aria-hidden="true">·</span>
    <span class="text-vesper-muted">{availabilityMessage(props.availability)}</span>
  </div>
);

function availabilityMessage(availability: SearchAvailability): string {
  if (!availability.enforced && availability.search_enabled) return "Search is available now.";
  switch (availability.status) {
    case "ready":
      return "Search is available now.";
    case "scheduled_offline":
      return "Search returns at 12:00 PM Pacific Time.";
    case "prewarming":
      return "Search is warming up and will be available at 12:00 PM.";
    case "starting":
      return "Search is starting. Please check back shortly.";
    case "degraded":
      return "Search should be available, but the search service is currently having trouble.";
  }
}
