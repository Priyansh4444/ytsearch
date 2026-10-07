// Fails when a dependency that ships in a runtime artifact (the Rust binaries'
// normal-dependency closure, or the frontend's npm `dependencies` closure) has
// a license expression that the permissive allowlist cannot satisfy.
//
//   bun scripts/check-licenses.ts
/* oxlint-disable no-console -- a CLI whose output is its result. */
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const ALLOWED = new Set([
  "0BSD",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "BSL-1.0",
  "CC0-1.0",
  "ISC",
  "MIT",
  "MIT-0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "Unlicense",
  "Zlib",
]);

// SPDX expression: OR binds looser than AND; `WITH` exceptions only narrow a
// license, so the base identifier decides. Old crates write `MIT/Apache-2.0`.
export function licenseAllowed(expression: string): boolean {
  const tokens = expression.replaceAll("/", " OR ").match(/\(|\)|[^\s()]+/g) ?? [];
  let position = 0;
  const peek = () => tokens[position];
  const parseOr = (): boolean => {
    let allowed = parseAnd();
    while (peek() === "OR") {
      position++;
      allowed = parseAnd() || allowed;
    }
    return allowed;
  };
  const parseAnd = (): boolean => {
    let allowed = parseFactor();
    while (peek() === "AND") {
      position++;
      allowed = parseFactor() && allowed;
    }
    return allowed;
  };
  const parseFactor = (): boolean => {
    const token = tokens[position++];
    if (token === "(") {
      const allowed = parseOr();
      position++; // ")"
      return allowed;
    }
    if (peek() === "WITH") position += 2;
    return token !== undefined && ALLOWED.has(token.replace(/\+$/, ""));
  };
  const allowed = parseOr();
  return allowed && position === tokens.length;
}

type CargoMetadata = {
  packages: {
    id: string;
    name: string;
    version: string;
    license: string | null;
    source: string | null;
  }[];
  resolve: {
    nodes: { id: string; deps: { pkg: string; dep_kinds: { kind: string | null }[] }[] }[];
  };
};

function rustRuntimeLicenses(): [string, string][] {
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      [
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--filter-platform",
        "x86_64-unknown-linux-gnu",
      ],
      { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
    ),
  ) as CargoMetadata;
  const edges = new Map(
    metadata.resolve.nodes.map((node) => [
      node.id,
      node.deps
        .filter((dep) => dep.dep_kinds.some((kind) => kind.kind === null))
        .map((dep) => dep.pkg),
    ]),
  );
  const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const seen = new Set(metadata.packages.filter((pkg) => pkg.source === null).map((pkg) => pkg.id));
  const queue = [...seen];
  while (queue.length > 0) {
    for (const next of edges.get(queue.pop()!) ?? []) {
      if (!seen.has(next)) {
        seen.add(next);
        queue.push(next);
      }
    }
  }
  return [...seen]
    .map((id) => packages.get(id)!)
    .filter((pkg) => pkg.source !== null)
    .map((pkg) => [`crate ${pkg.name} ${pkg.version}`, pkg.license ?? "NONE"]);
}

type PackageJson = {
  name: string;
  version: string;
  license?: string;
  dependencies?: Record<string, string>;
};

function npmRuntimeLicenses(root: string): [string, string][] {
  const read = (directory: string) =>
    JSON.parse(readFileSync(join(directory, "package.json"), "utf8")) as PackageJson;
  const resolve = (from: string, name: string) =>
    [join(from, "node_modules", name), join(root, "node_modules", name)].find((candidate) =>
      existsSync(join(candidate, "package.json")),
    );
  const found: [string, string][] = [];
  const seen = new Set<string>();
  const queue = Object.keys(read(root).dependencies ?? {}).map((name) => [root, name] as const);
  while (queue.length > 0) {
    const [from, name] = queue.pop()!;
    const directory = resolve(from, name);
    if (!directory || seen.has(directory)) continue;
    seen.add(directory);
    const manifest = read(directory);
    found.push([`npm ${manifest.name} ${manifest.version}`, manifest.license ?? "NONE"]);
    for (const dependency of Object.keys(manifest.dependencies ?? {})) {
      queue.push([directory, dependency]);
    }
  }
  return found;
}

if (import.meta.main) {
  const all = [...rustRuntimeLicenses(), ...npmRuntimeLicenses(process.cwd())];
  const rejected = all.filter(([, license]) => !licenseAllowed(license));
  for (const [name, license] of rejected)
    console.error(`license not allowed: ${name} (${license})`);
  console.log(`licenses=${rejected.length === 0 ? "ok" : "failed"} packages=${all.length}`);
  process.exit(rejected.length === 0 ? 0 : 1);
}
