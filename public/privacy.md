# Privacy

Last updated 6 October 2026

ytsearch searches the public captions of indexed YouTube creators. It has no accounts and sets no cookies. This page describes what the service at search.pronsh.dev processes.

## Searches

To run and improve search, each query is logged with its text, result counts, timings, the country Cloudflare reports for the request, your browser's user-agent string and a request ID. These logs are stored with Axiom and deleted after 30 days. Your IP address is used only in memory for rate limiting and is not logged.

## Usage analytics

Anonymous usage events go to PostHog (US): page views and searches (query length and number of words, never the query text), sort and page choices, and which creator or video result you open. They carry a random ID kept in your browser's local storage, not a profile of you. Cloudflare Web Analytics also counts page views without cookies.

## Stored in your browser

Your recent searches and the creator list stay in your browser's local storage and are never sent anywhere; the analytics ID is sent only with the usage events above. Clear recent searches on the home page, or clear site data in your browser to remove everything.

## YouTube

Thumbnails load from YouTube's image servers when results appear. The player loads from youtube-nocookie.com only when you press play. YouTube then processes data under the [Google Privacy Policy](https://policies.google.com/privacy). ytsearch uses YouTube API Services; see the [YouTube Terms of Service](https://www.youtube.com/t/terms).

## Shared links

A shared moment or result link carries its search words in the URL, so anyone you send it to can read them. ytsearch stores nothing when you make one; the page looks the moment up each time it is opened.

## Creator requests

When you request a creator, we store the handle, your optional note, the time and the request's status. The request is deleted once the creator is indexed; a rejected request is kept as a record of the decision.

## Hosting

Cloudflare serves the site and Fly.io runs the search servers; both process requests and keep standard operational logs.

## Contact

Questions or deletion requests: [open an issue](https://github.com/Priyansh4444/ytsearch/issues). Report security problems privately as described in [SECURITY.md](https://github.com/Priyansh4444/ytsearch/blob/master/SECURITY.md).
