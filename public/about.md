# About ytsearch

ytsearch finds the moment a creator said something. It searches the public captions of indexed YouTube channels and links straight to the timestamp, so you can check a quote, rediscover a half-remembered explanation, or see everything a channel has said about a topic without scrubbing through hours of video.

## What it covers

The index holds about 50,000 videos and nearly 10 million transcript chunks from more than 70 creators, mostly about software, science and engineering. New uploads are added automatically, and anyone can request another creator from the search box by typing its handle.

## How search works

Keyword search is a purpose-built Rust index with typo tolerance, phrase matching and ranking that keeps rare words ahead of common ones. An experimental hybrid mode adds EmbeddingGemma vectors in Qdrant so that questions and vague descriptions can match by meaning, not just by shared words.

## Who runs it

ytsearch is an independent, open-source project by Priyansh (pronsh). It is free to use, has no accounts or advertising, and is not affiliated with YouTube or Google. The [source code](https://github.com/Priyansh4444/ytsearch) is MIT licensed, and [donations](https://ko-fi.com/pronsh) help pay for hosting.

Developers and AI agents can use the same search through the [public API](https://search.pronsh.dev/docs).
