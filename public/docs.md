# ytsearch API docs

The ytsearch API is public and read-only. It needs no key: send an HTTP GET and read JSON. The machine-readable contract is the [OpenAPI document](https://search.pronsh.dev/openapi.json), and [llms.txt](https://search.pronsh.dev/llms.txt) tells agents when to use it.

## Quickstart

```sh
curl 'https://search.pronsh.dev/api/v1/search?q=server+actions&youtuber=%40theo&per_page=3'
```

Each video in `videos` has a `youtube_url` and `hits`; every hit is a transcript moment with its text and a `youtube_url` that opens at that timestamp.

## Endpoints

| Endpoint | Use |
| --- | --- |
| `GET /api/v1/search` | Find transcript moments, grouped by video. |
| `GET /api/v1/youtubers` | List indexed creators and their video counts. |
| `GET /api/v1/availability` | Check whether search is serving. |
| `GET /api/v1/health` | Check that the API can reach its index. |

## Search parameters

| Name | Meaning |
| --- | --- |
| `q` | Required. Words to find; quote a phrase to match it exactly. |
| `youtuber` | Only this creator, such as `@theo`. |
| `video_id` | Only this video, to list its moments. |
| `sort` | `relevance` (default), `relevance_engagement`, `views_desc`, `likes_desc`, `comments_desc`, `published_desc` or `published_asc`. |
| `page`, `per_page` | Paging; results are grouped by video. |
| `mode` | `lexical` (default) or, where it is hosted, `hybrid` or `semantic`. The response's `mode` says which actually answered. |

## Authentication and limits

No authentication, API key or OAuth flow exists. Search allows about 120 requests a minute per IP address; search responses state this in `RateLimit-Policy`, and beyond it you get `429` and should wait before retrying. Please do not bulk-export the index.

## Versioning

The path carries the major version. Additive changes, such as new response fields, stay in `/api/v1`; a breaking change gets `/api/v2`, and `/api/v1` keeps working until its deprecation is announced on this page.

## Errors

Errors are JSON: `{"error": "what went wrong", "code": "stable_code", "hint": "what to do"}`. `400` means fix the request, `404` means no such route, `429` means slow down and `503` means retry shortly.

## Sharing a moment

A link such as `/s/VIDEO_ID?t=754&end=784&q=server+actions` opens a page with that window of the video and the matching transcript lines. Request it with `Accept: text/markdown` to get the same page as Markdown; every page here does the same.
