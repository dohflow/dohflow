#!/usr/bin/env node
/**
 * Fail if any frontend dependency's license isn't AGPL-3.0-only compatible.
 *
 * The Rust side of this check is `deny.toml` (root) and
 * `apps/desktop/src-tauri/deny.toml`, run via `cargo deny check licenses` —
 * this is the frontend equivalent (personal-cfo-o1nxk item 3). There is no
 * frontend cargo-deny analogue, so this hand-rolls the same allowlist
 * against `pnpm licenses list --json`'s output rather than pulling in a new
 * devDependency (license-checker, etc.) for a one-shot CI gate.
 *
 * `pnpm licenses list` groups installed packages by their EXACT license
 * string, including compound SPDX-style expressions pnpm emits verbatim
 * (e.g. "MIT OR Apache-2.0", "MIT AND ISC") rather than as parsed
 * expression trees — this script does a literal string allowlist match
 * against the known-good set below, not a real SPDX-expression parser. A
 * compound expression not already in ALLOW fails closed (reported, not
 * silently accepted) even if every individual clause would be fine on its
 * own — add it explicitly, with the same kind of comment the existing
 * entries carry, once you've actually confirmed it's fine.
 *
 * Usage: node scripts/check-frontend-licenses.mjs [--prod]
 *   --prod   only check "dependencies"/"optionalDependencies" (what the
 *            packaged Tauri app actually bundles) — passed straight through
 *            to `pnpm licenses list`. Without it, devDependencies (build
 *            tooling, test frameworks — never shipped) are included too,
 *            for full transparency. CI runs this with --prod.
 */
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..");
const desktopDir = resolve(repoRoot, "apps/desktop");

const prodOnly = process.argv.includes("--prod");

// Same AGPL-3.0-only compatible set as deny.toml (root and
// apps/desktop/src-tauri), plus the frontend-specific licenses the first
// real audit run actually found — each documented here, not silently
// passed, exactly as deny.toml documents CDLA-Permissive-2.0 and the
// LLVM exception on the Rust side. See
// docs/security/release-review-v0.1.md item 3 for the decision record.
const ALLOW = new Set([
  "MIT",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Zlib",
  "MPL-2.0",
  "CC0-1.0",
  "BSL-1.0",
  "OpenSSL",
  // --- Found by this script's first real run, not pre-anticipated ---
  // Compound expressions pnpm reports verbatim — every clause in each is
  // independently on the allow list above, so the combination (either
  // clause applies, or both do) is fine.
  "Apache-2.0 OR MIT",
  "MIT OR Apache-2.0",
  "MIT AND ISC",
  // Blue Oak Model License 1.0 — a modern, maximally permissive license
  // (clearer patent grant than MIT/BSD, same spirit); no AGPL conflict.
  // devDependency only (minimatch, transitively via a build tool).
  "BlueOak-1.0.0",
  // Creative Commons Attribution 4.0 — governs caniuse-lite's bundled
  // browser-support DATA, not code; requires attribution for that data,
  // imposes nothing on how code using it is licensed. devDependency only.
  "CC-BY-4.0",
  // MIT with the attribution clause removed — strictly more permissive
  // than plain MIT, already allowed above. devDependency only.
  "MIT-0",
  // SIL Open Font License 1.1 — the standard permissive FONT license
  // (governs the bundled @fontsource-variable IBM Plex Sans / Nunito font
  // files, ADR 0063), not application code. Permits bundling/redistributing
  // the fonts with the app; the only restrictions (no selling the font
  // standalone, rename if modified) apply to the font files themselves,
  // not to DohFlow's own licensing. Same category as CDLA-Permissive-2.0
  // on the Rust side: a permissive license attached to bundled data/assets.
  // SHIPS IN PRODUCTION (the app bundles these fonts).
  "OFL-1.1",
  // Python Software Foundation License 2.0 — permissive, OSI-approved.
  // devDependency only (argparse, a transitive build-tool dependency).
  "Python-2.0",
]);

let raw;
try {
  const args = ["licenses", "list", "--json"];
  if (prodOnly) args.splice(2, 0, "--prod");
  raw = execFileSync("pnpm", args, { cwd: desktopDir, encoding: "utf8" });
} catch (err) {
  console.error("error: `pnpm licenses list` failed to run:", err.message);
  process.exit(1);
}

/** @type {Record<string, Array<{name: string, versions?: string[]}>>} */
const byLicense = JSON.parse(raw);

const unknown = Object.keys(byLicense).filter((lic) => !ALLOW.has(lic));

if (unknown.length > 0) {
  console.error(
    `check-frontend-licenses: ${unknown.length} license(s) not on the AGPL-3.0-only compatible allowlist${prodOnly ? " (production dependencies)" : ""}:\n`,
  );
  for (const lic of unknown) {
    const pkgs = byLicense[lic].map((p) => p.name).join(", ");
    console.error(`  "${lic}" — ${pkgs}`);
  }
  console.error(
    "\nThis is a HUMAN_DECISION (personal-cfo-o1nxk's own escalation rule): " +
      "do not add an exception here without the owner confirming the license " +
      "is actually AGPL-3.0-compatible. See scripts/check-frontend-licenses.mjs's ALLOW set.",
  );
  process.exit(1);
}

const total = Object.values(byLicense).reduce((n, pkgs) => n + pkgs.length, 0);
console.log(
  `check-frontend-licenses: ${total} package(s) across ${Object.keys(byLicense).length} license(s), all AGPL-3.0-only compatible${prodOnly ? " (production dependencies)" : ""}.`,
);
