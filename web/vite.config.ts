import { defineConfig } from "vite";

// Cross-origin isolation is configured up front (architecture.md): it is the
// prerequisite for a future SharedArrayBuffer display ring. Harmless now.
const coiHeaders = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  // package.json lives in web/, so the project root is web/ by default.
  server: { headers: coiHeaders },
  preview: { headers: coiHeaders },
  build: {
    target: "es2020",
    outDir: "dist",
  },
});
