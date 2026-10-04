// Lists dependencies with newer releases, including the cases `bun outdated`
// and `cargo update` do not show.
//
//   bun scripts/deps-outdated.ts      (bun run deps:outdated)
//
// JS: compares each package.json version with every npm dist-tag, so a
// prerelease line such as Solid 2 (published under `next`, while `latest` is
// still 1.x) shows its newest release. Rust: compares each direct dependency
// with the newest stable crate, flagging new major versions, which
// `cargo update` never applies.
/* oxlint-disable no-console -- a CLI whose output is its result. */
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const USER_AGENT = "ytsearch-deps-outdated (https://github.com/Priyansh4444/ytsearch)";

type Row = [name: string, current: string, available: string];

function semverParts(version: string): number[] {
  return (version.match(/\d+/g) ?? []).slice(0, 3).map(Number);
}

function newer(candidate: string, current: string): boolean {
  const [a, b] = [semverParts(candidate), semverParts(current)];
  for (let index = 0; index < 3; index++) {
    if ((a[index] ?? 0) !== (b[index] ?? 0)) return (a[index] ?? 0) > (b[index] ?? 0);
  }
  // Same x.y.z: a release beats its prereleases; prereleases compare by tail.
  const [preA, preB] = [candidate.split("-")[1], current.split("-")[1]];
  if (!preA || !preB) return !preA && Boolean(preB);
  return Number(preA.match(/(\d+)$/)?.[1] ?? 0) > Number(preB.match(/(\d+)$/)?.[1] ?? 0);
}

async function json<T>(url: string): Promise<T> {
  const response = await fetch(url, { headers: { "user-agent": USER_AGENT } });
  if (!response.ok) throw new Error(`${response.status} from ${url}`);
  return (await response.json()) as T;
}

async function javascriptRows(): Promise<Row[]> {
  const manifest = JSON.parse(readFileSync("package.json", "utf8")) as Record<
    string,
    Record<string, string> | undefined
  >;
  const declared = { ...manifest.dependencies, ...manifest.devDependencies };
  const rows = await Promise.all(
    Object.entries(declared).map(async ([name, spec]): Promise<Row | undefined> => {
      const current = spec.replace(/^[\^~]/, "");
      const tags = await json<Record<string, string>>(
        `https://registry.npmjs.org/-/package/${name}/dist-tags`,
      );
      // A stable pin moves along stable releases; a prerelease pin may also
      // move to a newer prerelease of the same line.
      const isPrerelease = current.includes("-");
      const candidates = Object.values(tags).filter(
        (version) => (isPrerelease || !version.includes("-")) && newer(version, current),
      );
      const best = candidates.sort((a, b) => (newer(a, b) ? -1 : 1))[0];
      return best ? [name, current, best] : undefined;
    }),
  );
  return rows.filter((row): row is Row => row !== undefined);
}

async function rustRows(): Promise<Row[]> {
  // cargo metadata lists every workspace crate's declared requirements;
  // `path` is set for workspace-internal dependencies.
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    }),
  ) as { packages: { dependencies: { name: string; req: string; path?: string }[] }[] };
  const declared = new Map<string, string>();
  for (const pkg of metadata.packages) {
    for (const dependency of pkg.dependencies) {
      if (!dependency.path) declared.set(dependency.name, dependency.req.replace(/^[\^~=]/, ""));
    }
  }
  const major = (version: string) => {
    const [x = 0, y = 0] = semverParts(version);
    return x > 0 ? `${x}` : `0.${y}`;
  };
  const rows = await Promise.all(
    [...declared].map(async ([name, version]): Promise<Row | undefined> => {
      const { crate } = await json<{ crate: { max_stable_version: string } }>(
        `https://crates.io/api/v1/crates/${name}`,
      );
      return major(crate.max_stable_version) !== major(version)
        ? [name, version, `${crate.max_stable_version} (new major)`]
        : undefined;
    }),
  );
  return rows.filter((row): row is Row => row !== undefined);
}

const print = (title: string, rows: Row[]) => {
  console.log(title);
  if (rows.length === 0) console.log("  up to date");
  for (const [name, current, available] of rows.sort((a, b) => a[0].localeCompare(b[0]))) {
    console.log(`  ${name.padEnd(34)} ${current.padEnd(18)} -> ${available}`);
  }
};
print("JS (package.json, all dist-tags):", await javascriptRows());
print(
  "Rust (direct dependencies, new majors only; `cargo update` handles the rest):",
  await rustRows(),
);
