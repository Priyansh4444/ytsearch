import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import type { Component } from "solid-js";
import {
  clearIndexStatusCache,
  fetchSearchAvailability,
  indexStatus,
  indexedChannels,
  requestIndex,
  searchTranscripts,
} from "./api";
import {
  capture,
  captureSearchLoaded,
  captureSearchSubmitted,
  initializeAnalytics,
} from "./analytics";
import { ChannelPicker } from "./components/channel-picker";
import { AvailabilityNotice } from "./components/availability-notice";
import { ChannelAvatar } from "./components/channel-profile";
import {
  channelMentionQuery,
  channelSuggestions as findChannelSuggestions,
  resolveChannelMention,
} from "./channel-query";
import { ChannelIdentity } from "./components/channel-profile";
import { IndexRequestDialog } from "./components/index-request-dialog";
import { PaginationControls } from "./components/pagination-controls";
import { RecentSearches } from "./components/recent-searches";
import { SearchForm } from "./components/search-form";
import {
  EmptyState,
  ErrorState,
  IndexRequestState,
  LoadingState,
  WelcomeState,
} from "./components/states";
import { VideoResult } from "./components/video-result";
import { BackButton, CopyLinkButton } from "./components/toolbar-actions";
import type {
  IndexedChannel,
  RecentSearch,
  SearchAvailability,
  SearchMode,
  SearchRequest,
  SearchSort,
  SearchVideoResult,
} from "./types";
import { isAvailableSearchSort } from "./search-sorts";
import { cacheChannels, loadCachedChannels } from "./channel-cache";
import { clearRecentSearches, loadRecentSearches, saveRecentSearch } from "./recent-searches";
import { evaluateSearchSchedule } from "../shared/service-schedule";
import {
  copyToClipboard,
  creatorFromPath,
  normalizeHandle,
  routeForHandle,
  withViewTransition,
} from "./utils";

const pluralize = (count: number, singular: string, plural = `${singular}s`) =>
  `${count} ${count === 1 ? singular : plural}`;

const videoCountLabel = (returned: number, total: number) =>
  returned < total ? `${returned} shown of ${total} videos` : pluralize(total, "video");

const sectionVideos = (videos: SearchVideoResult[], kind: "video" | "short") =>
  videos.filter((video) => video.content_kind === kind);

const unknownVideos = (videos: SearchVideoResult[]) =>
  videos.filter((video) => video.content_kind === "unknown");

const semanticSearchEnabled = import.meta.env.VITE_SEMANTIC_SEARCH_ENABLED === "true";
const supportUrl = import.meta.env.VITE_KOFI_URL?.trim() || "https://ko-fi.com/pronsh";

const provisionalAvailability = (now = new Date()): SearchAvailability => {
  const schedule = evaluateSearchSchedule(now);
  return {
    status:
      schedule.phase === "open"
        ? "starting"
        : schedule.phase === "prewarm"
          ? "prewarming"
          : "scheduled_offline",
    search_enabled: schedule.phase === "open",
    enforced: true,
    schedule_phase: schedule.phase,
    time_zone: "America/Los_Angeles",
    daily_window: {
      opens: "12:00",
      closes: "00:00",
      label: "12:00 PM–12:00 AM Pacific Time",
    },
    next_transition_at: schedule.nextTransitionAt,
  };
};

const searchStateFromLocation = (): SearchRequest | undefined => {
  const params = new URLSearchParams(window.location.search);
  const query = params.get("q")?.trim();
  if (!query) return undefined;
  const sort = (params.get("sort") ?? "relevance") as SearchSort;
  const mode = (params.get("mode") ?? "lexical") as SearchMode;
  const page = Number(params.get("page") ?? "1");
  return {
    query,
    youtuber: normalizeHandle(params.get("youtuber") ?? creatorFromPath() ?? ""),
    page: Number.isFinite(page) && page > 0 ? Math.floor(page) : 1,
    sort: isAvailableSearchSort(sort) ? sort : "relevance",
    mode: mode === "semantic" ? "semantic" : "lexical",
  };
};

