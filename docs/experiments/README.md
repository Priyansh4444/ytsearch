# Experiments

Experiments are isolated from the supported production path. Each experiment
must record its workload, invariants, measurements, correctness evidence, and
rollback before it can become a production proposal.

## Search backends

- [Search experiment index](search/README.md)
- [Common backend results](search/common-results.md)
- [Measurement methodology](search/methodology.md)
- [Raw inverted-index design](search/raw-inverted-index.md)
- [Raw validation](search/raw-validation.md)
- [Tantivy design](search/tantivy.md)

Historical reports retain their original control labels. Raw was promoted to
production in August 2026; Typesense is now a stopped, deprecated rollback and
differential oracle preserved outside the live read/write path.

## Indexing

- [Fetch, import, and scaling audit](indexing/README.md)
