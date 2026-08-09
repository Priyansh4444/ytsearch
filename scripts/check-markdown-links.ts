import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ignoredPrefixes = [".git/", ".local-notes/", "node_modules/", "target/"];
const markdownFiles: string[] = [];

function collectMarkdownFiles(directory: string, relativeDirectory = ""): void {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const relativePath = join(relativeDirectory, entry.name);
    const normalizedPath = relativePath.replaceAll("\\", "/");
    if (
      entry.isDirectory() &&
      !ignoredPrefixes.some((prefix) => `${normalizedPath}/`.startsWith(prefix))
    ) {
      collectMarkdownFiles(join(directory, entry.name), relativePath);
    } else if (entry.isFile() && entry.name.endsWith(".md")) {
      markdownFiles.push(normalizedPath);
    }
  }
}

collectMarkdownFiles(root);

const failures: string[] = [];
const linkPattern = /!?\[[^\]]*\]\(([^)]+)\)/g;

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
}

if (failures.length > 0) {
  // oxlint-disable-next-line no-console -- contributors need every broken path in CI output.
  console.error(
    "Broken local Markdown links:\n" + failures.map((failure) => `- ${failure}`).join("\n"),
  );
  process.exit(1);
}

// oxlint-disable-next-line no-console -- hooks and CI consume this summary.
console.log(`markdown_links=ok files=${markdownFiles.length}`);
