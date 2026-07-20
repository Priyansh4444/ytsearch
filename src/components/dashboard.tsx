import { For, Show, createMemo, createResource, onCleanup, onMount } from "solid-js";
import type { Component, JSX } from "solid-js";
import { dashboardData } from "../api";
import { calculateDashboardMetrics, channelCoverage } from "../dashboard-metrics";
import type { IndexedChannel } from "../types";
import { routeForHandle } from "../utils";
import { ChannelAvatar } from "./channel-profile";

const number = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });
const integer = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });

export const Dashboard: Component = () => {
  const [snapshot, { refetch }] = createResource(dashboardData);
  const metrics = createMemo(() => calculateDashboardMetrics(snapshot()?.channels ?? []));
  const channels = createMemo(() =>
    [...(snapshot()?.channels ?? [])].sort(
      (left, right) =>
        right.indexed_chunk_count - left.indexed_chunk_count ||
        right.indexed_video_count - left.indexed_video_count,
    ),
  );

  onMount(() => {
    const previousTitle = document.title;
    document.title = "Dashboard · YouTube Search";
    const interval = window.setInterval(() => void refetch(), 60_000);
    onCleanup(() => {
      document.title = previousTitle;
      window.clearInterval(interval);
    });
  });

  return (
    <main class="min-h-svh bg-vesper-bg text-vesper-text">
      <div class="mx-auto flex w-full max-w-7xl flex-col gap-8 px-4 py-5 sm:px-6 sm:py-8 lg:px-8">
        <header class="flex flex-col gap-4 border-b border-white/10 pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div>
            <a
              href="/"
              class="text-xs font-semibold uppercase tracking-[0.16em] text-vesper-yellow hover:text-white"
            >
              ← Search
            </a>
            <h1 class="mt-3 text-3xl font-semibold tracking-tight text-white sm:text-4xl">
              System dashboard
            </h1>
            <p class="mt-2 max-w-2xl text-sm leading-6 text-vesper-muted">
              Live public catalog, indexing coverage, queue activity, metadata completeness, and
              endpoint health. Refreshes every minute.
            </p>
          </div>
          <button
            type="button"
            class="min-h-11 border border-vesper-border bg-vesper-panel px-4 py-2 text-sm font-medium text-white transition-colors hover:border-vesper-yellow disabled:cursor-wait disabled:opacity-60"
            disabled={snapshot.loading}
            onClick={() => void refetch()}
          >
            {snapshot.loading ? "Refreshing…" : "Refresh metrics"}
          </button>
        </header>

        <Show when={snapshot()?.errors.length}>
          <section class="border border-vesper-red/40 bg-vesper-red/5 p-4" role="alert">
            <p class="font-medium text-vesper-red">Some metrics are unavailable</p>
            <For each={snapshot()?.errors ?? []}>
              {(error) => <p class="mt-1 break-words text-sm text-vesper-muted">{error}</p>}
            </For>
          </section>
        </Show>

        <section aria-labelledby="overview-heading">
          <SectionHeading id="overview-heading" title="Overview" detail="Current indexed corpus" />
          <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <MetricCard label="Creators" value={integer.format(metrics().channelCount)} />
            <MetricCard label="Indexed videos" value={integer.format(metrics().indexedVideos)} />
            <MetricCard label="Transcript moments" value={integer.format(metrics().moments)} />
            <MetricCard
              label="Catalog coverage"
              value={`${number.format(metrics().coveragePercent)}%`}
              detail={`${integer.format(metrics().remainingVideos)} videos remaining`}
            />
          </div>
        </section>

        <section aria-labelledby="operations-heading">
          <SectionHeading id="operations-heading" title="Operations" detail="Live service checks" />
          <div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <MetricCard
              label="API"
              value={snapshot()?.apiHealthy ? "Operational" : "Unavailable"}
              tone={snapshot()?.apiHealthy ? "green" : "red"}
              detail={latency(snapshot()?.latencyMs.health)}
            />
            <MetricCard
              label="Catalog API"
              value={snapshot()?.channelsAvailable ? "Online" : "Unavailable"}
              tone={snapshot()?.channelsAvailable ? "green" : "red"}
              detail={latency(snapshot()?.latencyMs.channels)}
            />
            <MetricCard
              label="Indexer"
              value={indexerLabel(snapshot()?.indexStatus?.status, metrics().remainingVideos)}
              tone={indexerTone(snapshot()?.indexStatus?.status, metrics().remainingVideos)}
              detail={indexerDetail(
                snapshot()?.indexStatus?.status,
                snapshot()?.indexStatus?.handle,
                metrics().remainingVideos,
                snapshot()?.latencyMs.indexStatus,
              )}
            />
            <MetricCard
              label="Last refreshed"
              value={snapshot() ? new Date(snapshot()!.fetchedAt).toLocaleTimeString() : "Loading…"}
              detail={navigator.onLine ? "Browser online" : "Browser offline"}
            />
          </div>
        </section>

        <section aria-labelledby="catalog-heading">
          <SectionHeading
            id="catalog-heading"
            title="Catalog diagnostics"
            detail="Derived totals"
          />
          <div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            <SmallMetric label="Public videos reported" value={metrics().publicVideos} />
            <SmallMetric label="Fully indexed creators" value={metrics().completeChannels} />
            <SmallMetric label="Partially indexed creators" value={metrics().partialChannels} />
            <SmallMetric label="Profiles with avatars" value={metrics().avatarCount} />
            <SmallMetric label="Profiles with upload playlists" value={metrics().playlistCount} />
            <SmallMetric
              label="Average videos / creator"
              value={metrics().averageVideosPerChannel}
            />
            <SmallMetric label="Average moments / video" value={metrics().averageMomentsPerVideo} />
            <SmallMetric
              label="Average moments / creator"
              value={metrics().averageMomentsPerChannel}
            />
          </div>
          <div class="mt-3 grid gap-3 lg:grid-cols-2">
            <Distribution
              label="Indexed videos per creator"
              values={metrics().indexedVideoDistribution}
            />
            <Distribution label="Moments per creator" values={metrics().momentDistribution} />
          </div>
        </section>

        <section aria-labelledby="creators-heading">
          <SectionHeading
            id="creators-heading"
            title="Creators"
            detail={`${integer.format(channels().length)} searchable profiles`}
          />
          <Show
            when={!snapshot.loading || channels().length > 0}
            fallback={<div class="border border-vesper-border bg-vesper-panel p-5">Loading…</div>}
          >
            <div class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              <For each={channels()}>{(channel) => <ChannelMetricCard channel={channel} />}</For>
            </div>
          </Show>
        </section>

        <footer class="border-t border-white/10 py-5 text-xs leading-5 text-vesper-dim">
          Counts come from the public search API. Latencies are browser-observed fetch times, not
          backend-only timings. Private Axiom events, Fly machine internals, tokens, and user-level
          search data are intentionally not exposed on this public page.
        </footer>
      </div>
    </main>
  );
};

