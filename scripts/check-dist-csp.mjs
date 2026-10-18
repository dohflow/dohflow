#!/usr/bin/env node
// Built-HTML guard for the effective Content Security Policy (personal-cfo-0hp6,
// ADR 0010).
//
// Tauri rewrites the configured CSP per HTML asset at build time: every `<style>`
// element gets a nonce, and that nonce is added to `style-src`. Under CSP rules a
// nonce in `style-src` makes browsers IGNORE `'unsafe-inline'` — so a single
// `<style>` element in the built HTML would silently switch off ADR 0010's accepted
// inline-style exception (and break Radix/Tailwind runtime styles). An inline
// `<script>` would be refused by `script-src 'self'`, and a remote script or
// stylesheet would need a remote origin the CSP does not allow. This fails the
// build output, not the config, so it catches what a bundler or plugin emits.
//
// Usage: node scripts/check-dist-csp.mjs [dist-dir]   (default apps/desktop/dist)

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const dist = process.argv[2] ?? "apps/desktop/dist";

function htmlFiles(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) return htmlFiles(full);
    return entry.endsWith(".html") ? [full] : [];
  });
}

const rules = [
  {
    name: "style element",
    // Any <style ...>…</style> — Tauri would add a nonce and disable 'unsafe-inline'.
    pattern: /<style[\s>]/i,
    why: "a <style> element makes Tauri add a style-src nonce, which disables the accepted 'unsafe-inline' exception",
  },
  {
    name: "inline script",
    // A <script> with no src attribute (its content would be inline code).
    pattern: /<script(?![^>]*\bsrc\s*=)[^>]*>/i,
    why: "inline scripts are refused by script-src 'self' (ADR 0010)",
  },
  {
    name: "remote script or stylesheet",
    pattern: /<(?:script|link)\b[^>]*\b(?:src|href)\s*=\s*["']?(?:https?:)?\/\//i,
    why: "remote origins are not allowed by the CSP — assets are self-hosted",
  },
];

let files;
try {
  files = htmlFiles(dist);
} catch (error) {
  console.error(`check-dist-csp: cannot read ${dist}: ${error.message}`);
  process.exit(2);
}
if (files.length === 0) {
  console.error(`check-dist-csp: no HTML files under ${dist} — build first`);
  process.exit(2);
}

const failures = [];
for (const file of files) {
  const html = readFileSync(file, "utf8");
  for (const rule of rules) {
    const match = html.match(rule.pattern);
    if (match) failures.push(`${file}: ${rule.name} (${match[0]}) — ${rule.why}`);
  }
}

if (failures.length > 0) {
  for (const failure of failures) console.error(`::error::${failure}`);
  process.exit(1);
}
console.log(`check-dist-csp: ${files.length} HTML file(s) under ${dist} keep the CSP effective.`);
