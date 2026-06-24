# Typesense Search Integration Research Report

This report documents the Typesense search integration details extracted from [hgsearch.ridhom.dev](https://hgsearch.ridhom.dev/) (HealthyGamerGG Video Search). It covers connection parameters, query shapes, and the exact structures of the response payload, along with guidance for implementing a similar search feature.

---

## 1. Connection & Endpoint Configuration

The search queries are processed by a self-hosted Typesense instance:

- **Host Protocol**: `https`
- **Host Domain**: `soteria.ridhom.dev`
- **Port**: `443`
- **Search Public API Key**: `w90jjKVS4odEx9Vmmbq05GaW93TRRUi4`
- **Primary Search Collection**: `youtube_captions_main`
- **Alternative Collection**: `chosen_podcasts` (used for custom podcast search routes)

Requests are sent to the standard Typesense Multi-Search endpoint:
`POST https://soteria.ridhom.dev/multi_search?x-typesense-api-key=w90jjKVS4odEx9Vmmbq05GaW93TRRUi4`

---

## 2. Request Structure

When searching for the word `"depression"`, the search adapter fires a request in the shape of a Typesense Multi-Search query. This allows searching across one or more collections in a single round-trip.

### Request Payload (`POST`)

```json
{
  "searches": [
    {
      "collection": "youtube_captions_main",
      "q": "depression",
      "query_by": "text,title",
      "group_by": "video_id",
      "group_limit": 5,
      "drop_tokens_threshold": 2,
      "typo_tokens_threshold": 2,
      "split_join_tokens": "fallback"
    }
  ]
}
```

### Key Request Parameters Explained

- **`collection`**: The search index target (e.g. `youtube_captions_main`).
- **`q`**: The search query (e.g. `"depression"`).
- **`query_by`**: The fields to match against. Here, it searches both caption text (`text`) and the video title (`title`).
- **`group_by`**: Groups search results by a specific attribute (in this case, `video_id`). This ensures that instead of seeing a flat list of individual matching caption segments (which might belong to the same video), results are grouped by video.
- **`group_limit`**: Limits the number of hits returned *within* each group (up to `5` matching caption segments per video).
- **`drop_tokens_threshold` & `typo_tokens_threshold`**: Tolerance parameters for typographical errors and drop-off thresholds.

---

## 3. Response Structure

The raw response is returned under a `results` array containing search results for each requested search item.

> [!TIP]
> You can inspect the full fetched API response in the temp directory here: [typesense_response.json](file:///home/pronsh/Coding/ytsearch/temp_research/typesense_response.json)

### Main Structure

```json
{
  "results": [
    {
      "facet_counts": [],
      "found": 679,
      "found_docs": 37696,
      "grouped_hits": [
        {
          "found": 3786,
          "group_key": ["Y-5Yq60ascI"],
          "hits": [
            {
              "document": { ... },
              "highlight": { ... },
              "highlights": [ ... ],
              "text_match": 578730123365712000,
              "text_match_info": { ... }
            }
          ]
        }
      ]
    }
  ]
}
```

### Detailed Field Breakdown

| JSON Field | Type | Description |
| :--- | :--- | :--- |
| **`found`** | Number | Number of unique groups (matching videos) found across the index. |
| **`found_docs`** | Number | Total number of underlying documents (captions) matching the search query. |
| **`grouped_hits`** | Array | Matches grouped by `video_id`. |
| **`grouped_hits[].group_key`** | Array | The value of the field grouped by (e.g. `["Y-5Yq60ascI"]`). |
| **`grouped_hits[].found`** | Number | The total number of matching hits within *this* specific group/video. |
| **`grouped_hits[].hits`** | Array | The top matches inside this video (limited by `group_limit` in request). |

---

## 4. Inside a Hit (A Single Match)

A match contains three main blocks: the source document, the highlights, and the text match score metadata.

### 4.1. The Document Schema

The document represents a single transcript chunk (usually a few seconds of video):

```json
"document": {
  "id": "Y-5Yq60ascI_5311",
  "video_id": "Y-5Yq60ascI",
  "title": "ADHD and Depression   Lecture Time (3 NOV 2021)",
  "text": "depression",
  "prev_text": "k's guide",
  "next_text": "is the evil is the price we pay for the",
  "sequence": 5311,
  "chapter": "Why Do People Have Adhd",
  "chapterIndex": 38,
  "duration": 4319,
  "t_start_ms": 6278320,
  "t_end_ms": 6282639,
  "thumbnail": "https://i.ytimg.com/vi/Y-5Yq60ascI/maxresdefault.jpg",
  "channel": "HealthyGamerGG VODs (Unofficial)",
  "upload_date": "20211208",
  "uploader_id": "@HGVODsUnofficial"
}
```

- **`t_start_ms` & `t_end_ms`**: Timestamps in milliseconds. These allow the frontend to link directly to the correct video time (e.g. `&t=6278` seconds).
- **`prev_text` & `next_text`**: Contextual cues representing the transcript snippets immediately preceding and following the matched text. This helps the UI build a continuous reading flow.

### 4.2. Highlights

Typesense returns inline highlights using HTML tags. By default, matched keywords are surrounded with `<mark>` tags.

```json
"highlight": {
  "text": {
    "matched_tokens": ["depression"],
    "snippet": "<mark>depression</mark>"
  },
  "title": {
    "matched_tokens": ["Depression"],
    "snippet": "ADHD and <mark>Depression</mark>   Lecture Time (3 NOV 2021)"
  }
}
```

### 4.3. Text Match Metadata

Contains statistics about the relevance score used for sorting:

```json
"text_match_info": {
  "best_field_score": "1108091339008",
  "best_field_weight": 15,
  "fields_matched": 2,
  "score": "578730123365711994",
  "tokens_matched": 1
}
```

---

## 5. Architectural Flow & UX Pattern

When replicating this interface:

1. **Frontend Request**: The search input triggers a debounced query.
2. **Result Grouping**: Grouping by `video_id` lets the UI display cards representing *videos*, rather than individual sentences.
3. **Inner Timestamps**: Under each video card, the UI loops over `hits` to show clickable timestamp links (e.g. `[1:44:38] "... ADHD and <mark>Depression</mark> ..."`). Clicking a timestamp loads the YouTube video embed starting exactly at `t_start_ms / 1000`.
4. **Context Rendering**: Because the document schema includes `prev_text` and `next_text`, the UI can reconstruct the full sentence surrounding the hit so users get the context without needing to play the video.
