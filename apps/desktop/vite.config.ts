import { readdirSync, readFileSync, statSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, type Plugin } from "vitest/config";

/// Expose `styles/globals.css` to tests as a raw string.
///
/// The design-token palette guard (`src/styles/chart-palette.test.ts`, ADR 0054) has to
/// read the stylesheet that actually ships — a test carrying its own copy of the hexes
/// would keep passing while the app rendered something else. But Vitest stubs CSS imports
/// to an empty string by default (`test.css: false`), and that stub wins over `?raw`,
/// `?inline` and `import.meta.glob`, so the guard would silently validate an EMPTY
/// palette and pass every check vacuously. Serving the file's text under a virtual id
/// sidesteps the CSS pipeline entirely.
function globalsCssRaw(): Plugin {
  const virtualId = "virtual:globals-css-raw";
  const resolvedId = `\0${virtualId}`;
  const file = fileURLToPath(new URL("./src/styles/globals.css", import.meta.url));
  return {
    name: "personal-cfo:globals-css-raw",
    resolveId: (id) => (id === virtualId ? resolvedId : null),
    load(id) {
      if (id !== resolvedId) return null;
      this.addWatchFile(file);
      return `export default ${JSON.stringify(readFileSync(file, "utf8"))};`;
    },
  };
}

/// Serves the frontend source tree — paths plus text — to the design-token audit
/// (personal-cfo-4d8.27.4.7).
///
/// The audit has to READ every source file, but this package's tsconfig deliberately
/// excludes node types, so a test cannot import `node:fs`. Doing the walk here (Vite config
/// runs in node) keeps the filesystem out of app code and matches how the palette guard
/// already solves the same problem.
function sourceTree(): Plugin {
  const virtualId = "virtual:source-tree";
  const resolvedId = `\0${virtualId}`;
  const root = fileURLToPath(new URL("./src", import.meta.url));
  const walk = (dir: string, out: string[] = []): string[] => {
    for (const entry of readdirSync(dir)) {
      if (entry === "node_modules" || entry === "dist") continue;
      const full = `${dir}/${entry}`;
      if (statSync(full).isDirectory()) walk(full, out);
      else if (/\.(tsx?|css)$/.test(entry)) out.push(full);
    }
    return out;
  };
  return {
    name: "personal-cfo:source-tree",
    resolveId: (id) => (id === virtualId ? resolvedId : null),
    load(id) {
      if (id !== resolvedId) return null;
      const files = walk(root).map((full) => ({
        path: full.slice(root.length + 1),
        text: readFileSync(full, "utf8"),
      }));
      return `export default ${JSON.stringify(files)};`;
    },
  };
}

// Frontend build + test config for the Tauri desktop app.
// Tauri-specific hardening (capabilities, fixed dev port wiring, no remote
// navigation) is layered on in bead personal-cfo-rhci. The design system
// (Tailwind v4 + shadcn/ui tokens) lands in personal-cfo-x99h.
export default defineConfig({
  plugins: [react(), tailwindcss(), globalsCssRaw(), sourceTree()],
  resolve: {
    // `@/…` resolves to `src/…` — the shadcn/ui import convention.
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  // Tauri expects a stable dev server; do not let Vite clear the screen or
  // hop ports out from under the Rust side.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
  },
});
