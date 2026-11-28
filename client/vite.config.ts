import { defineConfig } from "vite";

// The deployment serves the client under /play/; locally it lives at the root.
export default defineConfig({
  base: process.env.HOSTABLE_BASE ?? "/",
  // Preact's automatic JSX runtime through Vite's built-in transform.
  oxc: {
    jsx: { runtime: "automatic", importSource: "preact" },
  },
  build: {
    outDir: "dist",
    // Pack images are served from `<base>assets/`, so the app's own files
    // live somewhere else.
    assetsDir: "app",
    target: "es2020",
  },
});
