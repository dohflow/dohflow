#!/usr/bin/env bash
#
# Cut and publish a signed macOS DohFlow release (personal-cfo-867.1.3).
# See docs/operations/release-checklist.md for the full owner+agent
# procedure this script implements pieces of — read that first.
#
# This script does NOT build the app. Run scripts/release.sh first: it
# signs, notarizes, and writes the .dmg / .app.tar.gz / .sig / latest.json
# under apps/desktop/src-tauri/target/release/bundle/{macos,dmg}/. This
# script picks up from there — it tags the release commit, assembles and
# checksums the upload set, drafts the GitHub Release, and — on a later,
# separate invocation, only after the owner's second-Mac Gatekeeper smoke
# test has passed against the DRAFT — publishes it and triggers the
# dohflow-site rebuild.
#
# Usage:
#   ./scripts/publish-release.sh preflight
#   ./scripts/publish-release.sh tag
#   ./scripts/publish-release.sh package
#   ./scripts/publish-release.sh draft [--notes-file FILE]
#   ./scripts/publish-release.sh publish
#   ./scripts/publish-release.sh rebuild-site
#   ./scripts/publish-release.sh verify
#
# `publish` = `gh release edit --draft=false` + `rebuild-site` + `verify`,
# in that order (bead 867.1.3's owner-decided sequence: never trigger the
# site rebuild before the release itself is actually public). Run the
# owner's second-Mac smoke test against the DRAFT from `draft` before ever
# running `publish`.
#
# Flags (any subcommand):
#   --repo owner/name   GitHub repo the release lives on. Default: dohflow/dohflow.
#   --tag vX.Y.Z        Release tag. Default: v<tauri.conf.json's version>.
#
# Env:
#   DOHFLOW_SITE_DEPLOY_HOOK_URL  Cloudflare Workers Builds deploy hook for
#                                 dohflow-site (personal-cfo-z5ag4). Required by
#                                 `rebuild-site`/`publish`. Lives in
#                                 ~/.config/personal-cfo/release.env — never in
#                                 this repo.
#   RELEASE_ALLOW_DIRTY=1         Let `preflight`/`tag` proceed from a dirty
#                                 worktree (default: refuse — same convention
#                                 as scripts/release.sh's own flag of this name).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() { echo "error: $*" >&2; exit 1; }
note() { echo "note: $*"; }
ok() { echo "✓ $*"; }

# ── Shared paths (must match scripts/release.sh's own output layout) ──────────
desktop_dir="$repo_root/apps/desktop"
tauri_conf="$desktop_dir/src-tauri/tauri.conf.json"
desktop_cargo_toml="$desktop_dir/src-tauri/Cargo.toml"
desktop_package_json="$desktop_dir/package.json"
changelog="$repo_root/CHANGELOG.md"
bundle_macos_dir="$desktop_dir/src-tauri/target/release/bundle/macos"
bundle_dmg_dir="$desktop_dir/src-tauri/target/release/bundle/dmg"
assets_dir="$desktop_dir/src-tauri/target/release/bundle/release-assets"

app_path="$bundle_macos_dir/DohFlow.app"
updater_archive="$bundle_macos_dir/DohFlow.app.tar.gz"
updater_sig="$updater_archive.sig"
latest_json_src="$bundle_macos_dir/latest.json"

# ── Flags shared by every subcommand ──────────────────────────────────────────
repo="dohflow/dohflow"
tag_override=""

version() {
  python3 -c "import json;print(json.load(open('$tauri_conf'))['version'])"
}

tag_name() {
  if [[ -n "$tag_override" ]]; then
    echo "$tag_override"
  else
    echo "v$(version)"
  fi
}

rest=()
parse_common_flags() {
  # Consumes --repo/--tag from "$@"; leaves everything else in the global
  # `rest` array. (Not `mapfile` — the macOS system bash is 3.2, which
  # doesn't have it.)
  rest=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --repo) repo="$2"; shift 2 ;;
      --tag) tag_override="$2"; shift 2 ;;
      *) rest+=("$1"); shift ;;
    esac
  done
}

