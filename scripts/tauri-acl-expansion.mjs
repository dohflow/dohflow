#!/usr/bin/env node
// Expand every window's Tauri grants into the concrete commands they allow
// (personal-cfo-io42; the method of the 2026-09-27 capability audit, §3).
//
// The capability baseline (expected-capabilities.toml) pins grant IDENTIFIERS
// such as `core:default` or `updater:default`. What an identifier actually allows
// is defined by the Tauri version, so a Tauri upgrade can widen a grant without
// any file in this repository changing. Run this before and after an upgrade and
// diff the output: that diff goes in the upgrade PR ("what changed").
//
// Reads the ACL that `tauri-build` generated for the current build:
//   apps/desktop/src-tauri/gen/schemas/acl-manifests.json   (run any cargo build first)
//   apps/desktop/src-tauri/capabilities/*.json
//
// Usage: node scripts/tauri-acl-expansion.mjs [src-tauri dir]  > acl-before.txt

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2] ?? "apps/desktop/src-tauri";
let manifests;
try {
  manifests = JSON.parse(readFileSync(join(dir, "gen/schemas/acl-manifests.json"), "utf8"));
} catch (error) {
  console.error(`tauri-acl-expansion: ${error.message} — run a cargo build in ${dir} first`);
  process.exit(2);
}

/// `key:permission` → the set of `key|command` strings it allows (and `DENY …`).
function expand(key, permission, seen = new Set()) {
  const id = `${key}:${permission}`;
  if (seen.has(id)) return new Set();
  seen.add(id);
  const manifest = manifests[key];
  if (!manifest) return new Set([`?unknown manifest ${key}`]);
  const set =
    permission === "default"
      ? manifest.default_permission
      : manifest.permission_sets?.[permission];
  if (set) {
    const out = new Set();
    for (const inner of set.permissions) {
      const split = inner.lastIndexOf(":");
      const [innerKey, innerPermission] =
        split === -1 ? [key, inner] : [inner.slice(0, split), inner.slice(split + 1)];
      for (const command of expand(innerKey, innerPermission, seen)) out.add(command);
    }
    return out;
  }
  const single = manifest.permissions?.[permission];
  if (!single) return new Set([`?unknown permission ${id}`]);
  return new Set([
    ...(single.commands?.allow ?? []).map((c) => `${key}|${c}`),
    ...(single.commands?.deny ?? []).map((c) => `DENY ${key}|${c}`),
  ]);
}

/// A capability permission identifier → [manifest key, permission name].
function split(identifier) {
  const at = identifier.lastIndexOf(":");
  // App-defined permissions are referenced unprefixed (ADR 0010 addendum 2026-07-04).
  return at === -1 ? ["__app-acl__", identifier] : [identifier.slice(0, at), identifier.slice(at + 1)];
}

const windows = new Map();
for (const file of readdirSync(join(dir, "capabilities")).filter((f) => f.endsWith(".json")).sort()) {
  const capability = JSON.parse(readFileSync(join(dir, "capabilities", file), "utf8"));
  for (const window of capability.windows ?? []) {
    const grants = windows.get(window) ?? new Map();
    for (const entry of capability.permissions ?? []) {
      const identifier = typeof entry === "string" ? entry : entry.identifier;
      grants.set(identifier, file);
    }
    windows.set(window, grants);
  }
}

for (const [window, grants] of [...windows].sort(([a], [b]) => a.localeCompare(b))) {
  console.log(`window ${window}`);
  if (grants.size === 0) console.log("  (no grants)");
  for (const [identifier, file] of [...grants].sort(([a], [b]) => a.localeCompare(b))) {
    const commands = [...expand(...split(identifier))].sort();
    console.log(`  ${identifier}  [${file}]  ${commands.length} command(s)`);
    for (const command of commands) console.log(`    ${command}`);
  }
}
