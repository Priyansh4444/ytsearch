// Vite plugin: writes dist/third-party-licenses.txt with the license of every
// npm package that ends up in the frontend bundle, which is the code this
// project distributes to visitors. Packages come from the bundle's module ids,
// so the file always matches what shipped.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { Plugin } from "vite";

// The last node_modules segment names the package (Bun nests them under .bun).
const PACKAGE_PATH = /.*node_modules\/((?:@[^/]+\/)?[^/]+)\//;
const LICENSE_FILE = /^(licen[cs]e|copying|notice)(\.|$)/i;

type PackageJson = {
  name: string;
  version: string;
  license?: string;
  author?: string | { name?: string };
  repository?: string | { url?: string };
};

function packageRoot(moduleId: string, name: string): string {
  const marker = `node_modules/${name}/`;
  return moduleId.slice(0, moduleId.lastIndexOf(marker) + marker.length - 1);
}

function notice(root: string): string {
  const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as PackageJson;
  const author = typeof manifest.author === "string" ? manifest.author : manifest.author?.name;
  const repository =
    typeof manifest.repository === "string" ? manifest.repository : manifest.repository?.url;
  const header = [
    `${manifest.name} ${manifest.version}`,
    `License: ${manifest.license ?? "see package"}`,
    ...(author ? [`Author: ${author}`] : []),
    ...(repository ? [`Source: ${repository}`] : []),
  ];
  const texts = readdirSync(root)
    .filter((file) => LICENSE_FILE.test(file))
    .sort()
    .map((file) => readFileSync(join(root, file), "utf8").trim());
  // Some packages (effect) publish no license file; the identifier and author
  // above are then the complete notice they ship with.
  return [...header, "", ...texts].join("\n").trim();
}

export function thirdPartyNotices(): Plugin {
  return {
    name: "third-party-notices",
    apply: "build",
    generateBundle(_options, bundle) {
      const roots = new Map<string, string>();
      for (const output of Object.values(bundle)) {
        if (output.type !== "chunk") continue;
        for (const moduleId of Object.keys(output.modules)) {
          const name = PACKAGE_PATH.exec(moduleId.replaceAll("\\", "/"))?.[1];
          if (name && !roots.has(name)) roots.set(name, packageRoot(moduleId, name));
        }
      }
      const notices = [...roots.keys()]
        .sort()
        .map((name) => roots.get(name)!)
        .filter((root) => existsSync(join(root, "package.json")))
        .map(notice);
      this.emitFile({
        type: "asset",
        fileName: "third-party-licenses.txt",
        source: [
          "Third-party software in the ytsearch frontend bundle.",
          ...notices.map((text) => `${"=".repeat(78)}\n${text}`),
          "",
        ].join("\n\n"),
      });
    },
  };
}
