import tailwindcss from "@tailwindcss/vite";
import solidPlugin from "@solidjs/vite-plugin";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [tailwindcss(), solidPlugin()],
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