# ── preflight: version consistency, changelog entry, clean tree, no existing tag ──
cmd_preflight() {
  local v problems=0
  v="$(version)"
  echo "Checking release readiness for DohFlow $v ($(tag_name))…"
  echo

  # 1. Version consistency across the three files the bead names.
  local cargo_v pkg_v
  cargo_v="$(grep -m1 '^version = "' "$desktop_cargo_toml" | sed -E 's/^version = "([^"]+)"/\1/')"
  pkg_v="$(python3 -c "import json;print(json.load(open('$desktop_package_json'))['version'])")"
  if [[ "$cargo_v" == "$v" ]]; then
    ok "apps/desktop/src-tauri/Cargo.toml version matches ($cargo_v)"
  else
    echo "✗ apps/desktop/src-tauri/Cargo.toml version is '$cargo_v', expected '$v'" >&2
    problems=$((problems + 1))
  fi
  if [[ "$pkg_v" == "$v" ]]; then
    ok "apps/desktop/package.json version matches ($pkg_v)"
  else
    echo "✗ apps/desktop/package.json version is '$pkg_v', expected '$v'" >&2
    problems=$((problems + 1))
  fi

  # 2. CHANGELOG.md has a real, dated section for this version — not the
  #    "## [0.1.0] - YYYY-MM-DD" placeholder — AND that date is actually
  #    today, the day the tag is about to be cut. A date that was correct
  #    when the section was written but never updated after go-live slipped
  #    would otherwise ship a release whose own changelog lies about when it
  #    happened; catching it here means the fix is a one-line CHANGELOG edit
  #    before tagging, not an errata after publishing.
  local heading_date today
  heading_date="$(grep -m1 -E "^## \[$v\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" "$changelog" | sed -E 's/^## \[[^]]+\] - //' || true)"
  today="$(date +%Y-%m-%d)"
  if [[ -z "$heading_date" ]]; then
    echo "✗ CHANGELOG.md has no dated '## [$v] - YYYY-MM-DD' heading (still a placeholder, or missing)" >&2
    problems=$((problems + 1))
  elif [[ "$heading_date" != "$today" ]]; then
    echo "✗ CHANGELOG.md's ## [$v] heading is dated $heading_date, but today is $today — update the heading to the actual tag date (or wait until $heading_date) before tagging" >&2
    problems=$((problems + 1))
  else
    ok "CHANGELOG.md's ## [$v] heading is dated today ($heading_date)"
  fi

  # 3. Clean, committed tree — a release must be reproducible from a commit.
  if [[ "${RELEASE_ALLOW_DIRTY:-0}" != "1" && -n "$(git -C "$repo_root" status --porcelain 2>/dev/null)" ]]; then
    echo "✗ worktree is dirty — commit/clean first, or set RELEASE_ALLOW_DIRTY=1" >&2
    problems=$((problems + 1))
  else
    ok "worktree is clean (or RELEASE_ALLOW_DIRTY=1)"
  fi

  # 4. The tag must not already exist locally or on the remote — refuse to
  #    re-tag a version silently.
  local t; t="$(tag_name)"
  if git -C "$repo_root" rev-parse -q --verify "refs/tags/$t" >/dev/null; then
    echo "✗ tag $t already exists locally" >&2
    problems=$((problems + 1))
  elif git -C "$repo_root" ls-remote --exit-code --tags origin "$t" >/dev/null 2>&1; then
    echo "✗ tag $t already exists on origin" >&2
    problems=$((problems + 1))
  else
    ok "tag $t does not already exist locally or on origin"
  fi

  # 5. Tooling sanity — soft check, not fatal here (draft/publish check gh for real).
  if command -v gh >/dev/null 2>&1; then
    ok "gh CLI is installed"
  else
    note "gh CLI not found — required later for 'draft'/'publish'"
  fi

  echo
  if [[ "$problems" -gt 0 ]]; then
    fail "$problems preflight check(s) failed — fix before tagging. See docs/operations/release-checklist.md."
  fi
  ok "preflight passed — ready to tag $t."
}

# ── tag: signed annotated tag + push ───────────────────────────────────────────
cmd_tag() {
  cmd_preflight
  local t v; t="$(tag_name)"; v="$(version)"
  echo
  echo "── Tagging $t ─────────────────────────────────────────────────"
  git -C "$repo_root" tag -s "$t" -m "DohFlow $v"
  git -C "$repo_root" push origin "$t"
  echo
  ok "pushed $t. Confirm it shows 'Verified' at:"
  echo "  https://github.com/$repo/releases/tag/$t"
  echo "  (or https://github.com/$repo/commits/$t before the release exists)"
}