const writeSearchUrl = (request: SearchRequest) => {
  const params = new URLSearchParams();
  params.set("q", request.query);
  if (request.youtuber) params.set("youtuber", normalizeHandle(request.youtuber));
  if (request.sort !== "relevance") params.set("sort", request.sort);
  if (request.mode !== "lexical") params.set("mode", request.mode);
  if (request.page > 1) params.set("page", String(request.page));
  window.history.pushState({}, "", `/?${params.toString()}`);
};

const App: Component = () => {
  const initialSearch = searchStateFromLocation();
  const [routeHandle, setRouteHandle] = createSignal(
    initialSearch?.youtuber ? normalizeHandle(initialSearch.youtuber) : creatorFromPath(),
  );
  const [channels, setChannels] = createSignal<IndexedChannel[] | undefined>(loadCachedChannels());
  const refreshChannels = async () => {
    const refreshed = await indexedChannels();
    setChannels(refreshed);
    cacheChannels(refreshed);
    return refreshed;
  };
  const [availability, setAvailability] = createSignal(provisionalAvailability());
  const [recentSearches, setRecentSearches] = createSignal(loadRecentSearches());
  const [query, setQuery] = createSignal(initialSearch?.query ?? "");
  const [youtuber, setYoutuber] = createSignal(initialSearch?.youtuber ?? routeHandle() ?? "");
  const [page, setPage] = createSignal(initialSearch?.page ?? 1);
  const [sort, setSort] = createSignal<SearchSort>(initialSearch?.sort ?? "relevance");
  const [searchMode, setSearchMode] = createSignal<SearchMode>(initialSearch?.mode ?? "lexical");
  const [submitted, setSubmitted] = createSignal<SearchRequest | undefined>(initialSearch);
  const [pendingIndexHandle, setPendingIndexHandle] = createSignal<string>();
  const [indexRequestError, setIndexRequestError] = createSignal<string>();
  const [indexRequestBusy, setIndexRequestBusy] = createSignal(false);
  const [queuedIndexRequest, setQueuedIndexRequest] = createSignal<{
    handle: string;
    requestId: string;
  }>();
  const [indexingToast, setIndexingToast] = createSignal<string>();
  const [indexingChannelPreview, setIndexingChannelPreview] = createSignal<IndexedChannel>();
  const executableSearch = createMemo(() =>
    availability().search_enabled ? submitted() : undefined,
  );
  const [result] = createResource(executableSearch, (request) => searchTranscripts(request));

  const selectedChannel = createMemo(() => {
    const selectedHandle = normalizeHandle(youtuber());
    return channels()?.find((channel) => normalizeHandle(channel.handle) === selectedHandle);
  });
  const indexingChannel = createMemo(
    () =>
      channels()?.find(
        (channel) => normalizeHandle(channel.handle) === normalizeHandle(indexingToast() ?? ""),
      ) ?? indexingChannelPreview(),
  );
  const hasChannel = () => youtuber().length > 0;
  const hasSearchSurface = () => hasChannel() || submitted() !== undefined;
  const showIndexedChannels = () =>
    !hasChannel() && query().trim().length === 0 && submitted() === undefined;
  const directChannel = createMemo(() => {
    const handle = normalizeHandle(query());
    return channels()?.find((channel) => normalizeHandle(channel.handle) === handle);
  });
  const channelSuggestions = createMemo(() => findChannelSuggestions(channels() ?? [], query()));
  const mentionedChannel = createMemo(() => resolveChannelMention(channels() ?? [], query()));
  const mentionQuery = () => channelMentionQuery(query());
  const isIndexMode = () => query().trim().startsWith("@") && !mentionedChannel();
  const formMode = () => {
    if (mentionedChannel() && !mentionQuery()) return "channel";
    return isIndexMode() ? "request" : "search";
  };

  const syncRoute = () =>
    withViewTransition(() => {
      const searchState = searchStateFromLocation();
      const nextHandle = creatorFromPath();
      setRouteHandle(searchState?.youtuber ? normalizeHandle(searchState.youtuber) : nextHandle);
      setQuery(searchState?.query ?? "");
      setYoutuber(searchState?.youtuber ?? nextHandle ?? "");
      setSort(searchState?.sort ?? "relevance");
      setSearchMode(searchState?.mode ?? "lexical");
      setPage(searchState?.page ?? 1);
      setSubmitted(searchState);
      setPendingIndexHandle(undefined);
      setIndexRequestError(undefined);
      setQueuedIndexRequest(undefined);
      setIndexingToast(undefined);
    });

  onMount(() => {
    initializeAnalytics();
    window.addEventListener("popstate", syncRoute);
    let disposed = false;
    let shownRequestId = "";
    let previousStatus = "idle";
    let toastTimeout: number | undefined;
    const refreshAvailability = () => {
      setAvailability(provisionalAvailability());
      void fetchSearchAvailability()
        .then((value) => {
          if (disposed) return;
          setAvailability(value);
          if (value.search_enabled) void refreshChannels().catch(() => undefined);
        })
        .catch(() => undefined);
    };
    const checkStatus = () => {
      clearIndexStatusCache();
      void indexStatus()
        .then((status) => {
          if (disposed) return;
          if (availability().search_enabled && status.status !== previousStatus)
            void refreshChannels().catch(() => undefined);
          if (availability().search_enabled && ["running", "queued"].includes(status.status))
            void refreshChannels().catch(() => undefined);
          previousStatus = status.status;
          if (
            !["running", "queued"].includes(status.status) ||
            !status.request_id ||
            status.request_id === shownRequestId
          )
            return;
          shownRequestId = status.request_id;
          setIndexingToast(status.handle);
          setIndexingChannelPreview(status.channel);
          if (toastTimeout) window.clearTimeout(toastTimeout);
          toastTimeout = window.setTimeout(() => setIndexingToast(undefined), 5_000);
        })
        .catch(() => undefined);
    };
    refreshAvailability();
    checkStatus();
    if (availability().search_enabled) void refreshChannels().catch(() => undefined);
    const interval = window.setInterval(checkStatus, 15_000);
    const availabilityInterval = window.setInterval(refreshAvailability, 60_000);
    onCleanup(() => {
      disposed = true;
      window.removeEventListener("popstate", syncRoute);
      window.clearInterval(interval);
      window.clearInterval(availabilityInterval);
      if (toastTimeout) window.clearTimeout(toastTimeout);
    });
  });

  const selectChannel = (handle: string) => {
    const normalized = normalizeHandle(handle);
    capture("channel_opened", { handle: normalized, source: "channel_picker" });
    withViewTransition(() => {
      setYoutuber(normalized);
      setRouteHandle(normalized);
      setQuery("");
      setSubmitted(undefined);
      setPendingIndexHandle(undefined);
      setIndexRequestError(undefined);
      setQueuedIndexRequest(undefined);
      setIndexingToast(undefined);
      setPage(1);
      window.history.pushState({}, "", routeForHandle(normalized));
    });
    void indexStatus(normalized)
      .then((status) => {
        if (
          ["pending", "queued", "running", "failed", "rejected"].includes(status.status) &&
          status.request_id
        ) {
          setQueuedIndexRequest({
            handle: normalized,
            requestId: status.request_id,
          });
        }
      })
      .catch(() => undefined);
  };

  const confirmIndexRequest = async () => {
    const handle = pendingIndexHandle();
    if (!handle || indexRequestBusy()) return;
    setIndexRequestBusy(true);
    setIndexRequestError(undefined);
    try {
      const request = await requestIndex(handle);
      capture("index_request_queued", { handle });
      setQueuedIndexRequest({ handle, requestId: request.id });
      setPendingIndexHandle(undefined);
      setQuery(handle);
      setSubmitted(undefined);
    } catch (error) {
      setIndexRequestError(error instanceof Error ? error.message : "Could not queue this channel");
    } finally {
      setIndexRequestBusy(false);
    }
  };

  const runSearch = (nextPage = 1) => {
    const normalizedQuery = query().trim();
    if (!normalizedQuery) return;
    if (directChannel()) {
      selectChannel(directChannel()!.handle);
      return;
    }
    if (mentionedChannel()) {
      const suffix = mentionQuery();
      if (!suffix) {
        selectChannel(mentionedChannel()!.handle);
        return;
      }
      const request = {
        query: suffix,
        youtuber: mentionedChannel()!.handle,
        page: nextPage,
        sort: sort(),
        mode: searchMode(),
      };
      captureSearchSubmitted(request);
      setYoutuber(request.youtuber);
      setQuery(request.query);
      setPage(nextPage);
      setSubmitted(request);
      writeSearchUrl(request);
      return;
    }
    if (isIndexMode()) {
      setIndexRequestError(undefined);
      setPendingIndexHandle(normalizeHandle(normalizedQuery.split(/\s+/, 1)[0] ?? normalizedQuery));
      return;
    }
    if (!availability().search_enabled) return;
    const request = {
      query: normalizedQuery,
      youtuber: youtuber(),
      page: nextPage,
      sort: sort(),
      mode: searchMode(),
    };
    setPendingIndexHandle(undefined);
    captureSearchSubmitted(request);
    const current = submitted();
    if (
      current?.query === request.query &&
      current.youtuber === request.youtuber &&
      current.page === request.page &&
      current.sort === request.sort &&
      current.mode === request.mode
    ) {
      return;
    }
    setQuery(normalizedQuery);
    setQueuedIndexRequest(undefined);
    setPage(nextPage);
    setSubmitted(request);
    writeSearchUrl(request);
  };

  const goToPage = (nextPage: number) => {
    const current = submitted();
    if (!current) return;
    const request = { ...current, page: nextPage };
    captureSearchSubmitted(request);
    setPage(nextPage);
    setSubmitted(request);
    writeSearchUrl(request);
  };

  const goBack = () => {
    withViewTransition(() => {
      window.history.pushState({}, "", "/");
      setRouteHandle(null);
      setQuery("");
      setYoutuber("");
      setSort("relevance");
      setPage(1);
      setSubmitted(undefined);
      setPendingIndexHandle(undefined);
      setIndexRequestError(undefined);
      setQueuedIndexRequest(undefined);
    });
  };

  const [copiedSearchLink, setCopiedSearchLink] = createSignal(false);
  const copySearchLink = async () => {
    await copyToClipboard(window.location.href);
    capture("search_link_copied", {
      has_query: Boolean(submitted()),
      has_channel: hasChannel(),
    });
    setCopiedSearchLink(true);
    window.setTimeout(() => setCopiedSearchLink(false), 1_500);
  };

  createEffect(() => {
    const normalizedQuery = query().trim();
    const searchQuery = mentionedChannel() ? mentionQuery() : normalizedQuery;
    const currentFormMode =
      normalizedQuery.startsWith("@") && !mentionedChannel() ? "index" : "search";
    const currentYoutuber = mentionedChannel()?.handle ?? youtuber();
    const currentSort = sort();
    const currentSearchMode = searchMode();
    const currentSubmitted = submitted();
    if (currentFormMode !== "search" || searchQuery.length < 2 || !availability().search_enabled)
      return;
    if (
      currentSubmitted?.query === searchQuery &&
      currentSubmitted.youtuber === currentYoutuber &&
      currentSubmitted.sort === currentSort &&
      currentSubmitted.mode === currentSearchMode &&
      currentSubmitted.page === 1
    ) {
      return;
    }

    const controller = new AbortController();
    const timeout = window.setTimeout(() => {
      void searchTranscripts(
        {
          query: searchQuery,
          youtuber: currentYoutuber,
          page: 1,
          sort: currentSort,
          mode: currentSearchMode,
        },
        { signal: controller.signal },
      ).catch((error) => {
        if (error instanceof DOMException && error.name === "AbortError") return;
      });
    }, 900);

    onCleanup(() => {
      window.clearTimeout(timeout);
      controller.abort();
    });
  });

  let lastTrackedResponse = "";
  createEffect(() => {
    const request = submitted();
    const response = result();
    if (!request || !response) return;
    const key = `${request.query}\u0000${request.youtuber}\u0000${request.sort}\u0000${request.mode}\u0000${request.page}`;
    if (key === lastTrackedResponse) return;
    lastTrackedResponse = key;
    captureSearchLoaded(request, response);
    if (request.page === 1) {
      setRecentSearches((current) => saveRecentSearch(current, request));
    }
  });

  const selectRecentSearch = (recent: RecentSearch) => {
    setQuery(recent.query);
    setYoutuber(recent.youtuber);
    setSort(recent.sort);
    setSearchMode(recent.mode);
    setPage(1);
    if (!availability().search_enabled) {
      setSubmitted(undefined);
      return;
    }
    const request: SearchRequest = { ...recent, page: 1 };
    captureSearchSubmitted(request);
    setSubmitted(request);
    writeSearchUrl(request);
  };

  return (
    <main class="flex min-h-svh flex-col overflow-clip bg-vesper-bg text-vesper-text">
      <div class="mx-auto flex w-full max-w-7xl flex-1 flex-col gap-6 px-4 py-4 sm:px-6 sm:py-6 lg:px-8">
        <section
          class={`relative ${
            hasSearchSurface() ? "flex flex-col gap-4" : "flex flex-1 flex-col justify-center gap-5"
          }`}
        >
          <Show when={hasChannel()}>
            <div class="flex items-center justify-between gap-3">
              <div class="flex min-w-0 items-center gap-3">
                <BackButton onClick={goBack} />
                <ChannelIdentity channel={selectedChannel()} handle={youtuber()} />
              </div>
              <CopyLinkButton copied={copiedSearchLink()} onClick={() => void copySearchLink()} />
            </div>
          </Show>

          <Show when={!hasSearchSurface()}>
            <ChannelPicker
              channels={channels()}
              visible={showIndexedChannels()}
              onSelect={selectChannel}
            />
          </Show>

          <SearchForm
            query={query()}
            sort={sort()}
            searchMode={searchMode()}
            semanticSearchEnabled={semanticSearchEnabled}
            searchAvailable={availability().search_enabled}
            home={!hasSearchSurface()}
            selectedChannelName={selectedChannel()?.name}
            mode={formMode()}
            onQueryInput={setQuery}
            onSortChange={setSort}
            onSearchModeChange={setSearchMode}
            onSubmit={() => runSearch(1)}
            channelSuggestions={channelSuggestions()}
            onChannelSelect={selectChannel}
          />
          <AvailabilityNotice availability={availability()} />
          <Show when={!hasSearchSurface()}>
            <RecentSearches
              searches={recentSearches()}
              onSelect={selectRecentSearch}
              onClear={() => setRecentSearches(clearRecentSearches())}
            />
          </Show>
        </section>

        <section class={hasSearchSurface() ? "pb-8" : "hidden"}>
          <Show when={queuedIndexRequest()}>
            {(request) => (
              <IndexRequestState handle={request().handle} requestId={request().requestId} />
            )}
          </Show>

          <Show when={!queuedIndexRequest() && availability().search_enabled && result.loading}>
            <LoadingState />
          </Show>

          <Show when={!queuedIndexRequest() && availability().search_enabled && result.error}>
            {(error) => <ErrorState message={String(error())} />}
          </Show>

          <Show
            when={
              !queuedIndexRequest() &&
              availability().search_enabled &&
              submitted() &&
              !result.loading &&
              !result.error &&
              result()
            }
          >
            {(response) => (
              <div class="flex flex-col gap-5">
                <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-white/10 pb-3 text-sm text-vesper-muted">
                  <div class="flex flex-wrap items-center gap-2">
                    <span class="font-medium text-white">{response().query}</span>
                    <span class="border border-white/10 bg-vesper-panel px-2 py-0.5 text-xs uppercase tracking-[0.12em] text-vesper-dim">
                      {submitted()?.youtuber
                        ? `Channel: ${normalizeHandle(submitted()!.youtuber)}`
                        : "All channels"}
                    </span>
                  </div>
                  <span class="text-xs tabular-nums sm:text-sm">
                    {videoCountLabel(
                      response().videos.length,
                      response().total_videos ?? response().videos.length,
                    )}{" "}
                    · {pluralize(response().total_moments ?? response().total_found, "moment")} ·{" "}
                    {response().time_to_first_result_ms} ms to first result
                  </span>
                </div>

                <Show
                  when={response().videos.length > 0}
                  fallback={<EmptyState query={response().query} />}
                >
                  <div class="flex flex-col gap-7">
                    <ResultSection
                      title="Videos"
                      videos={[
                        ...sectionVideos(response().videos, "video"),
                        ...unknownVideos(response().videos),
                      ]}
                      query={response().query}
                      totalLabel="current page"
                    />
                    <ResultSection
                      title="Shorts"
                      videos={sectionVideos(response().videos, "short")}
                      query={response().query}
                      totalLabel="current page"
                    />
                  </div>

                  <PaginationControls
                    response={response()}
                    currentPage={page()}
                    onPageChange={goToPage}
                  />
                </Show>
              </div>
            )}
          </Show>

          <Show
            when={
              hasSearchSurface() &&
              !queuedIndexRequest() &&
              !submitted() &&
              !result.loading &&
              !result.error
            }
          >
            <WelcomeState />
          </Show>
        </section>
      </div>
      <Show when={pendingIndexHandle()}>
        {(handle) => (
          <IndexRequestDialog
            handle={handle()}
            busy={indexRequestBusy()}
            error={indexRequestError()}
            onCancel={() => {
              setPendingIndexHandle(undefined);
              setIndexRequestError(undefined);
            }}
            onConfirm={() => void confirmIndexRequest()}
          />
        )}
      </Show>
      <Show when={indexingToast()}>
        {(handle) => (
          <div
            class="fixed end-5 bottom-5 z-50 flex max-w-[calc(100vw-2.5rem)] items-center gap-3 border border-vesper-green bg-vesper-panel px-4 py-3 text-sm text-white shadow-xl"
            role="status"
          >
            <ChannelAvatar channel={indexingChannel()} size="compact" />
            <span class="min-w-0">
              <span class="block text-xs uppercase tracking-[0.12em] text-vesper-green">
                Currently indexing
              </span>
              <span class="block truncate font-medium">{indexingChannel()?.name ?? handle()}</span>
            </span>
          </div>
        )}
      </Show>
      <footer class="px-4 py-5 text-center text-sm sm:px-6 lg:px-8">
        <a
          href={supportUrl}
          target="_blank"
          rel="noreferrer"
          class="text-vesper-muted transition-colors hover:text-vesper-yellow"
        >
          Support on Ko-fi ↗
        </a>
      </footer>
    </main>
  );
};

export default App;

const ResultSection: Component<{
  title: string;
  videos: SearchVideoResult[];
  query: string;
  totalLabel: string;
}> = (props) => (
  <Show when={props.videos.length > 0}>
    <section class="flex flex-col gap-3">
      <div class="border-b border-white/10 pb-2 text-sm font-medium uppercase tracking-[0.12em] text-vesper-dim">
        {props.title} · {props.videos.length} on {props.totalLabel}
      </div>
      <div class="grid gap-2">
        <For each={props.videos}>
          {(video, index) => <VideoResult video={video} query={props.query} rank={index() + 1} />}
        </For>
      </div>
    </section>
  </Show>
);
