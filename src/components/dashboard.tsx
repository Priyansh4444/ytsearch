import { For, Show, createMemo, createSignal, isPending, onSettled } from "solid-js";
import type { Component } from "solid-js";
import { dashboardData } from "../api";
import {
  calculateDashboardMetrics,
  channelCountDifference,
  publicVideoCount,
  visibleCreators,
} from "../dashboard-metrics";
import type { CreatorFilter, CreatorSort, CreatorView } from "../dashboard-metrics";
import type { IndexedChannel, IndexStatus } from "../types";
import { routeForHandle } from "../utils";
import { ChannelAvatar } from "./channel-profile";

const number = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });
const integer = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });

export const Dashboard: Component = () => {
  const [refreshVersion, setRefreshVersion] = createSignal(0);
  const [view, setView] = createSignal<CreatorView>({
    query: "",
    filter: "all",
    sort: "difference",
  });
  const snapshot = createMemo(() => {
    refreshVersion();
    return dashboardData();
  });
  const snapshotPending = () => isPending(() => snapshot());
  const refetch = () => setRefreshVersion((version) => version + 1);
  const metrics = createMemo(() => calculateDashboardMetrics(snapshot().channels));
  const channels = createMemo(() => visibleCreators(snapshot().channels, view()));
  const catalogNumber = (value: number) =>
    snapshot().channelsAvailable ? integer.format(value) : "—";

  onSettled(() => {
    const previousTitle = document.title;
    document.title = "Dashboard · YouTube Search";
    const refreshVisiblePage = () => {
      if (!document.hidden) refetch();
    };
    const interval = window.setInterval(refreshVisiblePage, 60_000);
    document.addEventListener("visibilitychange", refreshVisiblePage);
    return () => {
      document.title = previousTitle;
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", refreshVisiblePage);
    };
  });

  return (
    <main class="min-h-svh bg-vesper-bg text-vesper-text">
      <div class="mx-auto w-full max-w-7xl px-4 sm:px-6 lg:px-8">
        <header class="flex flex-wrap items-center justify-between gap-4 border-b border-vesper-border py-5">
          <a href="/" class="text-sm font-medium text-vesper-muted hover:text-vesper-yellow">
            ← Back to search
          </a>
          <div class="flex items-center gap-3">
            <span class="hidden text-xs text-vesper-dim sm:inline">
              Public catalog / live snapshot
            </span>
            <button
              type="button"
              class="pressable min-h-11 border border-vesper-control bg-vesper-panel px-4 py-2 text-xs font-medium hover:border-vesper-yellow disabled:cursor-wait disabled:opacity-60"
              disabled={snapshotPending()}
              onClick={refetch}
            >
              {snapshotPending() ? "Refreshing…" : "Refresh metrics"}
            </button>
          </div>
        </header>

        <section class="py-8 sm:py-12" aria-labelledby="dashboard-heading">
          <div class="flex flex-wrap items-baseline justify-between gap-3">
            <h1
              id="dashboard-heading"
              class="text-xs font-semibold uppercase tracking-[0.2em] text-vesper-yellow"
            >
              System dashboard
            </h1>
            <p class="text-xs text-vesper-dim">
              Updates every minute · {new Date(snapshot().fetchedAt).toLocaleTimeString()}
            </p>
          </div>
          <div class="mt-7 grid gap-8 lg:grid-cols-[1.2fr_1fr] lg:gap-16">
            <div>
              <p class="text-sm text-vesper-muted">Indexed videos</p>
              <p class="mt-1 break-words text-6xl font-medium tracking-[-0.06em] tabular-nums sm:text-8xl">
                {catalogNumber(metrics().indexedVideos)}
              </p>
              <p class="mt-4 max-w-md text-sm leading-6 text-vesper-muted">
                A searchable archive of conversations, ideas, and the exact moment they happened.
              </p>
              <dl class="mt-6 flex flex-wrap gap-x-8 gap-y-4">
                <div>
                  <dt class="text-xs text-vesper-dim">Creators</dt>
                  <dd class="mt-1 text-xl tabular-nums">{catalogNumber(metrics().channelCount)}</dd>
                </div>
                <div>
                  <dt class="text-xs text-vesper-dim">Transcript moments</dt>
                  <dd class="mt-1 text-xl tabular-nums">{catalogNumber(metrics().moments)}</dd>
                </div>
              </dl>
            </div>
            <div class="border-l-2 border-vesper-yellow/60 bg-vesper-panel p-5 sm:p-6">
              <h2 class="text-sm font-medium">Understanding the count difference</h2>
              <dl class="mt-5 grid grid-cols-2 gap-4">
                <div>
                  <dt class="text-xs text-vesper-dim">Public videos reported</dt>
                  <dd class="mt-1 text-2xl tabular-nums">
                    {snapshot().channelsAvailable && metrics().knownPublicChannels > 0
                      ? integer.format(metrics().publicVideos)
                      : "Unknown"}
                  </dd>
                </div>
                <div>
                  <dt class="text-xs text-vesper-dim">Count difference</dt>
                  <dd class="mt-1 text-2xl tabular-nums text-vesper-yellow">
                    {snapshot().channelsAvailable
                      ? differenceLabel(metrics().countDifference)
                      : "Unknown"}
                  </dd>
                </div>
              </dl>
              <p class="mt-4 text-xs leading-5 text-vesper-muted">
                Public minus {catalogNumber(metrics().comparableIndexedVideos)} indexed videos,
                across {catalogNumber(metrics().knownPublicChannels)} creators with reported totals.
                A count difference does not identify missing or transcribable videos.
              </p>
              <p class="mt-2 text-xs leading-5 text-vesper-dim">
                Caption availability, private or deleted uploads, Shorts, and different snapshot
                dates can affect comparisons. These counts alone do not tell us which applies.
              </p>
              <Show when={metrics().unknownPublicChannels > 0}>
                <p class="mt-3 text-xs text-vesper-yellow">
                  Public totals unknown for {integer.format(metrics().unknownPublicChannels)}{" "}
                  creators; excluded from this comparison.
                </p>
              </Show>
            </div>
          </div>
        </section>

        <Show when={snapshot().errors.length}>
          <section class="mb-5 border border-vesper-red/40 bg-vesper-red/5 p-4" role="alert">
            <p class="text-sm font-medium text-vesper-red">Some metrics are unavailable</p>
            <For each={snapshot().errors}>
              {(error) => <p class="mt-1 break-words text-xs text-vesper-muted">{error}</p>}
            </For>
          </section>
        </Show>

        <section
          aria-label="Service status"
          class="grid gap-px overflow-hidden border border-vesper-border bg-vesper-border sm:grid-cols-3"
        >
          <ServiceStatus
            label="Search API"
            value={snapshot().apiHealthy ? "Operational" : "Unavailable"}
            healthy={snapshot().apiHealthy}
            detail={latency(snapshot().latencyMs.health)}
          />
          <ServiceStatus
            label="Catalog API"
            value={snapshot().channelsAvailable ? "Online" : "Unavailable"}
            healthy={snapshot().channelsAvailable}
            detail={latency(snapshot().latencyMs.channels)}
          />
          <ServiceStatus
            label="Indexer"
            value={indexerLabel(snapshot().indexStatus)}
            healthy={indexerHealthy(snapshot().indexStatus)}
            detail={snapshot().indexStatus?.handle || latency(snapshot().latencyMs.indexStatus)}
          />
        </section>

        <section aria-labelledby="creators-heading" class="py-8 sm:py-10">
          <div class="flex flex-wrap items-baseline justify-between gap-3">
            <h2 id="creators-heading" class="text-xl font-medium tracking-tight">
              Creator directory
            </h2>
            <p class="text-xs text-vesper-dim" role="status">
              {snapshot().channelsAvailable
                ? `${integer.format(channels().length)} of ${integer.format(metrics().channelCount)} creators`
                : "Catalog unavailable"}
            </p>
          </div>
          <div class="mt-5 grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto_auto]">
            <label class="min-w-0">
              <span class="sr-only">Find a creator</span>
              <input
                type="search"
                placeholder="Find a creator or @handle…"
                value={view().query}
                onInput={(event) =>
                  setView((previous) => ({ ...previous, query: event.currentTarget.value }))
                }
                class="min-h-11 w-full min-w-0 border border-vesper-control bg-vesper-panel px-3 text-sm placeholder:text-vesper-dim"
              />
            </label>
            <label class="flex min-w-0 items-center gap-2 text-xs text-vesper-muted">
              <span>Show</span>
              <select
                aria-label="Filter creators"
                value={view().filter}
                onChange={(event) =>
                  setView((previous) => ({
                    ...previous,
                    filter: event.currentTarget.value as CreatorFilter,
                  }))
                }
                class="min-h-11 min-w-0 flex-1 border border-vesper-control bg-vesper-panel px-3 text-sm"
              >
                <option value="all">All creators</option>
                <option value="difference">Count differences</option>
                <option value="unknown">Unknown public totals</option>
              </select>
            </label>
            <label class="flex min-w-0 items-center gap-2 text-xs text-vesper-muted">
              <span>Sort</span>
              <select
                aria-label="Sort creators"
                value={view().sort}
                onChange={(event) =>
                  setView((previous) => ({
                    ...previous,
                    sort: event.currentTarget.value as CreatorSort,
                  }))
                }
                class="min-h-11 min-w-0 flex-1 border border-vesper-control bg-vesper-panel px-3 text-sm"
              >
                <option value="difference">Count difference</option>
                <option value="videos">Indexed videos</option>
                <option value="moments">Transcript moments</option>
                <option value="name">Name A–Z</option>
              </select>
            </label>
          </div>
          <Show
            when={snapshot().channelsAvailable}
            fallback={
              <p class="mt-4 border border-vesper-border p-6 text-sm text-vesper-muted">
                Creator data is unavailable. Refresh to try again.
              </p>
            }
          >
            <Show
              when={channels().length > 0}
              fallback={
                <p class="mt-4 border border-vesper-border p-6 text-sm text-vesper-muted">
                  {view().query || view().filter !== "all"
                    ? "No creators match these filters."
                    : "No creators in the public catalog yet."}
                </p>
              }
            >
              <table class="mt-4 w-full border-collapse text-sm" aria-label="Creator catalog">
                <thead class="hidden border-y border-vesper-border text-left text-[11px] uppercase tracking-wider text-vesper-dim md:table-header-group">
                  <tr>
                    <th scope="col" class="py-3 font-medium">
                      Creator
                    </th>
                    <th scope="col" class="px-3 py-3 text-right font-medium">
                      Indexed / public
                    </th>
                    <th scope="col" class="px-3 py-3 text-right font-medium">
                      Difference
                    </th>
                    <th scope="col" class="py-3 text-right font-medium">
                      Moments
                    </th>
                  </tr>
                </thead>
                <tbody class="block md:table-row-group">
                  <For each={channels()}>{(channel) => <CreatorRow channel={channel} />}</For>
                </tbody>
              </table>
            </Show>
          </Show>
          <p class="mt-4 text-xs leading-5 text-vesper-dim">
            Difference = reported public videos − indexed videos. Negative values mean the index has
            more videos than the reported public total; equal counts do not establish complete
            coverage.
          </p>
        </section>

        <details class="border-y border-vesper-border py-4">
          <summary class="cursor-pointer text-sm text-vesper-muted hover:text-white">
            Catalog diagnostics{" "}
            <span class="ml-2 text-xs text-vesper-dim">Metadata & averages</span>
          </summary>
          <dl class="mt-5 grid gap-x-8 gap-y-4 pb-2 sm:grid-cols-2 lg:grid-cols-4">
            <Diagnostic
              label="Profiles with avatars"
              value={catalogNumber(metrics().avatarCount)}
            />
            <Diagnostic
              label="Upload playlists known"
              value={catalogNumber(metrics().playlistCount)}
            />
            <Diagnostic
              label="Average videos / creator"
              value={
                snapshot().channelsAvailable
                  ? number.format(metrics().averageVideosPerChannel)
                  : "Unknown"
              }
            />
            <Diagnostic
              label="Average moments / video"
              value={
                snapshot().channelsAvailable
                  ? number.format(metrics().averageMomentsPerVideo)
                  : "Unknown"
              }
            />
          </dl>
        </details>
        <footer class="py-6 text-xs leading-5 text-vesper-dim">
          Public search API snapshots. Latencies are browser-observed fetch times. Queue status
          describes reported activity, not catalog completeness.
        </footer>
      </div>
    </main>
  );
};

