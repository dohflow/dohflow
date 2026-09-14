/// <reference types="vite/client" />

/// Raw text of `src/styles/globals.css`, served by the `globals-css-raw` plugin in
/// `vite.config.ts`. Vitest stubs real CSS imports to an empty string, which would make
/// the ADR 0054 palette guard pass vacuously; this is the way in.
declare module "virtual:globals-css-raw" {
  const css: string;
  export default css;
}

/// The frontend source tree, served by the `source-tree` plugin for the design-token audit
/// (personal-cfo-4d8.27.4.7). The walk happens in Vite config because this package's
/// tsconfig excludes node types on purpose.
declare module "virtual:source-tree" {
  const files: { path: string; text: string }[];
  export default files;
}