const SectionHeading: Component<{ id: string; title: string; detail: string }> = (props) => (
  <div class="mb-3 flex flex-wrap items-baseline justify-between gap-2">
    <h2 id={props.id} class="text-sm font-semibold uppercase tracking-[0.14em] text-white">
      {props.title}
    </h2>
    <p class="text-xs text-vesper-dim">{props.detail}</p>
  </div>
);

const MetricCard: Component<{
  label: string;
  value: string;
  detail?: string;
  tone?: "green" | "yellow" | "red";
}> = (props) => (
  <div class="min-w-0 border border-vesper-border bg-vesper-panel p-4 sm:p-5">
    <p class="text-xs uppercase tracking-[0.12em] text-vesper-dim">{props.label}</p>
    <p
      class={`mt-2 break-words text-xl font-semibold sm:text-2xl ${
        props.tone === "green"
          ? "text-vesper-green"
          : props.tone === "red"
            ? "text-vesper-red"
            : props.tone === "yellow"
              ? "text-vesper-yellow"
              : "text-white"
      }`}
    >
      {props.value}
    </p>
    <Show when={props.detail}>
      <p class="mt-1 truncate text-xs text-vesper-muted" title={props.detail}>
        {props.detail}
      </p>
    </Show>
  </div>
);

const SmallMetric: Component<{ label: string; value: number }> = (props) => (
  <div class="flex items-center justify-between gap-4 border border-vesper-border bg-vesper-panel px-4 py-3">
    <span class="text-sm text-vesper-muted">{props.label}</span>
    <span class="shrink-0 font-mono text-sm text-white">{number.format(props.value)}</span>
  </div>
);

