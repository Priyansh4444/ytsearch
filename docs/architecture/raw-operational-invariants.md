# Raw operational invariants

Each row is a production failure and the code that now prevents it from
recurring. Keep the guard when changing the code it names; add a row for any
new failure class.

| Failure seen | Guard | Where |
| --- | --- | --- |
| The reader advertised `raw-index-schema-8-v2` while the worker built `schema-7-v2`; every publication failed after an hour-long build (2026-09-27) | Readiness reports `generation_format`; worker preflight rejects a mismatch before building. Tests pin the advertised format to the schema `build_index` writes and to the receiver's schema | `youtube_cli::require_remote_publication_api`, `engine/tests/format_invariants.rs`, `remote_publication/tests.rs` |
| The publication allowlist and the reader's file list drifted when `broad-single.bin` was added | Test that published files equal the reader's schema-7 list, and that every hot file belongs to every readable schema | `engine/tests/format_invariants.rs` |
| A published generation went live cold; the first searches faulted from the volume for 9–32 s | `ReloadableEngine::replace` accepts only a `WarmEngine`, produced by `warmed_for_activation`; every activation path, including reinstalling an existing generation, warms first | `engine/storage.rs`, `reloadable.rs`, `publisher.rs` |
| Warm-up inside activation outran the worker's 30 s request timeout and failed the publication | Compile-time assertion that `ACTIVATION_WARM_BUDGET` leaves headroom under `ADMIN_REQUEST_TIMEOUT`, which the client uses | `engine/storage.rs`, `youtube_index_store` |
| `prepare` exported the 964 MB mutation store inside the worker's 30 s admin request; the request timed out, the reader dropped the handler, and no job was recorded, so two daily publications failed and every retry restarted the export (2026-09-28) | Prepare uses the publication transfer timeout, like other requests whose work grows with the corpus; the reader runs prepare as a detached task, so an abandoned request still records its job and a retry returns it. Tests fail if the client reverts to the admin timeout or the reader lets a dropped request lose the job | `youtube_index_store::RawIndexAdminClient::prepare_publication`, `remote_publication::prepare`, `http/tests.rs`, `remote_publication/tests.rs` |
| The startup checksum evicted the whole index; uploads evicted the scanned files | One `HOT_MAPPED_FILES` list drives checksum retention, upload retention, and prefetch | `engine/query_support.rs`, `remote_publication.rs` |
| Partition planning inside open held the health check for minutes | Startup opens warm in the background; publication warms before activation | `engine/storage.rs` |
| One caption-less new video failed the whole daily pass, which then reran immediately | Gaps-only catalog runs exit 75 and complete; failed passes back off an hour | `youtube_cli::catalog_outcome`, `cloud-index-queue-entrypoint.sh` |
| A pass blocked by YouTube at the same creator every hour never reached later creators | A blocked pass records the creator; the next pass resumes after it | `reconciliation_manifest::resume_order` |
| A repeated query word broke ordered-phrase ranking | Queries keep their full token sequence for phrase evidence | `engine/tests/repeated_tokens.rs` |
| The reader bound only IPv4, so the worker could not reach it over private IPv6 | Cost/config guard requires `[::]:8108` | `scripts/check-fly-cost-config.sh` |
