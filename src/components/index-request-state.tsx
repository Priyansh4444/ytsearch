import { createSignal, onSettled, type Component } from "solid-js";
import { indexStatus } from "../api";
import type { IndexRequestStatus } from "../types";

export const IndexRequestState: Component<{ handle: string; requestId: string }> = (props) => {
  const [status, setStatus] = createSignal<IndexRequestStatus>("pending");
  const refresh = () =>
    void indexStatus(props.handle)
      .then((value) => setStatus(value.status))
      .catch(() => undefined);
  onSettled(() => {
    refresh();
    const timer = window.setInterval(refresh, 10_000);
    return () => window.clearInterval(timer);
  });
  return (
    <div
      class="border border-vesper-green/30 bg-vesper-panel p-5 shadow-2xl shadow-vesper-green/5"
      role="status"
      aria-live="polite"
    >
      <p class="text-sm uppercase tracking-[0.18em] text-vesper-green">
        {indexRequestHeading(status())}
      </p>
      <p class="mt-2 text-xl font-semibold text-white">{props.handle}</p>
      <p class="mt-2 max-w-2xl text-sm text-vesper-muted">
        {indexRequestDescription(status(), props.requestId)}
      </p>
    </div>
  );
};

function indexRequestHeading(status: IndexRequestStatus): string {
  switch (status) {
    case "pending":
      return "Awaiting approval";
    case "queued":
      return "Index request approved";
    case "running":
      return "Currently indexing";
    case "failed":
      return "Indexing failed";
    case "rejected":
      return "Request not approved";
    case "idle":
      return "Request no longer queued";
  }
}

function indexRequestDescription(status: IndexRequestStatus, requestId: string): string {
  switch (status) {
    case "pending":
      return `Request ${requestId} is waiting for administrator approval.`;
    case "queued":
      return `Request ${requestId} was approved and is waiting for the index worker.`;
    case "running":
      return `Request ${requestId} is being handled by the index worker.`;
    case "failed":
      return `Request ${requestId} could not be indexed and needs administrator review.`;
    case "rejected":
      return `Request ${requestId} was reviewed and declined.`;
    case "idle":
      return `Request ${requestId} is no longer in the indexing queue. Search this creator to check whether new videos are available.`;
  }
}
