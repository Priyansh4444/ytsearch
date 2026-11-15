import { For, Show, createSignal, onCleanup, type Component } from "solid-js";
import { ApiResponseError } from "../api-error";
import { approveIndexRequest, listIndexRequests } from "../dashboard-api";
import type { IndexQueueRequest, IndexedChannel } from "../types";

type ApprovalSession =
  | { kind: "locked" }
  | { kind: "unlocked"; passkey: string; requests: IndexQueueRequest[] };

export const DashboardApprovals: Component<{ channels: IndexedChannel[] }> = (props) => {
  const [session, setSession] = createSignal<ApprovalSession>({ kind: "locked" });
  const [passkey, setPasskey] = createSignal("");
  const [operation, setOperation] = createSignal<string>();
  const [error, setError] = createSignal<string>();
  let generation = 0;
  onCleanup(() => {
    generation += 1;
  });
  const lock = () => {
    generation += 1;
    setSession({ kind: "locked" });
    setPasskey("");
    setOperation(undefined);
    setError(undefined);
  };
  const run = async (id?: string) => {
    if (operation()) return;
    const current = session();
    const key = current.kind === "unlocked" ? current.passkey : passkey().trim();
    if (!key) return;
    const version = generation;
    setOperation(id ?? "refresh");
    setError(undefined);
    try {
      if (id && current.kind === "unlocked") {
        const approved = await approveIndexRequest(id, key);
        if (version !== generation) return;
        setSession({
          ...current,
          requests: current.requests.map((request) => (request.id === id ? approved : request)),
        });
      } else {
        const requests = await listIndexRequests(key);
        if (version !== generation) return;
        setSession({ kind: "unlocked", passkey: key, requests });
        setPasskey("");
      }
    } catch (cause) {
      if (version !== generation) return;
      if (cause instanceof ApiResponseError && [401, 403].includes(cause.status)) {
        lock();
        setError("Incorrect passkey or approval access expired. Enter your passkey again.");
      } else {
        setError("Could not complete the request. Please try again.");
      }
    } finally {
      if (version === generation) setOperation(undefined);
    }
  };
  const pending = () => {
    const current = session();
    if (current.kind === "locked") return [];
    const indexed = new Set(props.channels.map((channel) => channel.handle.toLowerCase()));
    return current.requests.filter(
      (request) => request.status === "pending" && !indexed.has(request.handle.toLowerCase()),
    );
  };
  const buttonClass =
    "pressable min-h-11 border border-vesper-control bg-vesper-panel px-4 py-2 text-sm text-white hover:border-vesper-yellow disabled:opacity-60";
  return (
    <section
      aria-labelledby="approvals-heading"
      class="border border-vesper-border bg-vesper-panel p-4 sm:p-5"
    >
      <h2
        id="approvals-heading"
        class="text-sm font-semibold uppercase tracking-[0.14em] text-white"
      >
        Indexing requests
      </h2>
      <Show
        when={session().kind === "unlocked"}
        fallback={
          <form
            class="mt-4 flex flex-wrap items-end gap-3"
            onSubmit={(event) => {
              event.preventDefault();
              void run();
            }}
          >
            <label class="flex flex-col gap-2 text-sm text-vesper-muted">
              Approval passkey
              <input
                type="password"
                autocomplete="off"
                value={passkey()}
                onInput={(event) => setPasskey(event.currentTarget.value)}
                class="min-h-11 border border-vesper-control bg-vesper-bg px-3 text-white"
              />
            </label>
            <button type="submit" class={buttonClass} disabled={!!operation() || !passkey().trim()}>
              {operation() ? "Unlocking…" : "Unlock approvals"}
            </button>
            <p class="w-full text-xs text-vesper-dim">
              Enter your approval password. Kept only until you lock or leave this page.
            </p>
          </form>
        }
      >
        <div class="mt-4 flex gap-3">
          <button
            type="button"
            class={buttonClass}
            disabled={!!operation()}
            onClick={() => void run()}
          >
            Refresh requests
          </button>
          <button type="button" class={buttonClass} onClick={lock}>
            Lock approvals
          </button>
        </div>
        <Show
          when={pending().length}
          fallback={<p class="mt-4 text-sm text-vesper-muted">No requests awaiting approval.</p>}
        >
          <ul class="mt-4 divide-y divide-white/10">
            <For each={pending()}>
              {(request) => (
                <li class="flex flex-wrap items-center justify-between gap-3 py-3">
                  <div>
                    <p class="font-medium text-white">{request.handle}</p>
                    <p class="text-xs text-vesper-muted">
                      Requested {new Date(request.requested_at * 1000).toLocaleString()}
                    </p>
                  </div>
                  <button
                    type="button"
                    class={buttonClass}
                    disabled={!!operation()}
                    aria-label={`Approve ${request.handle}`}
                    onClick={() => void run(request.id)}
                  >
                    {operation() === request.id ? "Approving…" : "Approve"}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
      <Show when={error()}>
        <p role="alert" class="mt-3 text-sm text-vesper-red">
          {error()}
        </p>
      </Show>
    </section>
  );
};
