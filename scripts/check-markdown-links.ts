import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const markdownFiles = execFileSync(
  "git",
  ["ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "*.md"],
  { cwd: root, encoding: "utf8" },
)
  .split("\0")
  .filter((path) => path.endsWith(".md") && existsSync(resolve(root, path)));

const failures: string[] = [];
const linkPattern = /!?\[[^\]]*\]\(([^)]+)\)/g;

// Code spans and fenced blocks name repository files and package scripts;
// a renamed or deleted one leaves the instruction silently wrong.
const codePattern = /```[\s\S]*?```|`[^`\n]+`/g;
const repositoryPathPattern =
  /(?<![\w./-])((?:scripts|docs|crates|src|worker|shared|config|tests|deploy|\.github)\/[\w./@-]+)/g;
const packageScriptPattern = /\bbun run ([\w:-]+)/g;
const packageScripts = new Set(
  Object.keys(
    (
      JSON.parse(readFileSync(resolve(root, "package.json"), "utf8")) as {
        scripts?: Record<string, string>;
      }
    ).scripts ?? {},
  ),
);

for (const path of markdownFiles) {
  const body = readFileSync(resolve(root, path), "utf8");
  for (const match of body.matchAll(linkPattern)) {
    let target = (match[1] ?? "").trim();
    if (target.startsWith("<") && target.endsWith(">")) target = target.slice(1, -1);
    target = target.split(/\s+["']/u, 1)[0] ?? target;
    if (!target || target.startsWith("#") || /^[a-z][a-z0-9+.-]*:/iu.test(target)) continue;

    const fileTarget = decodeURIComponent(target.split(/[?#]/u, 1)[0] ?? "");
    if (!fileTarget) continue;
    const destination = resolve(dirname(resolve(root, path)), fileTarget);
    if (!existsSync(destination)) failures.push(`${path}: missing ${target}`);
  }
  for (const [code] of body.matchAll(codePattern)) {
    for (const match of code.matchAll(repositoryPathPattern)) {
      const reference = (match[1] ?? "").replace(/[.,:;]+$/u, "");
      if (!existsSync(resolve(root, reference))) failures.push(`${path}: missing ${reference}`);
    }
    for (const match of code.matchAll(packageScriptPattern)) {
      const script = match[1] ?? "";
      if (!packageScripts.has(script)) failures.push(`${path}: no package script ${script}`);
    }
  }
}

if (failures.length > 0) {
  // oxlint-disable-next-line no-console -- contributors need every broken path in CI output.
  console.error(
    "Broken local Markdown references:\n" + failures.map((failure) => `- ${failure}`).join("\n"),
  );
  process.exit(1);
}

// oxlint-disable-next-line no-console -- hooks and CI consume this summary.
console.log(`markdown_links=ok files=${markdownFiles.length}`);