# ── package: checksum + sanity-check the artifacts scripts/release.sh built ───
sanity_check_sig() {
  # Structural check, no external tools required. A Tauri v2 updater .sig
  # file's on-disk content is base64 of the FULL minisign signature-file text
  # (verified against a real build, personal-cfo-867.1.3 go-live day — an
  # earlier version of this check assumed the on-disk content was base64 of
  # the raw 74-byte blob directly, which is wrong and would have failed every
  # real release):
  #
  #   untrusted comment: signature from tauri secret key
  #   <base64 of the 74-byte signature: 2-byte alg id + 8-byte key id + 64-byte sig>
  #   trusted comment: timestamp:...\tfile:...
  #   <base64 of the trusted-comment global signature>
  #
  # So this decodes ONE level to get that text, then decodes line 2 again to
  # get the actual 74-byte signature. The algorithm id is "Ed" for a
  # non-prehashed signature or "ED" for a prehashed one (minisign's default
  # for signing files of any real size, BLAKE2b-hashes the content first) —
  # both are legitimate for the same keypair, so either is accepted. The key
  # id is compared as raw bytes against the pinned public key's own raw
  # bytes — NOT against the human-readable "minisign public key: <hex>" text
  # in the pubkey's comment line, which displays the key id byte-reversed
  # from its actual on-disk encoding and would never match a direct hex
  # comparison against the signature's raw bytes.
  python3 -c "
import base64, json, sys
sig_path, conf_path = sys.argv[1], sys.argv[2]

sig_text = base64.b64decode(open(sig_path).read().strip()).decode()
sig_lines = sig_text.splitlines()
if len(sig_lines) < 2:
    sys.exit(f'signature file decodes to {len(sig_lines)} line(s), expected at least 2 (comment + signature)')
sig = base64.b64decode(sig_lines[1])
if len(sig) != 74:
    sys.exit(f'signature is {len(sig)} bytes, expected 74 (2 alg id + 8 key id + 64 signature)')
if sig[0:2] not in (b'Ed', b'ED'):
    sys.exit(f'signature algorithm id is {sig[0:2]!r}, expected b\'Ed\' or b\'ED\' (minisign Ed25519, prehashed or not)')
sig_keyid = sig[2:10]

conf = json.load(open(conf_path))
pub_text = base64.b64decode(conf['plugins']['updater']['pubkey']).decode()
pub_lines = pub_text.splitlines()
pub_raw = base64.b64decode(pub_lines[1])
pub_keyid = pub_raw[2:10]
if sig_keyid != pub_keyid:
    sys.exit(f'signature key id {sig_keyid.hex().upper()} does not match tauri.conf.json pubkey key id {pub_keyid.hex().upper()}')
print(f'  structural check passed: 74-byte {sig[0:2].decode()}25519 signature, key id {sig_keyid.hex().upper()} matches tauri.conf.json')
" "$updater_sig" "$tauri_conf"

  if command -v minisign >/dev/null 2>&1; then
    # minisign the CLI expects the plain multi-line signature-file text, not
    # the extra base64 wrapper Tauri stores on disk — decode one level first.
    local pub_tmp sig_tmp
    pub_tmp="$(mktemp)"
    sig_tmp="$(mktemp)"
    python3 -c "
import base64, json
conf = json.load(open('$tauri_conf'))
open('$pub_tmp', 'w').write(base64.b64decode(conf['plugins']['updater']['pubkey']).decode())
open('$sig_tmp', 'w').write(base64.b64decode(open('$updater_sig').read().strip()).decode())
"
    if minisign -Vm "$updater_archive" -x "$sig_tmp" -p "$pub_tmp" >/dev/null 2>&1; then
      ok "minisign cryptographic verification passed"
    else
      rm -f "$pub_tmp" "$sig_tmp"
      fail "minisign cryptographic verification FAILED — the signature does not validate against the pinned public key."
    fi
    rm -f "$pub_tmp" "$sig_tmp"
  else
    note "minisign not installed — ran the structural check only. Install minisign for a full cryptographic verification."
  fi
}

patch_latest_json_url() {
  local t="$1" placeholder="REPLACE_WITH_THE_UPLOADED_APP_TAR_GZ_ASSET_URL"
  local url="https://github.com/$repo/releases/download/$t/DohFlow.app.tar.gz"
  python3 -c "
import json, sys
path, placeholder, url = sys.argv[1], sys.argv[2], sys.argv[3]
manifest = json.load(open(path))
current = manifest['platforms']['darwin-aarch64']['url']
if current != placeholder:
    sys.exit(f'latest.json url is \'{current}\', expected the placeholder \'{placeholder}\' — refusing to overwrite an already-filled-in value. Was package already run?')
manifest['platforms']['darwin-aarch64']['url'] = url
json.dump(manifest, open(path, 'w'), indent=2)
" "$assets_dir/latest.json" "$placeholder" "$url"
  ok "latest.json url set to $url"
}

