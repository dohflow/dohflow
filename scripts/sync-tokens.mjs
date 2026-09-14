#!/usr/bin/env node
/**
 * Generate CSS custom properties from the canonical design tokens.
 *
 * Source of truth: docs/design-system/design-tokens.json (bead personal-cfo-x99h).
 * Consumers: the desktop app's globals.css and the marketing site's tokens.css,
 * so the two cannot drift into different greens (ADR 0061 §7).
 *
 * Usage: node scripts/sync-tokens.mjs [--out <path>]
 *        node scripts/sync-tokens.mjs --check   (exit 1 if <out> is stale)
 */
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(here, '..')
const TOKENS = resolve(repoRoot, 'docs/design-system/design-tokens.json')

const args = process.argv.slice(2)
const check = args.includes('--check')
const outIdx = args.indexOf('--out')
const outPath = outIdx !== -1 ? resolve(args[outIdx + 1]) : resolve(repoRoot, 'tokens.css')

const tokens = JSON.parse(readFileSync(TOKENS, 'utf8'))

const banner = `/* GENERATED FILE — DO NOT EDIT BY HAND.
 * Source: docs/design-system/design-tokens.json
 * Regenerate: node scripts/sync-tokens.mjs --out <path>
 * ADR 0061 §7. Contrast caveat (ADR 0054): terracotta is decorative only —
 * #db8f6b on white is ~2.6:1. Body text and links use emerald or a neutral.
 */`

const colorBlock = (mode) =>
  Object.entries(tokens.color[mode])
    .map(([k, v]) => `  --${k}: ${v};`)
    .join('\n')

const fontBlock = [
  // ADR 0063 — two families. The UI face is the DEFAULT and is also emitted as
  // --font-sans, because Tailwind's `font-sans` utility resolves to that name and
  // because defaulting to the data face is the safe direction: a surface that
  // forgets to opt in still renders sober numerals. Display is strictly opt-in.
  `  --font-ui: ${tokens.font.ui.$value};`,
  `  --font-sans: ${tokens.font.ui.$value};`,
  `  --font-display: ${tokens.font.display.$value};`,
  `  --font-numeric: ${tokens.font.features.numeric.$value};`,
  `  --font-smoothing: ${tokens.font.features.smoothing.$value};`,
].join('\n')

const scaleBlock = Object.entries(tokens.font.scale)
  .map(([k, v]) => `  --text-${k}: ${v.size};\n  --leading-${k}: ${v.lineHeight};`)
  .join('\n')

const radiusBlock = Object.entries(tokens.radius)
  .map(([k, v]) => `  --radius-${k}: ${typeof v === 'object' ? v.$value ?? v.value : v};`)
  .join('\n')

const css = `${banner}

:root {
${fontBlock}
${scaleBlock}
${radiusBlock}
${colorBlock('light')}
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
${colorBlock('dark')
    .split('\n')
    .map((l) => '  ' + l)
    .join('\n')}
  }
}

:root[data-theme="dark"] {
${colorBlock('dark')}
}
`

if (check) {
  if (!existsSync(outPath)) {
    console.error(`sync-tokens: ${outPath} does not exist. Run without --check to generate it.`)
    process.exit(1)
  }
  if (readFileSync(outPath, 'utf8') !== css) {
    console.error(`sync-tokens: ${outPath} is STALE. Regenerate with:\n  node scripts/sync-tokens.mjs --out ${outPath}`)
    process.exit(1)
  }
  console.log(`sync-tokens: ${outPath} is up to date.`)
} else {
  writeFileSync(outPath, css)
  console.log(`sync-tokens: wrote ${outPath} (${css.split('\n').length} lines)`)
}
