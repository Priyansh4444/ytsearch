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
import { indexStatus, indexedChannels, requestIndex, searchTranscripts } from "./api";
import {
  capture,
  captureSearchLoaded,
  captureSearchSubmitted,
  initializeAnalytics,
} from "./analytics";
import { ChannelPicker } from "./components/ChannelPicker";
import {
  channelMentionQuery,
  channelSuggestions as findChannelSuggestions,
  resolveChannelMention,
} from "./channel-query";
import { ChannelIdentity } from "./components/ChannelProfile";
import { IndexRequestDialog } from "./components/IndexRequestDialog";
import { PaginationControls } from "./components/PaginationControls";
import { SearchForm } from "./components/SearchForm";
import {
  EmptyState,
  ErrorState,
  IndexQueuedState,
  LoadingState,
  WelcomeState,
} from "./components/States";
import { VideoResult } from "./components/VideoResult";
import { BackButton, CopyLinkButton } from "./components/ToolbarActions";
import type { SearchRequest, SearchSort, SearchVideoResult } from "./types";
import {
  copyToClipboard,
  creatorFromPath,
  normalizeHandle,
  routeForHandle,
  withViewTransition,
} from "./utils";

const pluralize = (count: number, singular: string, plural = `${singular}s`) =>
  `${count} ${count === 1 ? singular : plural}`;

const sectionVideos = (videos: SearchVideoResult[], kind: "video" | "short") =>
  videos.filter((video) => video.content_kind === kind);

const unknownVideos = (videos: SearchVideoResult[]) =>
  videos.filter((video) => video.content_kind === "unknown");

const searchStateFromLocation = (): SearchRequest | undefined => {
  const params = new URLSearchParams(window.location.search);
  const query = params.get("q")?.trim();
  if (!query) return undefined;
  const sort = (params.get("sort") ?? "relevance") as SearchSort;
  const page = Number(params.get("page") ?? "1");
  return {
    query,
    youtuber: normalizeHandle(params.get("youtuber") ?? creatorFromPath() ?? ""),
    page: Number.isFinite(page) && page > 0 ? Math.floor(page) : 1,
    sort: ["relevance", "published_desc", "published_asc"].includes(sort) ? sort : "relevance",
  };
};

const writeSearchUrl = (request: SearchRequest) => {
  const params = new URLSearchParams();
  params.set("q", request.query);
  if (request.youtuber) params.set("youtuber", normalizeHandle(request.youtuber));
  if (request.sort !== "relevance") params.set("sort", request.sort);
  if (request.page > 1) params.set("page", String(request.page));
  window.history.pushState({}, "", `/?${params.toString()}`);
};

