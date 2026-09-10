import { Show, Loading, createEffect, createMemo, createSignal, lazy, onCleanup } from "solid-js";
import type { Component } from "solid-js";
import { indexStatus, indexedChannels, prepareSearchRuntime, requestIndex } from "./api";
import { capture, captureSearchLoaded, captureSearchSubmitted } from "./analytics";
import { useAppLifecycle } from "./app-lifecycle";
import { AvailabilityNotice } from "./components/availability-notice";
import { ChannelAvatar, ChannelIdentity } from "./components/channel-profile";
import {
  buildChannelKeyIndex,
  channelMentionQuery,
  channelSuggestionsByKeys,
  resolveChannelMentionByKeys,
} from "./channel-query";
import { SearchForm } from "./components/search-form";
import { ErrorState, LoadingState, WelcomeState } from "./components/states";
import { presentSearchError } from "./search-error";
import { createSearchResource } from "./search-resource";
import { BackButton, CopyLinkButton } from "./components/toolbar-actions";
import type { IndexedChannel, RecentSearch, SearchRequest, SearchSort } from "./types";
import { cacheChannels, loadCachedChannels } from "./channel-cache";
import { clearRecentSearches, loadRecentSearches, saveRecentSearch } from "./recent-searches";
import {
  provisionalAvailability,
  searchStateFromLocation,
  scrollToSearchResult,
  writeSearchUrl,
} from "./search-route";
import {
  copyToClipboard,
  creatorFromPath,
  normalizeHandle,
  routeForHandle,
  withViewTransition,
} from "./utils";

const supportUrl = import.meta.env.VITE_KOFI_URL?.trim() || "https://ko-fi.com/pronsh";
const SearchResults = lazy(() =>
  import("./components/search-results").then(({ SearchResults: component }) => ({
    default: component,
  })),
);
const ChannelPicker = lazy(() =>
  import("./components/channel-picker").then(({ ChannelPicker: component }) => ({
    default: component,
  })),
);
const IndexRequestDialog = lazy(() =>
  import("./components/index-request-dialog").then(({ IndexRequestDialog: component }) => ({
    default: component,
  })),
);
const RecentSearches = lazy(() =>
  import("./components/recent-searches").then(({ RecentSearches: component }) => ({
    default: component,
  })),
);
const IndexRequestState = lazy(() =>
  import("./components/index-request-state").then(({ IndexRequestState: component }) => ({
    default: component,
  })),
);

const prepareResultsRuntime = () => SearchResults.preload();
const prepareChannelPickerRuntime = () => ChannelPicker.preload();
const prepareIndexDialogRuntime = () => IndexRequestDialog.preload();

