import { defineConfig } from "vite";

export default defineConfig({
  base: "./",
  build: {
    target: "es2022",
    // The wasm module is the only large asset; keep it a separate file so the
    // browser can cache it independently of the app shell.
    assetsInlineLimit: 0,
  },
  server: { port: 4001 },
});