const App: Component = () => {
  const initialSearch = searchStateFromLocation();
  const [routeHandle, setRouteHandle] = createSignal(
    initialSearch?.youtuber ? normalizeHandle(initialSearch.youtuber) : creatorFromPath(),
  );
  const [channels] = createResource(indexedChannels);
  const [query, setQuery] = createSignal(initialSearch?.query ?? "");
  const [youtuber, setYoutuber] = createSignal(initialSearch?.youtuber ?? routeHandle() ?? "");
  const [page, setPage] = createSignal(initialSearch?.page ?? 1);
  const [sort, setSort] = createSignal<SearchSort>(initialSearch?.sort ?? "relevance");
  const [submitted, setSubmitted] = createSignal<SearchRequest | undefined>(initialSearch);
  const [pendingIndexHandle, setPendingIndexHandle] = createSignal<string>();
  const [indexRequestError, setIndexRequestError] = createSignal<string>();
  const [indexRequestBusy, setIndexRequestBusy] = createSignal(false);
  const [queuedIndexRequest, setQueuedIndexRequest] = createSignal<{
    handle: string;
    requestId: string;
  }>();
  const [result] = createResource(submitted, (request) => searchTranscripts(request));

  const selectedChannel = createMemo(() => {
    const selectedHandle = normalizeHandle(youtuber());
    return channels()?.find((channel) => normalizeHandle(channel.handle) === selectedHandle);
  });
  const hasChannel = () => youtuber().length > 0;
  const hasSearchSurface = () => hasChannel() || submitted() !== undefined;
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
      setPage(searchState?.page ?? 1);
      setSubmitted(searchState);
      setPendingIndexHandle(undefined);
      setIndexRequestError(undefined);
      setQueuedIndexRequest(undefined);
    });

  onMount(() => {
    initializeAnalytics();
    window.addEventListener("popstate", syncRoute);
    onCleanup(() => window.removeEventListener("popstate", syncRoute));
  });

  const selectChannel = (handle: string) => {
    const normalized = normalizeHandle(handle);
    capture("channel_opened", { handle: normalized, source: "channel_picker" });
    withViewTransition(() => {
      setYoutuber(normalized);
      setRouteHandle(normalized);
      setSubmitted(undefined);
      setPendingIndexHandle(undefined);
      setIndexRequestError(undefined);
      setQueuedIndexRequest(undefined);
      setPage(1);
      window.history.pushState({}, "", routeForHandle(normalized));
    });
    void indexStatus(normalized)
      .then((status) => {
        if (status.status === "queued" && status.request_id) {
          setQueuedIndexRequest({ handle: normalized, requestId: status.request_id });
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
      };
      captureSearchSubmitted(request);
      withViewTransition(() => {
        setYoutuber(request.youtuber);
        setQuery(request.query);
        setPage(nextPage);
        setSubmitted(request);
        writeSearchUrl(request);
      });
      return;
    }
    if (isIndexMode()) {
      setIndexRequestError(undefined);
      setPendingIndexHandle(normalizeHandle(normalizedQuery.split(/\s+/, 1)[0] ?? normalizedQuery));
      return;
    }
    const request = {
      query: normalizedQuery,
      youtuber: youtuber(),
      page: nextPage,
      sort: sort(),
    };
    setPendingIndexHandle(undefined);
    captureSearchSubmitted(request);
    const current = submitted();
    if (
      current?.query === request.query &&
      current.youtuber === request.youtuber &&
      current.page === request.page &&
      current.sort === request.sort
    ) {
      return;
    }
    withViewTransition(() => {
      setQuery(normalizedQuery);
      setQueuedIndexRequest(undefined);
      setPage(nextPage);
      setSubmitted(request);
      writeSearchUrl(request);
    });
  };

  const goToPage = (nextPage: number) => {
    const current = submitted();
    if (!current) return;
    const request = { ...current, page: nextPage };
    captureSearchSubmitted(request);
    withViewTransition(() => {
      setPage(nextPage);
      setSubmitted(request);
      writeSearchUrl(request);
    });
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
    capture("search_link_copied", { has_query: Boolean(submitted()), has_channel: hasChannel() });
    setCopiedSearchLink(true);
    window.setTimeout(() => setCopiedSearchLink(false), 1_500);
  };

  createEffect(() => {
    const normalizedQuery = query().trim();
    const searchQuery = mentionedChannel() ? mentionQuery() : normalizedQuery;
    const currentMode = normalizedQuery.startsWith("@") && !mentionedChannel() ? "index" : "search";
    const currentYoutuber = mentionedChannel()?.handle ?? youtuber();
    const currentSort = sort();
    const currentSubmitted = submitted();
    if (currentMode !== "search" || searchQuery.length < 2) return;
    if (
      currentSubmitted?.query === searchQuery &&
      currentSubmitted.youtuber === currentYoutuber &&
      currentSubmitted.sort === currentSort &&
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
    const key = `${request.query}\u0000${request.youtuber}\u0000${request.sort}\u0000${request.page}`;
    if (key === lastTrackedResponse) return;
    lastTrackedResponse = key;
    captureSearchLoaded(request, response);
  });

  return (
    <main class="min-h-screen bg-vesper-bg text-vesper-text">
      <div class="mx-auto flex w-full max-w-7xl flex-col gap-8 px-4 py-6 sm:px-6 sm:py-8 lg:px-8">
        <section
          class={`transition-all duration-300 ease-out ${
            hasSearchSurface()
              ? "flex flex-col gap-4"
              : "flex min-h-[42vh] flex-col justify-center gap-5"
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

          <SearchForm
            query={query()}
            sort={sort()}
            mode={formMode()}
            onQueryInput={setQuery}
            onSortChange={setSort}
            onSubmit={() => runSearch(1)}
            channelSuggestions={channelSuggestions()}
            onChannelSelect={selectChannel}
          />

          <Show when={!hasChannel()}>
            <div class="space-y-3">
              <p class="text-sm uppercase tracking-[0.18em] text-vesper-dim">Indexed channels</p>
              <ChannelPicker
                channels={channels()}
                selectedHandle={youtuber()}
                onSelect={selectChannel}
              />
            </div>
          </Show>
        </section>

        <section class="min-h-[22rem]">
          <Show when={queuedIndexRequest()}>
            {(request) => (
              <IndexQueuedState handle={request().handle} requestId={request().requestId} />
            )}
          </Show>

          <Show when={!queuedIndexRequest() && result.loading}>
            <LoadingState />
          </Show>

          <Show when={!queuedIndexRequest() && result.error}>
            {(error) => <ErrorState message={String(error())} />}
          </Show>

          <Show when={!queuedIndexRequest() && !result.loading && !result.error && result()}>
            {(response) => (
              <div class="flex flex-col gap-5">
                <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-white/10 pb-3 text-sm text-vesper-muted">
                  <span class="font-medium text-white">{response().query}</span>
                  <span class="text-xs sm:text-sm">
                    {pluralize(response().total_videos ?? response().videos.length, "video")} ·{" "}
                    {pluralize(response().total_moments ?? response().total_found, "moment")} ·{" "}
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

          <Show when={!queuedIndexRequest() && !result.loading && !result.error && !result()}>
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
      <footer class="border-t border-white/10 px-4 py-6 text-center text-sm text-vesper-muted sm:px-6 lg:px-8">
        <span>Support this project on </span>
        <a
          class="font-medium text-white underline decoration-white/30 underline-offset-4 transition hover:decoration-white"
          href="https://ko-fi.com/pronsh"
          target="_blank"
          rel="noreferrer"
        >
          Ko-fi
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
