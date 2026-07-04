import { For, Show, createMemo, createResource, createSignal } from "solid-js";
import type { Component } from "solid-js";
import { indexedChannels, searchTranscripts } from "./api";
import { ChannelIdentity, ChannelPicker } from "./components/ChannelPicker";
import { PaginationControls } from "./components/PaginationControls";
import { SearchForm } from "./components/SearchForm";
import { EmptyState, ErrorState, LoadingState, WelcomeState } from "./components/States";
import { VideoResult } from "./components/VideoResult";
import type { SearchRequest, SearchSort } from "./types";
import { creatorFromPath, normalizeHandle } from "./utils";

const App: Component = () => {
  const routeHandle = creatorFromPath();
  const [channels] = createResource(indexedChannels);
  const [query, setQuery] = createSignal("");
  const [youtuber, setYoutuber] = createSignal(routeHandle ?? "");
  const [page, setPage] = createSignal(1);
  const [sort, setSort] = createSignal<SearchSort>("relevance");
  const [submitted, setSubmitted] = createSignal<SearchRequest>();
  const [result] = createResource(submitted, searchTranscripts);

  const selectedChannel = createMemo(() =>
    channels()?.find((channel) => channel.handle === youtuber()),
  );
  const hasChannel = () => youtuber().length > 0;

  const selectChannel = (handle: string) => {
    setYoutuber(normalizeHandle(handle));
    setPage(1);
  };

  const runSearch = (nextPage = 1) => {
    const normalizedQuery = query().trim();
    if (!normalizedQuery) return;
    setQuery(normalizedQuery);
    setPage(nextPage);
    setSubmitted({
      query: normalizedQuery,
      youtuber: youtuber(),
      page: nextPage,
      sort: sort(),
    });
  };

  const goToPage = (nextPage: number) => {
    const current = submitted();
    if (!current) return;
    setPage(nextPage);
    setSubmitted({ ...current, page: nextPage });
  };

  return (
    <main class="min-h-screen bg-vesper-bg px-4 py-8 text-vesper-text sm:px-6 lg:px-8">
      <div class="mx-auto flex w-full max-w-6xl flex-col gap-6">
        <section
          class={`transition-all duration-500 ease-out ${
            hasChannel() ? "flex flex-col gap-4" : "flex min-h-[45vh] flex-col justify-center gap-6"
          }`}
        >
          <Show when={hasChannel()}>
            <ChannelIdentity channel={selectedChannel()} handle={youtuber()} />
          </Show>

          <SearchForm
            query={query()}
            sort={sort()}
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
          <Show when={result.loading}>
            <LoadingState />
          </Show>

          <Show when={result.error}>{(error) => <ErrorState message={String(error())} />}</Show>

          <Show when={!result.loading && !result.error && result()}>
            {(response) => (
              <div class="flex flex-col gap-3">
                <div class="flex items-center justify-between py-2 text-sm text-vesper-muted">
                  <span class="text-white">{response().query}</span>
                  <span>
                    {response().videos.length} videos · {response().hits.length} moments
                  </span>
                </div>

                <Show
                  when={response().videos.length > 0}
                  fallback={<EmptyState query={response().query} />}
                >
                  <div class="grid gap-2">
                    <For each={response().videos}>
                      {(video) => <VideoResult video={video} query={response().query} />}
                    </For>
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

          <Show when={!result.loading && !result.error && !result()}>
            <WelcomeState />
          </Show>
        </section>
      </div>
    </main>
  );
};

export default App;
