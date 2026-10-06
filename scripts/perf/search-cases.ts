// The /api/search requests the perf tools replay: every benchmark case
// (scripts/benchmark-search-cases.tsv) under each sort and page.
import { readFileSync } from "node:fs";

const SORTS = ["relevance", "published_desc", "views_desc"];
const PAGES = [1, 2];

export function searchCaseParams(casesFile: string): URLSearchParams[] {
  const requests: URLSearchParams[] = [];
  for (const line of readFileSync(casesFile, "utf8").split("\n")) {
    if (!line.trim() || line.startsWith("#")) continue;
    const [, query, youtuber] = line.split("\t");
    for (const sort of SORTS) {
      for (const page of PAGES) {
        const params = new URLSearchParams({ q: query ?? "", sort, page: String(page) });
        if (youtuber) params.set("youtuber", youtuber);
        requests.push(params);
      }
    }
  }
  return requests;
}
