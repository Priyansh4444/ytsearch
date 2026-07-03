import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";
import devtools from "solid-devtools/vite";

export default defineConfig({
  plugins: [devtools(), solidPlugin()],
  server: {
    port: 3000,
    proxy: {
      "/api": "http://127.0.0.1:8787",
    },
  },
  build: {
    target: "esnext",
  },
});
