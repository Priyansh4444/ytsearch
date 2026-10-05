import tailwindcss from "@tailwindcss/vite";
import solidPlugin from "@solidjs/vite-plugin";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [tailwindcss(), solidPlugin()],
  server: {
    port: 3000,
    proxy: {
      // YTSEARCH_DEV_API=https://ytsearch-api-raw-production.fly.dev runs the
      // frontend against real data without a local backend.
      "/api": {
        target: process.env.YTSEARCH_DEV_API ?? "http://127.0.0.1:8787",
        changeOrigin: true,
      },
    },
  },
  build: {
    target: "esnext",
  },
});
