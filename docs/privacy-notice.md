# ytsearch Privacy and YouTube Compliance Notice

A project template and checklist, not legal advice. Review it with qualified
counsel before operating ytsearch as a public product.

## What it processes

ytsearch indexes YouTube creator metadata and transcript text so users can
search timestamped moments and open them in the embedded player or on YouTube.
It may process:

- channel and video metadata from the YouTube Data API;
- public captions fetched with `yt-dlp`;
- transcripts generated locally from audio, only with `--transcribe-missing`;
- user search queries, and API and CLI server logs.

A public deployment should link to the
[YouTube Terms of Service](https://www.youtube.com/t/terms), the
[Google Privacy Policy](https://policies.google.com/privacy) and the
[YouTube API Services Developer Policies](https://developers.google.com/youtube/terms/developer-policies),
and state that it uses YouTube API Services.

## Embedded player

The API returns privacy-enhanced embed URLs:

```text
https://www.youtube-nocookie.com/embed/{video_id}?start={seconds}&rel=0
```

Result cards show poster thumbnails from `i.ytimg.com` (Google). No player loads
until the user clicks a poster; the embed then loads with `autoplay=1`, so that
click starts playback. Google receives the thumbnail requests as soon as results
render, and YouTube may receive information when the player loads; disclose both
in your privacy policy.

## Local transcription

`--transcribe-missing` extracts audio with `yt-dlp`, converts it with `ffmpeg`
and transcribes it with `voxtype`. YouTube's Terms and Developer Policies
restrict downloading and storing audiovisual content without permission, so use
it only where you have permission or another legal basis, and never to bypass
access controls or redistribute content.

## Stored data

Local caches (audio, chunks, transcript JSON) live under the gitignored `data/`
directory. YouTube's Developer Policies generally require non-authorized API data
to be deleted or refreshed within 30 days. ytsearch has no user accounts; adding
them, or OAuth, brings consent, retention, deletion and revocation duties.

## Search queries and analytics

Queries reach three places in the maintained deployment:

- the API's `tracing` logs;
- Axiom `search_query` events, when `AXIOM_API_KEY` (or `AXIOM_TOKEN`) and
  `AXIOM_DATASET` are set: query text, timings, result counts, the Cloudflare
  country header (`cf-ipcountry`) and request ID (`cf-ray`);
- PostHog, when `VITE_POSTHOG_KEY` is set: search events carry the query's
  length and term count (not its text), sort and page; interaction events carry
  the creator handle and the opened video's identifiers. Every event carries a
  random ID kept in `localStorage` and the page URL without its query string,
  which holds the search text.

Queries can be personal data. Minimize logged text, limit retention, give users
notice, and support deletion where required. The maintained deployment states
what it does in [`public/privacy.html`](../public/privacy.html) (served at `/privacy`) (Axiom keeps
search events for 30 days); update that page with any change here.

YouTube's policies add requirements for child-directed clients and Made for Kids
content, including embeds; do not target children without those controls.

Keys stay server-side as described in the
[credential boundary](architecture.md#credential-boundary): the frontend calls
`youtube_api`, which holds only the search key.

## Production checklist

- [ ] Publish a privacy policy and terms of use linking to YouTube's Terms.
- [ ] Confirm YouTube API Services compliance.
- [ ] Decide whether the audio fallback is permitted; avoid storing audio.
- [ ] Implement retention and deletion for local files, Raw mutation records
      and published generations.
- [ ] Keep admin keys server-side and embeds click-to-play.
- [ ] Rate-limit the API.
- [ ] Publish a contact address for privacy and compliance requests.
