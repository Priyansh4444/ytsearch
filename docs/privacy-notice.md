# ytsearch Privacy and YouTube Compliance Notice

This document is a project template and checklist. It is not legal advice. Review with qualified counsel before operating ytsearch as a public product.

## What the app does

ytsearch indexes YouTube creator metadata and transcript-like text into its Raw
inverted index so users can search timestamped moments and open matching videos
in the official embedded YouTube player or on YouTube.

## Data sources

The project may process:

- YouTube channel metadata from the YouTube Data API.
- YouTube video metadata from the YouTube Data API.
- Public caption/timed text data discovered via `yt-dlp`.
- Optional locally generated transcripts from downloaded/extracted audio when `--transcribe-missing` is enabled.
- Search queries submitted by users of the local/frontend API.
- Server logs produced by the Rust API and CLI.

## YouTube and Google notices

If this becomes a public API client, the user-facing app should clearly link to:

- YouTube Terms of Service: https://www.youtube.com/t/terms
- Google Privacy Policy: https://policies.google.com/privacy
- YouTube API Services Developer Policies: https://developers.google.com/youtube/terms/developer-policies

The app should state that it uses YouTube API Services when YouTube API data is used.

## Embedded YouTube player privacy

The API returns embed URLs on the YouTube privacy-enhanced domain:

```text
https://www.youtube-nocookie.com/embed/{video_id}?start={seconds}&rel=0
```

Result cards show a poster thumbnail loaded from `i.ytimg.com` (Google). No
YouTube player loads until the user clicks a poster; the frontend then loads the
embed with `autoplay=1`, so that click starts playback without a second click.

Google receives requests for poster thumbnails as soon as results render, and
YouTube/Google may receive information when the player loads. The app should
disclose this in its privacy policy.

## Local audio transcription fallback

`--transcribe-missing` extracts audio with `yt-dlp`, converts it with `ffmpeg`, and transcribes chunks with `voxtype`.

Important compliance considerations:

- YouTube API Developer Policies restrict downloading, importing, backing up, caching, or storing copies of YouTube audiovisual content without YouTube's prior written approval.
- YouTube Terms restrict downloading/reusing content except as expressly authorized or with permission from YouTube and relevant rights holders.
- Use the audio fallback only where you have permission or a valid legal basis.
- Do not use the fallback to bypass access controls, infringe copyright, or redistribute third-party audiovisual content.

## Storage and retention

Local generated files are stored under `data/` by default and are gitignored.

Potential contents:

- normalized audio files
- temporary chunks
- transcript JSON caches

For production, implement:

- retention limits
- deletion requests
- revalidation/refetch schedules
- secure storage
- operator audit logs

YouTube API Developer Policies generally require that temporarily stored non-authorized API data be deleted or refreshed within 30 days. Authorized user data has additional consent, revocation, and deletion requirements.

## User data

The current local app does not implement accounts. If accounts are added, document:

- what account data is collected
- why it is collected
- how long it is retained
- how users can delete it
- who it is shared with
- how to revoke Google/YouTube OAuth access if OAuth is used

## Search query logs and analytics

Search queries reach three places in the maintained deployment:

- the API's `tracing` logs;
- Axiom `search_query` events, when `AXIOM_API_KEY` (or `AXIOM_TOKEN`) and
  `AXIOM_DATASET` are set: query text, timings, result counts, the
  Cloudflare country header (`cf-ipcountry`) and request ID (`cf-ray`);
- PostHog, when `VITE_POSTHOG_KEY` is set: search events carry the query's
  length and term count (not its text), sort and page; interaction events
  carry the creator handle and the opened video's identifiers.

If deployed publicly, search queries can be personal data. Consider:

- minimizing logged query text
- redacting or hashing IP addresses
- limiting retention
- giving users notice
- allowing deletion where required

## Children and Made for Kids content

YouTube API Developer Policies include additional requirements for child-directed API clients and Made for Kids content, especially when embedding players.

If ytsearch is operated publicly, add logic/policy for Made for Kids videos and do not target children unless you have implemented the required compliance controls.

## API keys and secrets

Never expose:

- `YOUTUBE_API_KEY` in frontend bundles if avoidable
- `RAW_SEARCH_ADMIN_API_KEY`
- `RAW_SEARCH_API_KEY`
- OAuth client secrets
- refresh tokens

The frontend talks to `youtube_api`, which uses only the Raw search-only key.
The Raw admin key belongs only in trusted indexing processes.

## Recommended production checklist

- [ ] Publish a privacy policy before public launch.
- [ ] Publish terms of use that link to YouTube Terms.
- [ ] Confirm YouTube API Services compliance.
- [ ] Decide whether audio fallback is permitted for the intended use case.
- [ ] Implement retention/deletion for local files, Raw mutation records, and
  published generations.
- [ ] Avoid storing raw audio unless required and permitted.
- [ ] Keep admin keys server-side only.
- [ ] Keep embeds click-to-play: load the player only after the user chooses a video.
- [ ] Add abuse/rate limits to the API.
- [ ] Add a contact address for privacy/compliance requests.