cmd_package() {
  local t; t="$(tag_name)"

  [[ -f "$app_path/Contents/Info.plist" || -d "$app_path" ]] || fail "no build at $app_path — run scripts/release.sh first."
  local dmg=""
  shopt -s nullglob
  local dmgs=("$bundle_dmg_dir"/*.dmg)
  shopt -u nullglob
  [[ ${#dmgs[@]} -ge 1 ]] || fail "no .dmg found in $bundle_dmg_dir — run scripts/release.sh first."
  dmg="${dmgs[0]}"
  [[ -f "$updater_archive" ]] || fail "no updater archive at $updater_archive — run scripts/release.sh first."
  [[ -f "$updater_sig" ]] || fail "no updater signature at $updater_sig — run scripts/release.sh first."
  [[ -f "$latest_json_src" ]] || fail "no latest.json at $latest_json_src — run scripts/release.sh first."

  echo "Assembling release assets for $t into $assets_dir …"
  rm -f "$assets_dir/DohFlow.dmg" "$assets_dir/DohFlow.app.tar.gz" "$assets_dir/DohFlow.app.tar.gz.sig" \
        "$assets_dir/latest.json" "$assets_dir/SHA256SUMS.txt"
  mkdir -p "$assets_dir"
  cp "$dmg" "$assets_dir/DohFlow.dmg"
  cp "$updater_archive" "$assets_dir/DohFlow.app.tar.gz"
  cp "$updater_sig" "$assets_dir/DohFlow.app.tar.gz.sig"
  cp "$latest_json_src" "$assets_dir/latest.json"

  # Re-point the paths the sanity check + minisign use at the STAGED copies,
  # so what's checked is byte-for-byte what will be uploaded.
  updater_archive="$assets_dir/DohFlow.app.tar.gz"
  updater_sig="$assets_dir/DohFlow.app.tar.gz.sig"

  echo
  echo "── Sanity-checking the updater signature ─────────────────────────"
  sanity_check_sig

  echo
  echo "── Patching latest.json's asset URL ───────────────────────────────"
  patch_latest_json_url "$t"

  echo
  echo "── Writing SHA256SUMS.txt ──────────────────────────────────────────"
  ( cd "$assets_dir" && shasum -a 256 DohFlow.dmg DohFlow.app.tar.gz DohFlow.app.tar.gz.sig latest.json > SHA256SUMS.txt )
  cat "$assets_dir/SHA256SUMS.txt"

  echo
  ok "release assets ready in $assets_dir:"
  ls -la "$assets_dir"
}

# ── draft: gh release create --draft with the assembled assets ────────────────
changelog_section() {
  local v="$1"
  awk -v v="$v" '
    BEGIN { found=0 }
    $0 ~ "^## \\[" v "\\]" { found=1; next }
    found && /^## \[/ { exit }
    found { print }
  ' "$changelog"
}

cmd_draft() {
  command -v gh >/dev/null 2>&1 || fail "gh CLI is required for 'draft'. Install it: https://cli.github.com/"
  [[ -d "$assets_dir" ]] || fail "no assets in $assets_dir — run '$0 package' first."

  local t v notes_file=""
  t="$(tag_name)"; v="$(version)"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --notes-file) notes_file="$2"; shift 2 ;;
      *) fail "unknown argument to draft: $1" ;;
    esac
  done

  if [[ -z "$notes_file" ]]; then
    notes_file="$(mktemp)"
    changelog_section "$v" > "$notes_file"
    if [[ ! -s "$notes_file" ]]; then
      fail "CHANGELOG.md has no content under '## [$v]' — pass --notes-file explicitly, or fill in the changelog section first."
    fi
    note "release notes drawn from CHANGELOG.md's '## [$v]' section ($notes_file)"
  fi

  echo "Creating DRAFT release $t on $repo …"
  gh release create "$t" --repo "$repo" --verify-tag --draft \
    --title "DohFlow $v" --notes-file "$notes_file" \
    "$assets_dir/DohFlow.dmg" "$assets_dir/DohFlow.app.tar.gz" \
    "$assets_dir/DohFlow.app.tar.gz.sig" "$assets_dir/SHA256SUMS.txt" \
    "$assets_dir/latest.json"

  echo
  ok "draft created. STOP HERE and run the owner's second-Mac Gatekeeper smoke"
  echo "  test against this draft (docs/operations/release-checklist.md) before"
  echo "  ever running '$0 publish'."
}

# ── rebuild-site: POST to the Cloudflare Workers Builds deploy hook ───────────
cmd_rebuild_site() {
  : "${DOHFLOW_SITE_DEPLOY_HOOK_URL:?set it (source ~/.config/personal-cfo/release.env) — see personal-cfo-z5ag4}"
  echo "Triggering the dohflow-site rebuild…"
  curl -fsS -X POST "$DOHFLOW_SITE_DEPLOY_HOOK_URL"
  echo
  ok "rebuild triggered. Check dohflow-site → Deployments for a new build."
}

# ── verify: confirm the release + site are actually live ──────────────────────
cmd_verify() {
  local t v; t="$(tag_name)"; v="$(version)"

  echo "── Checking releases/latest/download/latest.json ────────────────"
  local status
  status="$(curl -sI -o /dev/null -w '%{http_code}' "https://github.com/$repo/releases/latest/download/latest.json")"
  if [[ "$status" == "200" ]]; then
    ok "latest.json is live (HTTP $status)"
  else
    echo "✗ latest.json returned HTTP $status, expected 200" >&2
    fail "verify failed."
  fi

  echo
  echo "── Checking dohflow.app/download mentions $v ─────────────────────"
  if curl -fsS "https://dohflow.app/download" | grep -qF "$v"; then
    ok "dohflow.app/download mentions $v"
  else
    echo "✗ dohflow.app/download does not (yet) mention $v — the site rebuild may still be in progress; re-check in a minute." >&2
    fail "verify failed."
  fi
}

# ── verify_asset_url: GitHub gives a DRAFT release's assets a temporary
# "releases/download/untagged-<hash>/<file>" URL, not the tag-based one —
# confirmed against a real draft during personal-cfo-867.1.3's go-live run,
# and it is genuinely unclear (unresolved even in upstream `gh`/GitHub issues)
# whether publishing rewrites it to the tag-based path automatically. So
# `package`'s "deterministic URL" was only ever a prediction of what the
# asset URL WILL be once published — verify it for real now that it's
# public, and correct latest.json (re-uploading it) if the prediction was
# wrong, rather than shipping an updater manifest on faith.
verify_asset_url() {
  local t="$1" expected actual
  expected="https://github.com/$repo/releases/download/$t/DohFlow.app.tar.gz"
  actual="$(gh release view "$t" --repo "$repo" --json assets \
    --jq '.assets[] | select(.name == "DohFlow.app.tar.gz") | .url')"
  if [[ -z "$actual" ]]; then
    fail "could not find the DohFlow.app.tar.gz asset on the published release to verify its URL."
  fi
  if [[ "$actual" == "$expected" ]]; then
    ok "asset URL confirmed: $actual"
    return
  fi
  echo "note: asset URL is '$actual', not the predicted '$expected' — GitHub's" >&2
  echo "  untagged-release URL did not resolve to the tag path on publish. Correcting" >&2
  echo "  latest.json and re-uploading." >&2
  python3 -c "
import json
path = '$assets_dir/latest.json'
manifest = json.load(open(path))
manifest['platforms']['darwin-aarch64']['url'] = '$actual'
json.dump(manifest, open(path, 'w'), indent=2)
"
  gh release upload "$t" --repo "$repo" --clobber "$assets_dir/latest.json"
  ok "latest.json corrected and re-uploaded with the real asset URL."
}

# ── publish: flip the draft public, then rebuild the site, then verify ────────
cmd_publish() {
  command -v gh >/dev/null 2>&1 || fail "gh CLI is required for 'publish'. Install it: https://cli.github.com/"
  local t; t="$(tag_name)"
  echo "Publishing $t on $repo (draft=false) …"
  gh release edit "$t" --repo "$repo" --draft=false
  ok "$t is now public."
  echo
  echo "── Verifying the updater asset URL ───────────────────────────────"
  verify_asset_url "$t"
  echo
  echo "Reminder — the roll-forward rule: releases are immutable once"
  echo "published. Never delete or unpublish a release; a bad release is"
  echo "superseded by a higher patch version instead."
  echo
  cmd_rebuild_site
  echo
  cmd_verify
}

usage() {
  sed -n '2,33p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

main() {
  local cmd="${1:-}"
  [[ $# -gt 0 ]] && shift

  case "$cmd" in
    preflight|tag|package|publish|rebuild-site|verify) ;;
    draft) ;;
    ""|-h|--help|help) usage; exit 0 ;;
    *) usage; fail "unknown command: $cmd" ;;
  esac

  parse_common_flags "$@"

  case "$cmd" in
    preflight) cmd_preflight ;;
    tag) cmd_tag ;;
    package) cmd_package ;;
    draft) cmd_draft "${rest[@]+"${rest[@]}"}" ;;
    publish) cmd_publish ;;
    rebuild-site) cmd_rebuild_site ;;
    verify) cmd_verify ;;
  esac
}

main "$@"
