import { For, Show, createEffect, createResource, createSignal } from "solid-js";
import type { Component } from "solid-js";

type SearchHit = {
  id: string;
  video_id: string;
  title: string;
  text: string;
  prev_text?: string;
  next_text?: string;
  text_highlight?: string;
  title_highlight?: string;
  match_kind: "transcript" | "title";
  youtuber_handle: string;
  youtuber_name: string;
  thumbnail?: string;
  published_at?: string;
  t_start_ms: number;
  timestamp?: string;
  youtube_url: string;
  embed_url: string;
};

type SearchResponse = {
  query: string;
  total_found: number;
  out_of: number;
  page: number;
  per_page: number;
  hits: SearchHit[];
};

type SearchRequest = {
  query: string;
  youtuber: string;
  limit: number;
  page: number;
};

const examples = ["battery", "water", "react", "typescript", "valorant"];

const App: Component = () => {
  const routeHandle = creatorFromPath();
  const [query, setQuery] = createSignal("");
  const [youtuber, setYoutuber] = createSignal(routeHandle ?? "@t3dotgg");
  const [limit, setLimit] = createSignal(25);
  const [page, setPage] = createSignal(1);
  const [submitted, setSubmitted] = createSignal<SearchRequest>();
  const [result] = createResource(submitted, searchTranscripts);

  createEffect(() => {
    const handle = creatorFromPath();
    if (handle) setYoutuber(handle);
  });

  const runSearch = (nextQuery = query(), nextPage = 1) => {
    const normalizedQuery = nextQuery.trim();
    if (!normalizedQuery) return;
    setQuery(normalizedQuery);
    setPage(nextPage);
    setSubmitted({
      query: normalizedQuery,
      youtuber: youtuber().trim(),
      limit: limit(),
      page: nextPage,
    });
  };

  const goToPage = (nextPage: number) => {
    const current = submitted();
    if (!current) return;
    setPage(nextPage);
    setSubmitted({ ...current, page: nextPage });
  };

  return (
    <main class="min-h-screen bg-vesper-bg px-4 py-6 text-vesper-text sm:px-6 lg:px-8">
      <div class="mx-auto flex w-full max-w-6xl flex-col gap-5">
        <header class="border border-vesper-border bg-vesper-panel p-5">
          <div class="flex flex-col gap-2 md:flex-row md:items-end md:justify-between">
            <div>
              <p class="text-xs font-semibold uppercase tracking-[0.28em] text-vesper-yellow">
                ytsearch
              </p>
              <h1 class="mt-2 text-3xl font-semibold tracking-tight text-white">
                Transcript search
              </h1>
            </div>
            <p class="max-w-xl text-sm leading-6 text-vesper-muted">
              Search exact transcript moments. Use URLs like <code>/t3dotgg</code> or{" "}
              <code>/jo2uke</code> to set the creator filter.
            </p>
          </div>
        </header>

        <section class="border border-vesper-border bg-vesper-panel p-4">
          <form
            class="grid gap-3 lg:grid-cols-[1fr_13rem_7rem_auto]"
            onSubmit={(event) => {
              event.preventDefault();
              runSearch(query(), 1);
            }}
          >
            <Field label="Search">
              <input
                value={query()}
                onInput={(event) => setQuery(event.currentTarget.value)}
                placeholder="Search transcript text..."
                class="h-12 w-full border border-vesper-border bg-vesper-bg px-4 text-base text-white outline-none placeholder:text-vesper-dim focus:border-vesper-yellow"
              />
            </Field>

            <Field label="YouTuber">
              <input
                value={youtuber()}
                onInput={(event) => setYoutuber(normalizeHandle(event.currentTarget.value))}
                placeholder="@handle"
                class="h-12 w-full border border-vesper-border bg-vesper-bg px-4 text-base text-white outline-none placeholder:text-vesper-dim focus:border-vesper-yellow"
              />
            </Field>

            <Field label="Hits">
              <input
                type="number"
                min="1"
                max="100"
                value={limit()}
                onInput={(event) => setLimit(Number(event.currentTarget.value))}
                class="h-12 w-full border border-vesper-border bg-vesper-bg px-3 text-base text-white outline-none focus:border-vesper-yellow"
              />
            </Field>

            <button
              type="submit"
              class="mt-auto h-12 border border-vesper-yellow bg-vesper-yellow px-6 font-semibold text-vesper-bg transition hover:bg-vesper-orange"
            >
              Search
            </button>
          </form>

          <div class="mt-3 flex flex-wrap gap-2">
            <For each={examples}>
              {(example) => (
                <button
                  type="button"
                  onClick={() => runSearch(example, 1)}
                  class="border border-vesper-border bg-vesper-panel2 px-3 py-1.5 text-sm text-vesper-muted transition hover:border-vesper-yellow hover:text-vesper-yellow"
                >
                  {example}
                </button>
              )}
            </For>
          </div>
        </section>

        <section class="min-h-[22rem]">
          <Show when={result.loading}>
            <LoadingState />
          </Show>

          <Show when={result.error}>{(error) => <ErrorState message={String(error())} />}</Show>

          <Show when={!result.loading && !result.error && result()}>
            {(response) => (
              <div class="flex flex-col gap-3">
                <div class="flex items-center justify-between border border-vesper-border bg-vesper-panel p-4">
                  <div>
                    <p class="text-xs uppercase tracking-[0.18em] text-vesper-dim">Results</p>
                    <h2 class="text-xl font-semibold text-white">{response().query}</h2>
                  </div>
                  <p class="border border-vesper-border bg-vesper-bg px-3 py-2 text-sm text-vesper-muted">
                    <span class="text-white">{response().hits.length}</span> shown /{" "}
                    <span class="text-white">{response().total_found}</span> found
                  </p>
                </div>

                <Show
                  when={response().hits.length > 0}
                  fallback={<EmptyState query={response().query} />}
                >
                  <div class="grid gap-2">
                    <For each={response().hits}>
                      {(hit) => <HitRow hit={hit} query={response().query} />}
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

const Field: Component<{ label: string; children: unknown }> = (props) => (
  <label class="flex flex-col gap-1.5">
    <span class="text-xs font-medium uppercase tracking-[0.18em] text-vesper-dim">
      {props.label}
    </span>
    {props.children}
  </label>
);

async function searchTranscripts(request: SearchRequest): Promise<SearchResponse> {
  const params = new URLSearchParams({
    q: request.query,
    per_page: String(request.limit),
    page: String(request.page),
  });
  if (request.youtuber) params.set("youtuber", request.youtuber);

  const response = await fetch(`/api/search?${params.toString()}`);
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as {
      error?: string;
    } | null;
    throw new Error(body?.error ?? `Search failed with HTTP ${response.status}`);
  }

  return response.json();
}

const PaginationControls: Component<{
  response: SearchResponse;
  currentPage: number;
  onPageChange: (page: number) => void;
}> = (props) => {
  const totalPages = () =>
    Math.max(1, Math.ceil(props.response.total_found / props.response.per_page));
  const pages = () => pageNumbers(props.currentPage, totalPages());

  return (
    <Show when={props.response.total_found > props.response.per_page}>
      <nav
        aria-label="Search result pages"
        class="mt-2 flex flex-wrap items-center justify-center gap-2 border border-vesper-border bg-vesper-panel p-3 text-sm"
      >
        <button
          type="button"
          disabled={props.currentPage <= 1}
          onClick={() => props.onPageChange(props.currentPage - 1)}
          class="border border-vesper-border bg-vesper-bg px-3 py-1.5 text-vesper-muted transition hover:border-vesper-yellow hover:text-vesper-yellow disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:border-vesper-border disabled:hover:text-vesper-muted"
        >
          Previous
        </button>

        <For each={pages()}>
          {(pageNumber) => (
            <button
              type="button"
              aria-current={pageNumber === props.currentPage ? "page" : undefined}
              onClick={() => props.onPageChange(pageNumber)}
              class={`border px-3 py-1.5 transition ${
                pageNumber === props.currentPage
                  ? "border-vesper-yellow bg-vesper-yellow text-vesper-bg"
                  : "border-vesper-border bg-vesper-bg text-vesper-muted hover:border-vesper-yellow hover:text-vesper-yellow"
              }`}
            >
              {pageNumber}
            </button>
          )}
        </For>

        <button
          type="button"
          disabled={props.currentPage >= totalPages()}
          onClick={() => props.onPageChange(props.currentPage + 1)}
          class="border border-vesper-border bg-vesper-bg px-3 py-1.5 text-vesper-muted transition hover:border-vesper-yellow hover:text-vesper-yellow disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:border-vesper-border disabled:hover:text-vesper-muted"
        >
          Next
        </button>
      </nav>
    </Show>
  );
};

function pageNumbers(currentPage: number, totalPages: number): number[] {
  const start = Math.max(1, currentPage - 3);
  const end = Math.min(totalPages, currentPage + 3);
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}

const HitRow: Component<{ hit: SearchHit; query: string }> = (props) => (
  <article class="border border-vesper-border bg-vesper-panel">
    <div class="grid gap-0 md:grid-cols-[18rem_1fr]">
      <div class="relative min-h-40 bg-vesper-bg">
        <iframe
          class="aspect-video h-full min-h-40 w-full"
          src={props.hit.embed_url}
          title={`YouTube player for ${props.hit.title}`}
          loading="lazy"
          allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
          referrerpolicy="strict-origin-when-cross-origin"
          allowfullscreen
        />
        <span class="absolute bottom-2 left-2 bg-vesper-bg/90 px-2 py-1 text-xs font-semibold text-vesper-yellow ring-1 ring-vesper-border">
          {props.hit.timestamp ?? "title"}
        </span>
      </div>

      <div class="flex flex-col gap-3 p-4">
        <div class="flex items-start justify-between gap-4">
          <div>
            <a
              href={props.hit.youtube_url}
              target="_blank"
              rel="noreferrer"
              class="text-lg font-semibold leading-snug text-white hover:text-vesper-yellow"
            >
              <HighlightedText
                text={props.hit.title}
                highlight={props.hit.title_highlight}
                query={props.query}
              />
            </a>
            <p class="mt-1 text-sm text-vesper-muted">
              {props.hit.youtuber_name} · {props.hit.youtuber_handle}
            </p>
          </div>
          <a
            href={props.hit.youtube_url}
            target="_blank"
            rel="noreferrer"
            class="shrink-0 border border-vesper-border bg-vesper-bg px-3 py-1.5 text-sm font-medium text-vesper-green hover:border-vesper-green"
          >
            {props.hit.timestamp ? `Open ${props.hit.timestamp}` : "Open"}
          </a>
        </div>

        <TranscriptExcerpt hit={props.hit} query={props.query} />
      </div>
    </div>
  </article>
);

const TranscriptExcerpt: Component<{ hit: SearchHit; query: string }> = (props) => (
  <div class="border border-vesper-border bg-vesper-bg p-3 text-sm leading-6 text-white">
    <Show
      when={props.hit.match_kind === "transcript"}
      fallback={
        <p class="text-vesper-muted">
          Title match. Open the video or search transcript wording for a timestamped moment.
        </p>
      }
    >
      <Show when={props.hit.prev_text}>
        <span class="text-vesper-muted">{props.hit.prev_text} </span>
      </Show>
      <span class="text-white">
        <HighlightedText
          text={props.hit.text}
          highlight={props.hit.text_highlight}
          query={props.query}
        />
      </span>
      <Show when={props.hit.next_text}>
        <span class="text-vesper-muted"> {props.hit.next_text}</span>
      </Show>
    </Show>
  </div>
);

const HighlightedText: Component<{
  text: string;
  highlight?: string;
  query: string;
}> = (props) => {
  const parts = () => highlightedParts(props.text, props.highlight, props.query);

  return (
    <>
      <For each={parts()}>
        {(part) => (
          <Show when={part.highlighted} fallback={<>{part.text}</>}>
            <mark class="bg-vesper-yellow/20 px-1 font-semibold text-vesper-yellow">
              {part.text}
            </mark>
          </Show>
        )}
      </For>
    </>
  );
};

type HighlightPart = {
  text: string;
  highlighted: boolean;
};

function highlightedParts(
  text: string,
  highlight: string | undefined,
  query: string,
): HighlightPart[] {
  if (highlight) return partsFromTypesenseSnippet(highlight);
  return partsFromQuery(text, query);
}

function partsFromTypesenseSnippet(snippet: string): HighlightPart[] {
  const parts: HighlightPart[] = [];
  const regex = /<mark>(.*?)<\/mark>/gis;
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = regex.exec(snippet))) {
    if (match.index > lastIndex) {
      parts.push({
        text: decodeEntities(snippet.slice(lastIndex, match.index)),
        highlighted: false,
      });
    }
    parts.push({ text: decodeEntities(match[1] ?? ""), highlighted: true });
    lastIndex = regex.lastIndex;
  }

  if (lastIndex < snippet.length) {
    parts.push({
      text: decodeEntities(snippet.slice(lastIndex)),
      highlighted: false,
    });
  }

  return parts.length > 0 ? parts : [{ text: decodeEntities(snippet), highlighted: false }];
}

function partsFromQuery(text: string, query: string): HighlightPart[] {
  const terms = query
    .toLowerCase()
    .split(/\s+/)
    .map((term) => term.replace(/[^a-z0-9']/gi, ""))
    .filter((term) => term.length >= 2);
  if (terms.length === 0) return [{ text, highlighted: false }];

  const pattern = new RegExp(`(${terms.map(escapeRegex).join("|")})`, "gi");
  return text
    .split(pattern)
    .filter(Boolean)
    .map((part) => ({
      text: part,
      highlighted: terms.includes(part.toLowerCase()),
    }));
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function decodeEntities(value: string): string {
  const element = document.createElement("textarea");
  element.innerHTML = value;
  return element.value;
}

function creatorFromPath(): string | null {
  const firstSegment = window.location.pathname.split("/").filter(Boolean)[0]?.trim();
  if (!firstSegment) return null;
  if (firstSegment === "api") return null;
  return normalizeHandle(firstSegment);
}

function normalizeHandle(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  return trimmed.startsWith("@") ? trimmed : `@${trimmed}`;
}

const LoadingState: Component = () => (
  <div class="grid gap-2">
    <For each={[0, 1, 2]}>
      {() => <div class="h-40 animate-pulse border border-vesper-border bg-vesper-panel" />}
    </For>
  </div>
);

const ErrorState: Component<{ message: string }> = (props) => (
  <div class="border border-vesper-red/40 bg-vesper-red/10 p-5 text-vesper-red">
    <p class="font-semibold">Search failed</p>
    <p class="mt-2 text-sm text-red-100/80">{props.message}</p>
  </div>
);

const EmptyState: Component<{ query: string }> = (props) => (
  <div class="border border-vesper-border bg-vesper-panel p-8 text-center">
    <p class="text-xl font-semibold text-white">No hits for “{props.query}”</p>
    <p class="mt-2 text-vesper-muted">
      Try a broader phrase, remove the creator filter, or index more videos.
    </p>
  </div>
);

const WelcomeState: Component = () => (
  <div class="border border-vesper-border bg-vesper-panel p-8 text-center">
    <p class="text-xl font-semibold text-white">Search a creator transcript.</p>
    <p class="mx-auto mt-2 max-w-xl text-vesper-muted">
      Transcript hits are timestamped. Title-only hits are de-duplicated and open the video without
      pretending to know a spoken moment.
    </p>
  </div>
);

export default App;
