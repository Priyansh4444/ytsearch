# Agent readiness

## Fast path

```bash
scripts/check-agent-surface.sh https://search.pronsh.dev
npx is-agentic search.pronsh.dev --json | jq '{score, scanned_at, issues: [.issues[] | {id, tier, result}]}'
```

Reports are immutable snapshots refreshed only when older than 6 hours, so compare
`scanned_at`; use Rescan at `https://is-agentic.com/scan/<host>` right after a deploy.

The Worker owns what static hosting cannot do, in [pages.ts](../../worker/pages.ts):

- Real 404s: the assets binding has no SPA fallback. `/`, `/dashboard` and known creators
  serve the app. A creator path is checked against `/api/youtubers`, cached for five minutes.
- `Accept: text/markdown` on any page: `public/<page>.md`, or generated for shared moments
  and creators. Every negotiated response carries `Vary: Accept`.
- API failures that are not already JSON become `{error, code, hint}`
  ([api-error.ts](../../worker/api-error.ts)).
- `PUBLIC_APP_ORIGIN` replaces the production origin in pages and discovery files, so a test
  deployment describes itself.

Static content lives in `public/`: `about`, `contact`, `docs` and `privacy`, each as `.html`
and `.md`; `llms.txt`, `robots.txt`, `sitemap.xml`, `openapi.json` and
`.well-known/api-catalog`. Change a page's two forms together. `index.html` carries the
first paint and an about section that work without JavaScript; `src/index.tsx` replaces
the first paint when the app starts.

## Traps

- `wrangler.jsonc` sends every path but `/assets/*`, `/favicon.png` and `/og-card.png` to
  the Worker. Pages therefore run at the Worker's pinned region, one trip to `aws:us-west-1`
  from a distant edge, while hashed bundles still come from the nearest edge. Check
  `perf:frontend` when changing that list.
- A wrong 404 would hide a real creator, so an unreachable catalog serves the app. A soft
  404 is therefore possible while the API is down; on the experimental site that includes
  the first request while its machine boots.
- `public/_headers` covers only responses from the assets binding. Pages generated in the
  Worker repeat it through `SECURITY_HEADERS`; change both.

## Link previews

- Discord's [docs](https://docs.discord.com/developers/link-previews/overview) accept only
  a direct `.mp4`, `.webm` or `.mov` for `og:video`; YouTube links play through its own
  handling. X plays a `twitter:player` only from approved domains.
- So `/s/*` sends `Discordbot` and `Twitterbot` to the YouTube timestamp (`wantsYouTubeUnfurl`
  in [page-route.ts](../../worker/page-route.ts)), and everyone else gets the clip page. The
  preview is YouTube's card: it starts at the window but does not stop at its end and has
  no quote. `?card=page` shows those crawlers this site's own card, to compare.
- A preview that stops at the end and quotes the words would need a hosted clip file, which
  re-hosts creators' video. Do not do that without their permission.
- Previews are cached per URL. Change `t` to retest.

## Facts

Scored 29/100 on 2026-08-21. Essential checks share 80 points, recommended share 20 and
emerging signals add at most 5. Two recommended checks depend on the outside world, not this
repository: brand and developer-resource web search. `org-schema-completeness` wants a
postal address, which this project does not have and will not invent.