const Distribution: Component<{
  label: string;
  values: { min: number; median: number; max: number };
}> = (props) => (
  <div class="border border-vesper-border bg-vesper-panel p-4">
    <p class="text-sm text-vesper-muted">{props.label}</p>
    <dl class="mt-3 grid grid-cols-3 gap-2 text-center">
      <DistributionValue label="Minimum" value={props.values.min} />
      <DistributionValue label="Median" value={props.values.median} />
      <DistributionValue label="Maximum" value={props.values.max} />
    </dl>
  </div>
);

const DistributionValue: Component<{ label: string; value: number }> = (props) => (
  <div>
    <dt class="text-[0.65rem] uppercase tracking-[0.1em] text-vesper-dim">{props.label}</dt>
    <dd class="mt-1 font-mono text-sm text-white">{number.format(props.value)}</dd>
  </div>
);

const ChannelMetricCard: Component<{ channel: IndexedChannel }> = (props) => {
  const coverage = () => channelCoverage(props.channel);
  const remaining = () =>
    Math.max(0, props.channel.public_video_count - props.channel.indexed_video_count);
  const momentsPerVideo = () =>
    props.channel.indexed_video_count > 0
      ? props.channel.indexed_chunk_count / props.channel.indexed_video_count
      : 0;
  return (
    <a
      href={routeForHandle(props.channel.handle)}
      class="group min-w-0 border border-vesper-border bg-vesper-panel p-4 transition-colors hover:border-vesper-yellow focus-visible:border-vesper-yellow focus-visible:outline-none"
    >
      <div class="flex min-w-0 items-center gap-3">
        <ChannelAvatar channel={props.channel} size="compact" />
        <div class="min-w-0 flex-1">
          <p class="truncate font-medium text-white group-hover:text-vesper-yellow">
            {props.channel.name}
          </p>
          <p class="truncate text-xs text-vesper-muted">{props.channel.handle}</p>
        </div>
        <span class="font-mono text-xs text-vesper-green">{number.format(coverage())}%</span>
      </div>
      <div class="mt-4 h-1 overflow-hidden bg-white/10" aria-hidden="true">
        <div class="h-full bg-vesper-green" style={{ width: `${coverage()}%` }} />
      </div>
      <dl class="mt-4 grid grid-cols-2 gap-x-4 gap-y-3 text-xs">
        <ChannelValue
          label="Indexed / public"
          value={`${integer.format(props.channel.indexed_video_count)} / ${integer.format(props.channel.public_video_count)}`}
        />
        <ChannelValue label="Remaining" value={integer.format(remaining())} />
        <ChannelValue label="Moments" value={integer.format(props.channel.indexed_chunk_count)} />
        <ChannelValue label="Moments / video" value={number.format(momentsPerVideo())} />
      </dl>
    </a>
  );
};

const ChannelValue: Component<{ label: string; value: JSX.Element }> = (props) => (
  <div class="min-w-0">
    <dt class="truncate text-vesper-dim">{props.label}</dt>
    <dd class="mt-0.5 truncate font-mono text-white">{props.value}</dd>
  </div>
);

function latency(value?: number): string {
  return value === undefined ? "No response" : `${integer.format(value)} ms browser latency`;
}

function indexerLabel(status: string | undefined, remainingVideos: number): string {
  if (status === "idle") return remainingVideos > 0 ? "Scheduled repair" : "Caught up";
  if (!status) return "Unknown";
  return status.replace(/^./, (character) => character.toUpperCase());
}

function indexerTone(
  status: string | undefined,
  remainingVideos: number,
): "green" | "yellow" | "red" | undefined {
  if (status === "idle") return remainingVideos > 0 ? "yellow" : "green";
  if (status === "running") return "green";
  if (status === "queued") return "yellow";
  if (status === "failed" || status === "rejected") return "red";
  return undefined;
}

function indexerDetail(
  status: string | undefined,
  handle: string | undefined,
  remainingVideos: number,
  latencyMs: number | undefined,
): string {
  if (status === "running" && handle) return handle;
  if (status === "idle" && remainingVideos > 0) {
    return `${integer.format(remainingVideos)} count gap · wakes every 6h`;
  }
  if (status === "idle") return "No public/indexed count gap";
  return latency(latencyMs);
}
