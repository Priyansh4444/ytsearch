import { defineConfig } from "vitest/config";
import solidPlugin from "@solidjs/vite-plugin";

export default defineConfig({
  plugins: [solidPlugin({ hot: false })],
  test: {
    environment: "jsdom",
    include: ["src/**/*.{test,spec}.{ts,tsx}", "shared/**/*.{test,spec}.{ts,tsx}"],
    setupFiles: ["./src/test/setup.ts"],
    clearMocks: true,
    restoreMocks: true,
    coverage: {
      // Solid 2's generated control-flow functions can exceed the recursive
      // range merger in @bcoe/v8-coverage. Istanbul instruments only our
      // included source files and avoids that unbounded merge path.
      provider: "istanbul",
      reporter: ["text", "json-summary"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/main.tsx", "src/test/**"],
      thresholds: {
        statements: 80,
        branches: 65,
        functions: 85,
        lines: 80,
      },
    },
  },
});