const ServiceStatus: Component<{
  label: string;
  value: string;
  healthy: boolean | undefined;
  detail: string;
}> = (props) => (
  <div class="flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-1 bg-vesper-panel px-4 py-3">
    <div class="flex items-center gap-2 text-xs">
      <span
        aria-hidden="true"
        class={`size-1.5 rounded-full ${props.healthy === true ? "bg-vesper-green" : props.healthy === false ? "bg-vesper-red" : "bg-vesper-dim"}`}
      />
      <span class="text-vesper-muted">{props.label}</span>
      <span>{props.value}</span>
    </div>
    <p class="w-full truncate pl-3.5 text-[11px] text-vesper-dim" title={props.detail}>
      {props.detail}
    </p>
  </div>
);

const CreatorRow: Component<{ channel: IndexedChannel }> = (props) => (
  <tr class="grid grid-cols-2 gap-x-4 gap-y-4 border-b border-vesper-border py-5 md:table-row md:hover:bg-vesper-panel">
    <th scope="row" class="col-span-2 min-w-0 text-left font-normal md:max-w-64 md:py-4">
      <a
        href={routeForHandle(props.channel.handle)}
        class="group flex min-h-11 min-w-0 items-center gap-3"
      >
        <ChannelAvatar channel={props.channel} size="compact" />
        <span class="min-w-0">
          <span class="block truncate font-medium group-hover:text-vesper-yellow">
            {props.channel.name}
          </span>
          <span class="block truncate text-xs text-vesper-dim">{props.channel.handle}</span>
        </span>
        <span
          aria-hidden="true"
          class="ml-auto pr-2 text-vesper-dim group-hover:text-vesper-yellow md:hidden"
        >
          ↗
        </span>
      </a>
    </th>
    <td class="min-w-0 md:px-3 md:py-4 md:text-right">
      <span class="mb-1 block text-[11px] text-vesper-dim md:hidden">Indexed / public</span>
      <span class="font-mono text-xs">
        {integer.format(props.channel.indexed_video_count)} /{" "}
        {publicVideoCount(props.channel) === null
          ? "Unknown"
          : integer.format(props.channel.public_video_count)}
      </span>
    </td>
    <td class="min-w-0 md:px-3 md:py-4 md:text-right">
      <span class="mb-1 block text-[11px] text-vesper-dim md:hidden">Count difference</span>
      <span
        class={`font-mono text-xs ${channelCountDifference(props.channel) ? "text-vesper-yellow" : "text-vesper-muted"}`}
      >
        {differenceLabel(channelCountDifference(props.channel))}
      </span>
    </td>
    <td class="col-span-2 flex items-center justify-between md:table-cell md:py-4 md:text-right">
      <span class="text-[11px] text-vesper-dim md:hidden">Transcript moments</span>
      <span class="font-mono text-xs text-vesper-muted">
        {integer.format(props.channel.indexed_chunk_count)}
      </span>
    </td>
  </tr>
);

const Diagnostic: Component<{ label: string; value: string }> = (props) => (
  <div>
    <dt class="text-xs text-vesper-dim">{props.label}</dt>
    <dd class="mt-1 font-mono text-sm">{props.value}</dd>
  </div>
);

function differenceLabel(value: number | null): string {
  if (value === null) return "Unknown";
  return `${value > 0 ? "+" : ""}${integer.format(value)}`;
}

function latency(value?: number): string {
  return value === undefined ? "No response" : `${integer.format(value)} ms browser latency`;
}

function indexerLabel(status: IndexStatus | undefined): string {
  if (!status) return "Unknown";
  return status.status.replace(/^./, (character) => character.toUpperCase());
}

function indexerHealthy(status: IndexStatus | undefined): boolean | undefined {
  if (!status) return undefined;
  if (status.status === "failed" || status.status === "rejected") return false;
  if (status.status === "running") return true;
  return undefined;
}