const App: Component = () => {
  const initialSearch = searchStateFromLocation();
  const initialRouteHandle = initialSearch?.youtuber
    ? normalizeHandle(initialSearch.youtuber)
    : creatorFromPath();
  const [, setRouteHandle] = createSignal(initialRouteHandle);
  const [channels, setChannels] = createSignal<IndexedChannel[] | undefined>(loadCachedChannels());
  const refreshChannels = async (signal?: AbortSignal) => {
    const [refreshed] = await Promise.all([indexedChannels(signal), prepareChannelPickerRuntime()]);
    setChannels(refreshed);
    cacheChannels(refreshed);
    return refreshed;
  };
  const [availability, setAvailability] = createSignal(provisionalAvailability());
  const [recentSearches, setRecentSearches] = createSignal(loadRecentSearches());
  const [query, setQuery] = createSignal(initialSearch?.query ?? "");
  const [youtuber, setYoutuber] = createSignal(initialSearch?.youtuber ?? initialRouteHandle ?? "");
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
  const [indexingToast, setIndexingToast] = createSignal<string>();
  const [indexingChannelPreview, setIndexingChannelPreview] = createSignal<IndexedChannel>();
  // Do NOT inline this as a plain accessor: `createEffect` nodes are built with
  // `_equals: false`, so an effect sourced from an accessor re-runs on EVERY
  // write to its dependencies. `useAppLifecycle` writes a fresh
  // `provisionalAvailability()` object on mount, on visibilitychange, and every
  // 60 s; without the memo's `===` barrier each of those writes aborts the
  // in-flight search and refetches the identical request, flashing `LoadingState`
  // over the results. The memo's equality check is load-bearing, not overhead.
  const executableSearch = createMemo(() =>
    availability().search_enabled ? submitted() : undefined,
  );
  const search = createSearchResource(executableSearch);

  // Both lookups are built once per channel-list change (fetch/cache), never per
  // keystroke. They key differently on purpose: `channelByKey` uses
  // `normalizeHandle` (trim + `@`, case-preserving, first match wins — the
  // contract the previous `channels().find(...)` calls had), while `channelKeys`
  // lowercases for the substring scan.
  const channelByKey = createMemo(() => {
    const map = new Map<string, IndexedChannel>();
    for (const channel of channels() ?? []) {
      const key = normalizeHandle(channel.handle);
      if (!map.has(key)) map.set(key, channel);
    }
    return map;
  });
  const channelKeys = createMemo(() => buildChannelKeyIndex(channels() ?? []));

  const selectedChannel = createMemo(() => {
    const handle = youtuber();
    return handle ? channelByKey().get(normalizeHandle(handle)) : undefined;
  });
  const indexingChannel = createMemo(() => {
    const toast = indexingToast();
    const exact = toast ? channelByKey().get(normalizeHandle(toast)) : undefined;
    return exact ?? indexingChannelPreview();
  });
  const hasChannel = () => youtuber().length > 0;
  const hasSearchSurface = () => hasChannel() || submitted() !== undefined;
  const showIndexedChannels = () =>
    !hasChannel() && query().trim().length === 0 && submitted() === undefined;
  // Only read inside `runSearch` (an untracked event handler), so this stays a
  // plain accessor — a memo node would buy nothing here.
  const directChannel = () => channelByKey().get(normalizeHandle(query()));
  const channelSuggestions = createMemo(() => channelSuggestionsByKeys(channelKeys(), query()));
  const mentionedChannel = createMemo(() => {
    const currentQuery = query();
    const token = currentQuery.trim().match(/^(@[^\s]+)/)?.[1];
    if (!token) return undefined;
    return resolveChannelMentionByKeys(
      channelKeys(),
      channelByKey().get(normalizeHandle(token)),
      currentQuery,
    );
  });
  const mentionQuery = () => channelMentionQuery(query());
  const isIndexMode = () => query().trim().startsWith("@") && !mentionedChannel();
  const formMode = () => {
    if (mentionedChannel() && !mentionQuery()) return "channel";
    return isIndexMode() ? "request" : "search";
  };
  const searchError = createMemo(() => {
    if (queuedIndexRequest() || !availability().search_enabled) return undefined;
    const state = search();
    return state.status === "error" ? presentSearchError(state.error) : undefined;
  });
  const successfulSearch = createMemo(() => {
    if (queuedIndexRequest() || !availability().search_enabled) return undefined;
    const request = submitted();
    const state = search();
    if (!request || state.status !== "success") return undefined;
    return {
      response: state.response,
      youtuber: request.youtuber,
      page: page(),
    };
  });
  const indexDialogState = createMemo(() => {
    const handle = pendingIndexHandle();
    return handle ? { handle, busy: indexRequestBusy(), error: indexRequestError() } : undefined;
  });
  const indexingNotice = createMemo(() => {
    const handle = indexingToast();
    return handle ? { handle, channel: indexingChannel() } : undefined;
  });

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
      setIndexingToast(undefined);
    });

  useAppLifecycle({
    availability,
    setAvailability,
    refreshChannels,
    setIndexingToast,
    setIndexingChannelPreview,
    syncRoute,
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
    const direct = directChannel();
    if (direct) {
      selectChannel(direct.handle);
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
      setYoutuber(request.youtuber);
      setQuery(request.query);
      setPage(nextPage);
      setSubmitted(request);
      writeSearchUrl(request);
      return;
    }
    if (isIndexMode()) {
      void prepareIndexDialogRuntime();
      setIndexRequestError(undefined);
      setPendingIndexHandle(normalizeHandle(normalizedQuery.split(/\s+/, 1)[0] ?? normalizedQuery));
      return;
    }
    if (!availability().search_enabled) return;
    // Fetch result rendering code in parallel with the API request. Keeping it
    // out of the initial route protects the browser budget without adding a
    // serial step to first-result rendering.
    void prepareResultsRuntime();
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
    setQuery(normalizedQuery);
    setQueuedIndexRequest(undefined);
    setPage(nextPage);
    setSubmitted(request);
    writeSearchUrl(request);
  };

  const goToPage = (nextPage: number) => {
    const current = submitted();
    if (!current) return;
    // Explicit literal: same fields, stable object shape, no spread copy.
    const request: SearchRequest = {
      query: current.query,
      youtuber: current.youtuber,
      page: nextPage,
      sort: current.sort,
    };
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

  createEffect(
    () => {
      const normalizedQuery = query().trim();
      const searchQuery = mentionedChannel() ? mentionQuery() : normalizedQuery;
      const currentFormMode =
        normalizedQuery.startsWith("@") && !mentionedChannel() ? "index" : "search";
      // Plain boolean, not a `{ shouldWarm }` wrapper: the effect handler runs on
      // every dependency notification either way (effect nodes do not compare
      // values), so this only drops one allocation per evaluation.
      return (
        currentFormMode === "search" && searchQuery.length >= 2 && availability().search_enabled
      );
    },
    (shouldWarm) => {
      if (!shouldWarm) return;
      const timeout = window.setTimeout(() => {
        // Warm only the lazy Effect runtime. Speculative network searches become
        // duplicate origin work whenever a user pauses before submitting, and a
        // completed speculative response cannot provide honest click-to-result timing.
        void Promise.all([prepareSearchRuntime(), prepareResultsRuntime()]).catch(() => undefined);
      }, 900);
      return () => window.clearTimeout(timeout);
    },
  );

  let lastTrackedResponse = "";
  createEffect(
    () => ({ request: submitted(), state: search() }),
    ({ request, state }) => {
      if (!request || state.status !== "success") return;
      const key = `${request.query}\u0000${request.youtuber}\u0000${request.sort}\u0000${request.page}`;
      if (key === lastTrackedResponse) return;
      lastTrackedResponse = key;
      captureSearchLoaded(request, state.response);
      if (request.page === 1) {
        setRecentSearches((current) => saveRecentSearch(current, request));
      }
    },
  );

  const scrollToLinkedResult = () => {
    if (search().status !== "success") return;
    scrollToSearchResult();
  };

  createEffect(
    () => search().status,
    (status) => {
      if (status !== "success") return;
      let frame: number | undefined;
      let cancelled = false;
      void prepareResultsRuntime().then(() => {
        // Only the frame is skippable: with no hash there is nothing to scroll
        // to. The preload above still runs so a direct `/?q=…` load does not
        // start the lazy results chunk at render time instead of on success.
        if (cancelled || !window.location.hash) return;
        frame = window.requestAnimationFrame(scrollToLinkedResult);
      });
      return () => {
        cancelled = true;
        if (frame !== undefined) window.cancelAnimationFrame(frame);
      };
    },
  );

  window.addEventListener("hashchange", scrollToLinkedResult);
  onCleanup(() => window.removeEventListener("hashchange", scrollToLinkedResult));

  const selectRecentSearch = (recent: RecentSearch) => {
    setQuery(recent.query);
    setYoutuber(recent.youtuber);
    setSort(recent.sort);
    setPage(1);
    if (!availability().search_enabled) {
      setSubmitted(undefined);
      return;
    }
    // Explicit literal (drops the dead `searchedAt` the spread carried).
    const request: SearchRequest = {
      query: recent.query,
      youtuber: recent.youtuber,
      page: 1,
      sort: recent.sort,
    };
    captureSearchSubmitted(request);
    setSubmitted(request);
    writeSearchUrl(request);
  };

  // Stable handlers: these bodies are identical for every invocation, so they
  // are created once here instead of once per JSX construction (Solid runs a
  // component body once per mount, so the saving is one closure per mount each,
  // not a per-update win).
  const submitFirstPage = () => runSearch(1);
  const handleCopySearchLink = () => void copySearchLink();
  const handleClearRecent = () => setRecentSearches(clearRecentSearches());
  const dismissIndexDialog = () => {
    setPendingIndexHandle(undefined);
    setIndexRequestError(undefined);
  };
  const confirmDialogRequest = () => void confirmIndexRequest();

  return (
    <main class="flex min-h-svh flex-col overflow-clip bg-vesper-bg text-vesper-text">
      <h1 class="sr-only">Search YouTube transcripts</h1>
      <div class="mx-auto flex w-full max-w-7xl flex-1 flex-col gap-6 px-4 py-4 [padding-bottom:calc(1rem+env(safe-area-inset-bottom))] sm:px-6 sm:py-6 lg:px-8">
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
              <CopyLinkButton copied={copiedSearchLink()} onClick={handleCopySearchLink} />
            </div>
          </Show>

          <Show when={!hasSearchSurface()}>
            <Loading>
              <ChannelPicker
                channels={channels()}
                visible={showIndexedChannels()}
                onSelect={selectChannel}
              />
            </Loading>
          </Show>

          <SearchForm
            query={query()}
            sort={sort()}
            searchAvailable={availability().search_enabled}
            busy={search().status === "loading"}
            home={!hasSearchSurface()}
            selectedChannelName={selectedChannel()?.name}
            mode={formMode()}
            onQueryInput={setQuery}
            onSortChange={setSort}
            onSubmit={submitFirstPage}
            channelSuggestions={channelSuggestions()}
            onChannelSelect={selectChannel}
          />
          <AvailabilityNotice availability={availability()} />
          <Show when={!hasSearchSurface() && recentSearches().length > 0}>
            <Loading>
              <RecentSearches
                searches={recentSearches()}
                onSelect={selectRecentSearch}
                onClear={handleClearRecent}
              />
            </Loading>
          </Show>
        </section>

        <section class={hasSearchSurface() ? "pb-8" : "hidden"}>
          <Show when={queuedIndexRequest()}>
            {(request) => (
              <Loading>
                <IndexRequestState handle={request().handle} requestId={request().requestId} />
              </Loading>
            )}
          </Show>

          <Show
            when={
              !queuedIndexRequest() &&
              availability().search_enabled &&
              search().status === "loading"
            }
          >
            <LoadingState />
          </Show>

          <Show when={searchError()} keyed>
            {(presentation) => (
              <ErrorState title={presentation.title} message={presentation.message} />
            )}
          </Show>

          <Show when={successfulSearch()} keyed>
            {(result) => (
              <Loading fallback={<LoadingState />}>
                <SearchResults
                  response={result.response}
                  youtuber={result.youtuber}
                  page={result.page}
                  onPageChange={goToPage}
                />
              </Loading>
            )}
          </Show>

          <Show
            when={
              hasSearchSurface() &&
              !queuedIndexRequest() &&
              !submitted() &&
              search().status === "idle"
            }
          >
            <WelcomeState />
          </Show>
        </section>
      </div>
      <Show when={indexDialogState()} keyed>
        {(dialog) => (
          <Loading>
            <IndexRequestDialog
              handle={dialog.handle}
              busy={dialog.busy}
              error={dialog.error}
              onCancel={dismissIndexDialog}
              onConfirm={confirmDialogRequest}
            />
          </Loading>
        )}
      </Show>
      <Show when={indexingNotice()} keyed>
        {(notice) => (
          <div
            class="fixed inset-x-4 bottom-[calc(1rem+env(safe-area-inset-bottom))] z-50 flex items-center gap-3 border border-vesper-green bg-vesper-panel px-4 py-3 text-sm text-white shadow-xl sm:end-5 sm:inset-x-auto sm:max-w-[calc(100vw-2.5rem)]"
            role="status"
          >
            <ChannelAvatar channel={notice.channel} size="compact" />
            <span class="min-w-0">
              <span class="block text-xs uppercase tracking-[0.12em] text-vesper-green">
                Currently indexing
              </span>
              <span class="block truncate font-medium">
                {notice.channel?.name ?? notice.handle}
              </span>
            </span>
          </div>
        )}
      </Show>
      <footer class="px-4 py-5 text-center text-sm sm:px-6 lg:px-8">
        <a
          href={supportUrl}
          target="_blank"
          rel="noreferrer"
          class="inline-flex min-h-11 items-center text-vesper-muted transition-colors hover:text-vesper-yellow"
        >
          Support on Ko-fi ↗
        </a>
      </footer>
    </main>
  );
};

export default App;
