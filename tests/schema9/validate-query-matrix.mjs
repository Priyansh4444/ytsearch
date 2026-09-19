#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const path = fileURLToPath(new URL("./query-matrix.json", import.meta.url));
const pinnedSha = "4ab1279df8cb0debb8f877a59917320fac86a2c4ef43ab26b08153fa1927e492";
const counts = Object.freeze({
  common: 40,
  phrase: 40,
  typo: 30,
  prefix: 25,
  negation: 25,
  filter: 35,
  rank: 35,
  unicode: 30,
  threshold: 20,
  empty: 20,
});
const sorts = new Set([
  "relevance",
  "published_desc",
  "published_asc",
  "relevance_engagement",
  "views_desc",
  "likes_desc",
  "comments_desc",
  "magic_desc",
]);
const requiredRegressions = new Set([
  "slow-common-query",
  "curspr-cursor",
  "create-effect",
  "convex-con",
  "react-ordering",
  "one-fire",
  "coding-is-dead",
  "css-dead",
  "blackhole",
  "threshold-boundary",
]);
const matrix = JSON.parse(readFileSync(path, "utf8"));

assert.equal(matrix.schema_version, 1);
assert.equal(matrix.matrix_id, "schema9-v2-acceptance-300");
assert.equal(matrix.immutable_ids, true);
assert.deepEqual(matrix.category_counts, counts);
assert.deepEqual(matrix.execution, {
  valid_page_sizes: [1, 3, 10, 20],
  run_valid_cases_to_exhaustion: true,
  independent_exhaustive_reference_required: true,
});
assert.equal(matrix.cases.length, 300);

const ids = new Set();
const names = new Set();
const requests = new Set();
const regressions = new Set();
const pendingGates = new Set();
const actual = Object.fromEntries(Object.keys(counts).map((category) => [category, 0]));
for (const c of matrix.cases) {
  assert.match(
    c.id,
    /^(common|phrase|typo|prefix|negation|filter|rank|unicode|threshold|empty)-\d{3}-[a-z0-9-]+$/,
  );
  assert(!ids.has(c.id), `duplicate immutable ID: ${c.id}`);
  ids.add(c.id);
  assert.equal(typeof c.name, "string", `${c.id}: name`);
  assert(c.name.length > 5 && !names.has(c.name), `${c.id}: name must be unique and descriptive`);
  names.add(c.name);
  assert(Object.hasOwn(actual, c.category), `${c.id}: category`);
  actual[c.category] += 1;
  assert(Array.isArray(c.regression_tags), `${c.id}: regression_tags`);
  c.regression_tags.forEach((tag) => regressions.add(tag));

  const r = c.request;
  const requestKey = JSON.stringify(r);
  assert(!requests.has(requestKey), `${c.id}: duplicate full request`);
  requests.add(requestKey);
  assert.equal(typeof r.q, "string", `${c.id}: request.q`);
  assert.equal(typeof r.scope, "object", `${c.id}: request.scope`);
  assert(["all", "creator", "planet"].includes(r.scope.kind), `${c.id}: scope.kind`);
  if (r.scope.kind === "creator")
    assert.equal(typeof r.scope.handle, "string", `${c.id}: scope.handle`);
  assert(sorts.has(r.sort), `${c.id}: sort`);
  assert(Number.isInteger(r.grammar_version), `${c.id}: grammar_version`);
  assert(Number.isInteger(r.rank_version), `${c.id}: rank_version`);
  assert(
    Array.isArray(r.page_sizes) && r.page_sizes.length > 0 && r.page_sizes.every(Number.isInteger),
    `${c.id}: page_sizes`,
  );
  assert(Array.isArray(r.filters), `${c.id}: filters`);
  assert(r.cursor === null || typeof r.cursor === "string", `${c.id}: cursor`);
  assert.deepEqual(
    r.include,
    { exact_totals: true, tier0_overview: true, highlights: true },
    `${c.id}: include`,
  );

  assert(
    ["success", "error", "decision_pending"].includes(c.expected.kind),
    `${c.id}: expected.kind`,
  );
  if (c.expected.kind === "success") {
    assert(
      ["all", "creator"].includes(r.scope.kind),
      `${c.id}: successful request cannot use an invalid scope`,
    );
    assert.notEqual(
      r.sort,
      "magic_desc",
      `${c.id}: successful request cannot use an unsupported sort`,
    );
    assert.equal(r.grammar_version, 2, `${c.id}: successful request grammar version`);
    assert.equal(r.rank_version, 2, `${c.id}: successful request rank version`);
    assert.deepEqual(r.page_sizes, [1, 3, 10, 20], `${c.id}: valid page sizes`);
    assert.deepEqual(
      c.expected.exactness,
      {
        exhaustive_reference: true,
        enumerate_all_pages: true,
        totals_approximate: false,
        stable_across_page_sizes: true,
      },
      `${c.id}: exactness`,
    );
    assert(
      Array.isArray(c.expected.assertions) && c.expected.assertions.length > 0,
      `${c.id}: assertions`,
    );
    assert.deepEqual(Object.keys(c.expected.legacy).sort(), [
      "disposition",
      "expected_video_ids",
      "reason",
    ]);
  } else if (c.expected.kind === "error") {
    assert.deepEqual(Object.keys(c.expected.error).sort(), [
      "code",
      "message_contains",
      "parameter",
    ]);
    for (const field of Object.values(c.expected.error))
      assert.equal(typeof field, "string", `${c.id}: error field`);
  } else {
    pendingGates.add(c.expected.gate_id);
    assert.match(c.expected.gate_id, /^(QG|QV|QS|QR)-[0-9]{2}$/u, `${c.id}: pending gate ID`);
    assert.equal(typeof c.expected.reason, "string", `${c.id}: pending reason`);
    assert(c.expected.reason.length > 0, `${c.id}: pending reason must be explicit`);
    assert(Array.isArray(c.expected.allowed_outcomes), `${c.id}: pending outcomes`);
    assert(
      c.expected.allowed_outcomes.length >= 2,
      `${c.id}: pending outcomes must preserve the unresolved choice`,
    );
  }
}
assert.deepEqual(actual, counts);
for (const tag of requiredRegressions)
  assert(regressions.has(tag), `missing named regression: ${tag}`);
assert.deepEqual(
  pendingGates,
  new Set([
    "QG-01",
    "QG-02",
    "QG-03",
    "QG-04",
    "QG-05",
    "QG-06",
    "QG-07",
    "QV-01",
    "QS-01",
    "QS-02",
    "QS-03",
    "QS-04",
    "QR-01",
    "QR-02",
  ]),
  "every unresolved query-contract gate must remain explicit in the matrix",
);

const sha = createHash("sha256").update(JSON.stringify(matrix)).digest("hex");
assert.equal(
  sha,
  pinnedSha,
  "matrix changed: preserve IDs, review semantics, then deliberately update the digest",
);
// oxlint-disable-next-line no-console -- CI and operators consume this compact receipt.
console.log(`schema9 query matrix: ok (${matrix.cases.length} cases, sha256=${sha})`);
