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
import { indexedChannels, requestIndex, searchTranscripts } from "./api";
import { ChannelPicker } from "./components/ChannelPicker";
import { ChannelIdentity } from "./components/ChannelProfile";
import { PaginationControls } from "./components/PaginationControls";
import { SearchForm } from "./components/SearchForm";
import {
  EmptyState,
  ErrorState,
  IndexQueuedState,
  IndexUnavailableState,
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
  const [unavailableIndexHandle, setUnavailableIndexHandle] = createSignal<string>();
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
  const isIndexMode = () => query().trim().startsWith("@") && !directChannel();
  const formMode = () => (directChannel() ? "channel" : isIndexMode() ? "index" : "search");

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
      setUnavailableIndexHandle(undefined);
      setQueuedIndexRequest(undefined);
    });

  onMount(() => {
    window.addEventListener("popstate", syncRoute);
    onCleanup(() => window.removeEventListener("popstate", syncRoute));
  });

  const selectChannel = (handle: string) => {
    const normalized = normalizeHandle(handle);
    withViewTransition(() => {
      setYoutuber(normalized);
      setRouteHandle(normalized);
      setSubmitted(undefined);
      setUnavailableIndexHandle(undefined);
      setQueuedIndexRequest(undefined);
      setPage(1);
      window.history.pushState({}, "", routeForHandle(normalized));
    });
  };

  const queueIndexRequest = async () => {
    const handle = normalizeHandle(query());
    if (!handle) return;
    setQuery(handle);
    setSubmitted(undefined);
    setUnavailableIndexHandle(undefined);
    const request = await requestIndex(handle);
    setQueuedIndexRequest({ handle, requestId: request.id });
  };

  const runSearch = (nextPage = 1) => {
    const normalizedQuery = query().trim();
    if (!normalizedQuery) return;
    if (directChannel()) {
      selectChannel(directChannel()!.handle);
      return;
    }
    if (isIndexMode()) {
      void queueIndexRequest().catch((error) => {
        setQueuedIndexRequest(undefined);
        setUnavailableIndexHandle(String(error));
      });
      return;
    }
    const request = {
      query: normalizedQuery,
      youtuber: youtuber(),
      page: nextPage,
      sort: sort(),
    };
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
      setUnavailableIndexHandle(undefined);
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
      setUnavailableIndexHandle(undefined);
      setQueuedIndexRequest(undefined);
    });
  };

  const [copiedSearchLink, setCopiedSearchLink] = createSignal(false);
  const copySearchLink = async () => {
    await copyToClipboard(window.location.href);
    setCopiedSearchLink(true);
    window.setTimeout(() => setCopiedSearchLink(false), 1_500);
  };

  createEffect(() => {
    const normalizedQuery = query().trim();
    const currentMode = normalizedQuery.startsWith("@") ? "index" : "search";
    const currentYoutuber = youtuber();
    const currentSort = sort();
    const currentSubmitted = submitted();
    if (currentMode !== "search" || normalizedQuery.length < 2) return;
    if (
      currentSubmitted?.query === normalizedQuery &&
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
          query: normalizedQuery,
          youtuber: currentYoutuber,
          page: 1,
          sort: currentSort,
        },
        { signal: controller.signal },
      ).catch((error) => {
        if (error instanceof DOMException && error.name === "AbortError") return;
      });
    }, 450);

    onCleanup(() => {
      window.clearTimeout(timeout);
      controller.abort();
    });
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
          <Show when={unavailableIndexHandle()}>
            {(handle) => <IndexUnavailableState handle={handle()} />}
          </Show>

          <Show when={!unavailableIndexHandle() && queuedIndexRequest()}>
            {(request) => (
              <IndexQueuedState handle={request().handle} requestId={request().requestId} />
            )}
          </Show>

          <Show when={!unavailableIndexHandle() && !queuedIndexRequest() && result.loading}>
            <LoadingState />
          </Show>

          <Show when={!unavailableIndexHandle() && !queuedIndexRequest() && result.error}>
            {(error) => <ErrorState message={String(error())} />}
          </Show>

          <Show
            when={
              !unavailableIndexHandle() &&
              !queuedIndexRequest() &&
              !result.loading &&
              !result.error &&
              result()
            }
          >
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

          <Show
            when={
              !unavailableIndexHandle() &&
              !queuedIndexRequest() &&
              !result.loading &&
              !result.error &&
              !result()
            }
          >
            <WelcomeState />
          </Show>
        </section>
      </div>
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
          {(video) => <VideoResult video={video} query={props.query} />}
        </For>
      </div>
    </section>
  </Show>
);
