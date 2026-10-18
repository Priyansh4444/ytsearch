import { cloudflareTest } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    cloudflareTest({
      main: "./worker/index.ts",
      wrangler: { configPath: "./wrangler.jsonc" },
    }),
  ],
  test: {
    // workerd cannot read Vitest 5's host filesystem cache paths.
    fsModuleCache: false,
    include: ["worker/**/*.test.ts"],
    clearMocks: true,
    restoreMocks: true,
  },
});
