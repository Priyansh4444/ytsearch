import { For, Show, createResource, createSignal } from "solid-js";
import type { Component } from "solid-js";

type SearchHit = {
  id: string;
  video_id: string;
  title: string;
  text: string;
  prev_text?: string;
  next_text?: string;
  text_highlight?: string;
  prev_text_highlight?: string;
  next_text_highlight?: string;
  title_highlight?: string;
  youtuber_handle: string;
  youtuber_name: string;
  thumbnail?: string;
  published_at?: string;
  t_start_ms: number;
  t_end_ms: number;
  timestamp: string;
  youtube_url: string;
};

type SearchResponse = {
  query: string;
  total_found: number;
  out_of: number;
  hits: SearchHit[];
};

type SearchRequest = {
  query: string;
  youtuber: string;
  limit: number;
};

const examples = [
  "chiikawa",
  "resident evil",
  "valorant",
  "silent hill",
  "yakuza",
];

const App: Component = () => {
  const [query, setQuery] = createSignal("");
  const [youtuber, setYoutuber] = createSignal("@jo2uke");
  const [limit, setLimit] = createSignal(25);
  const [submitted, setSubmitted] = createSignal<SearchRequest>();
  const [result] = createResource(submitted, searchTranscripts);

  const runSearch = (nextQuery = query()) => {
    const normalizedQuery = nextQuery.trim();
    if (!normalizedQuery) return;
    setQuery(normalizedQuery);
    setSubmitted({
      query: normalizedQuery,
      youtuber: youtuber().trim(),
      limit: limit(),
    });
  };

  return (
    <main class="min-h-screen px-4 py-6 text-vesper-text sm:px-6 lg:px-8">
      <div class="mx-auto flex w-full max-w-6xl flex-col gap-8">
        <header class="overflow-hidden rounded-[2rem] border border-vesper-border bg-vesper-panel/80 p-6 shadow-vesper backdrop-blur md:p-8">
          <div class="flex flex-col gap-6 lg:flex-row lg:items-end lg:justify-between">
            <div class="max-w-3xl">
              <p class="mb-3 inline-flex rounded-full border border-vesper-border bg-vesper-panel2 px-3 py-1 text-xs font-medium uppercase tracking-[0.24em] text-vesper-yellow">
                ytsearch
              </p>
              <h1 class="text-4xl font-semibold tracking-tight text-white sm:text-5xl">
                Search YouTube transcripts by exact timestamp.
              </h1>
              <p class="mt-4 max-w-2xl text-base leading-7 text-vesper-muted">
                A Vesper-inspired transcript search UI backed by the Rust API
                and Typesense. Repeated videos are intentionally shown so every
                matching moment is easy to jump to.
              </p>
            </div>
            <div class="rounded-2xl border border-vesper-border bg-black/20 px-4 py-3 text-sm text-vesper-muted">
              <span class="text-vesper-green">
                Search key stays server-side.
              </span>
              <br />
              Frontend only talks to <code>/api/search</code>.
            </div>
          </div>
        </header>

        <section class="rounded-[1.75rem] border border-vesper-border bg-vesper-panel/90 p-4 shadow-vesper md:p-5">
          <form
            class="grid gap-3 lg:grid-cols-[1fr_14rem_9rem_auto]"
            onSubmit={(event) => {
              event.preventDefault();
              runSearch();
            }}
          >
            <label class="group flex flex-col gap-2">
              <span class="text-xs font-medium uppercase tracking-[0.18em] text-vesper-dim">
                Search
              </span>
              <input
                value={query()}
                onInput={(event) => setQuery(event.currentTarget.value)}
                placeholder="Try: chiikawa, resident evil, valorant..."
                class="h-14 rounded-2xl border border-vesper-border bg-vesper-bg px-4 text-base text-white outline-none transition placeholder:text-vesper-dim focus:border-vesper-yellow focus:ring-4 focus:ring-vesper-yellow/10"
              />
            </label>

            <label class="flex flex-col gap-2">
              <span class="text-xs font-medium uppercase tracking-[0.18em] text-vesper-dim">
                YouTuber
              </span>
              <input
                value={youtuber()}
                onInput={(event) => setYoutuber(event.currentTarget.value)}
                placeholder="@handle or blank"
                class="h-14 rounded-2xl border border-vesper-border bg-vesper-bg px-4 text-base text-white outline-none transition placeholder:text-vesper-dim focus:border-vesper-green focus:ring-4 focus:ring-vesper-green/10"
              />
            </label>

            <label class="flex flex-col gap-2">
              <span class="text-xs font-medium uppercase tracking-[0.18em] text-vesper-dim">
                Hits
              </span>
              <input
                type="number"
                min="1"
                max="100"
                value={limit()}
                onInput={(event) => setLimit(Number(event.currentTarget.value))}
                class="h-14 rounded-2xl border border-vesper-border bg-vesper-bg px-4 text-base text-white outline-none transition focus:border-vesper-blue focus:ring-4 focus:ring-vesper-blue/10"
              />
            </label>

            <button
              type="submit"
              class="mt-auto h-14 rounded-2xl bg-vesper-yellow px-6 font-semibold text-vesper-bg transition hover:-translate-y-0.5 hover:bg-vesper-orange focus:outline-none focus:ring-4 focus:ring-vesper-yellow/20 active:translate-y-0"
            >
              Search
            </button>
          </form>

          <div class="mt-4 flex flex-wrap gap-2">
            <For each={examples}>
              {(example) => (
                <button
                  type="button"
                  onClick={() => runSearch(example)}
                  class="rounded-full border border-vesper-border bg-vesper-panel2 px-3 py-1.5 text-sm text-vesper-muted transition hover:border-vesper-yellow hover:text-vesper-yellow"
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

          <Show when={result.error}>
            {(error) => <ErrorState message={String(error())} />}
          </Show>

          <Show when={!result.loading && !result.error && result()}>
            {(response) => (
              <div class="flex flex-col gap-4">
                <div class="flex flex-col justify-between gap-3 rounded-3xl border border-vesper-border bg-vesper-panel/70 p-4 sm:flex-row sm:items-center">
                  <div>
                    <p class="text-sm text-vesper-muted">
                      Showing all timestamp hits for
                    </p>
                    <h2 class="text-2xl font-semibold text-white">
                      “{response().query}”
                    </h2>
                  </div>
                  <div class="rounded-2xl border border-vesper-border bg-vesper-bg px-4 py-3 text-sm text-vesper-muted">
                    <span class="text-white">{response().hits.length}</span>{" "}
                    shown /{" "}
                    <span class="text-white">{response().total_found}</span>{" "}
                    found
                  </div>
                </div>

                <Show
                  when={response().hits.length > 0}
                  fallback={<EmptyState query={response().query} />}
                >
                  <div class="grid gap-4">
                    <For each={response().hits}>
                      {(hit) => <HitCard hit={hit} query={response().query} />}
                    </For>
                  </div>
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

async function searchTranscripts(
  request: SearchRequest,
): Promise<SearchResponse> {
  const params = new URLSearchParams({
    q: request.query,
    per_page: String(request.limit),
  });
  if (request.youtuber) params.set("youtuber", request.youtuber);

  const response = await fetch(`/api/search?${params.toString()}`);
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as {
      error?: string;
    } | null;
    throw new Error(
      body?.error ?? `Search failed with HTTP ${response.status}`,
    );
  }

  return response.json();
}

const HitCard: Component<{ hit: SearchHit; query: string }> = (props) => {
  const duration = () =>
    `${props.hit.timestamp}–${formatTimestamp(props.hit.t_end_ms)}`;
  const embedUrl = () =>
    `https://www.youtube-nocookie.com/embed/${props.hit.video_id}?start=${Math.floor(
      props.hit.t_start_ms / 1000,
    )}&rel=0`;

  return (
    <article class="group overflow-hidden rounded-[1.75rem] border border-vesper-border bg-vesper-panel/90 shadow-vesper transition hover:-translate-y-0.5 hover:border-vesper-yellow/60">
      <div class="grid gap-0 lg:grid-cols-[24rem_1fr]">
        <div class="relative min-h-64 bg-vesper-panel2">
          <iframe
            class="aspect-video h-full min-h-64 w-full"
            src={embedUrl()}
            title={`YouTube player for ${props.hit.title} at ${props.hit.timestamp}`}
            loading="lazy"
            allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
            referrerpolicy="strict-origin-when-cross-origin"
            allowfullscreen
          />
          <span class="absolute bottom-3 left-3 rounded-full bg-vesper-bg/90 px-3 py-1 text-sm font-semibold text-vesper-yellow ring-1 ring-vesper-border">
            {duration()}
          </span>
        </div>

        <div class="flex flex-col gap-4 p-5">
          <div class="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
            <div>
              <a
                href={props.hit.youtube_url}
                target="_blank"
                rel="noreferrer"
                class="text-xl font-semibold leading-snug text-white transition hover:text-vesper-yellow"
              >
                <HighlightedText
                  text={props.hit.title}
                  highlight={props.hit.title_highlight}
                  query={props.query}
                />
              </a>
              <p class="mt-1 text-sm text-vesper-muted">
                {props.hit.youtuber_name} <span class="text-vesper-dim">·</span>{" "}
                {props.hit.youtuber_handle}
              </p>
            </div>
            <a
              href={props.hit.youtube_url}
              target="_blank"
              rel="noreferrer"
              class="shrink-0 rounded-full border border-vesper-border bg-vesper-panel2 px-3 py-1.5 text-sm font-medium text-vesper-green transition hover:border-vesper-green hover:bg-vesper-green/10"
            >
              Open at {props.hit.timestamp}
            </a>
          </div>

          <TranscriptExcerpt hit={props.hit} query={props.query} />
        </div>
      </div>
    </article>
  );
};

const TranscriptExcerpt: Component<{ hit: SearchHit; query: string }> = (
  props,
) => (
  <div class="rounded-2xl border border-vesper-border bg-vesper-bg p-4 text-base leading-7 text-white">
    <Show when={props.hit.prev_text}>
      <span class="text-vesper-muted">
        <HighlightedText
          text={props.hit.prev_text ?? ""}
          highlight={props.hit.prev_text_highlight}
          query={props.query}
        />{" "}
      </span>
    </Show>
    <span class="font-medium text-white">
      <HighlightedText
        text={props.hit.text}
        highlight={props.hit.text_highlight}
        query={props.query}
      />
    </span>
    <Show when={props.hit.next_text}>
      <span class="text-vesper-muted">
        {" "}
        <HighlightedText
          text={props.hit.next_text ?? ""}
          highlight={props.hit.next_text_highlight}
          query={props.query}
        />
      </span>
    </Show>
  </div>
);

const HighlightedText: Component<{
  text: string;
  highlight?: string;
  query: string;
}> = (props) => {
  const parts = () =>
    highlightedParts(props.text, props.highlight, props.query);

  return (
    <>
      <For each={parts()}>
        {(part) => (
          <Show when={part.highlighted} fallback={<>{part.text}</>}>
            <mark class="rounded-md bg-vesper-yellow/20 px-1 py-0.5 font-semibold text-vesper-yellow ring-1 ring-vesper-yellow/20">
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

  return parts.length > 0
    ? parts
    : [{ text: decodeEntities(snippet), highlighted: false }];
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

const LoadingState: Component = () => (
  <div class="grid gap-4">
    <For each={[0, 1, 2]}>
      {() => (
        <div class="h-44 animate-pulse rounded-[1.75rem] border border-vesper-border bg-vesper-panel/70" />
      )}
    </For>
  </div>
);

const ErrorState: Component<{ message: string }> = (props) => (
  <div class="rounded-[1.75rem] border border-vesper-red/40 bg-vesper-red/10 p-6 text-vesper-red">
    <p class="font-semibold">Search failed</p>
    <p class="mt-2 text-sm text-red-100/80">{props.message}</p>
  </div>
);

const EmptyState: Component<{ query: string }> = (props) => (
  <div class="rounded-[1.75rem] border border-vesper-border bg-vesper-panel/80 p-8 text-center">
    <p class="text-2xl font-semibold text-white">No hits for “{props.query}”</p>
    <p class="mt-2 text-vesper-muted">
      Try a broader phrase, remove the YouTuber filter, or index more videos.
    </p>
  </div>
);

const WelcomeState: Component = () => (
  <div class="rounded-[1.75rem] border border-vesper-border bg-vesper-panel/80 p-8 text-center">
    <p class="text-2xl font-semibold text-white">
      Start with a transcript search.
    </p>
    <p class="mx-auto mt-2 max-w-xl text-vesper-muted">
      Results show every matching transcript chunk with its timestamp, video
      title, surrounding context, and a YouTube deep link.
    </p>
  </div>
);

function formatTimestamp(milliseconds: number): string {
  const totalSeconds = Math.floor(milliseconds / 1000);
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);

  if (hours === 0) return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  return `${hours}:${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
}

export default App;
