# Linux AppImage build spike (LIN-1)

- **Bead:** `personal-cfo-xcrsk`
- **Scope:** exploratory Linux AppImage build/launch proof; no release channel or
  required CI check is introduced.
- **Workflow job:** `linux-appimage-spike` in `.github/workflows/ci.yml`.
- **Runner:** `ubuntu-latest` (recorded by the manual run below).

This spike exists before ADR 0079 so that the ADR records a measured floor,
not an untested promise. The existing `ipc-codegen` job already proves the
standalone desktop crate compiles on Linux; this job adds the real frontend,
AppImage bundling, an Xvfb window smoke, and the platform-specific evidence
that the ADR needs.

## Run record

The job is manual-only (`workflow_dispatch`) and is intentionally not a
required check. Replace these placeholders after dispatching the branch:

- **Run URL:** pending
- **Artifact:** pending (`linux-appimage-spike-<run-id>`; contains the AppImage,
  build log/warnings, runner versions, SQLCipher probe, Argon2 timing, launch
  log, and `vault-screen.png`)

## Exact runner setup

The job installs this exact apt package list:

```text
build-essential
libwebkit2gtk-4.1-dev
libgtk-3-dev
libayatana-appindicator3-dev
librsvg2-dev
libxdo-dev
libssl-dev
patchelf
xvfb
xdotool
imagemagick
```

The run records `/etc/os-release`, `uname -a`, glibc from
`getconf GNU_LIBC_VERSION`, the `libwebkit2gtk-4.1-dev` and GTK package
versions, Node 22, pnpm 11.5.2, and the pinned Rust toolchain in the artifact.

## Build and launch procedure

The job runs `pnpm install --frozen-lockfile`, `pnpm build`, then:

```text
pnpm tauri build --bundles appimage \
  --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

It records elapsed build time, AppImage byte size, and every build-log line
mentioning a warning, icon, or desktop entry. The AppImage is launched with a
temporary `PCFO_DATA_DIR` under `xvfb-run`; the script waits for a live window
named `DohFlow`, captures it with ImageMagick, and appends the marker
`DohFlow vault screen detected` to the launch log. This is a fresh no-vault
run, so the visible first screen is the vault gate and no real data is used.

## Evidence captured by the run

### SQLCipher

The job records the active `rusqlite` feature (`bundled-sqlcipher-vendored-openssl`)
and runs a temporary Finance Kernel probe that prints both
`PRAGMA cipher_version` and the linked SQLite version exposed by
`finance_kernel::sqlite_version()` on the Linux runner. The
temporary source is removed before the job ends; no production code is added.

### Updater with no Linux channel

`tauri.conf.json` has the macOS GitHub Releases endpoint only and no Linux
endpoint. The job verifies that configuration, then proves the updater plugin
initializes without blocking the locked vault window. The release-channel
update check is mounted after unlock; therefore a Linux manifest omission cannot
prevent the first vault screen from appearing. No Linux update channel is
claimed by this spike.

### Capability and CSP regression

The job asserts that `app.security.csp` is present and runs the desktop
`acl_coverage` integration test on Linux. This is the same deny-by-default
capability/CSP contract used by the existing IPC CI gate.

### Argon2id

A temporary probe measures five `Profile::InteractiveDefault` derivations and
records the median wall-clock time with memory, time-cost, and parallelism. It
uses the production `vault-crypto` implementation and deletes the probe before
the job ends.

### Owner-only arm64 VM leg

The owner VM leg is **DEFERRED**: `personal-cfo-zz7f7` (MACH-1) is still open,
and the bead explicitly says to defer this leg until the Mac Mini cutover if
the spike runs first. No arm64 VM claim is inferred from the hosted x86_64
runner.

## Verdict for ADR 0079

**DEFER — pending the manual workflow run.** After the run, replace this line
with `GO`, `NO-GO`, or `DEFER`, include the exact measured floor, and keep the
arm64 VM result explicit. The candidate floor to evaluate is **Ubuntu 22.04 LTS
or newer, WebKitGTK 4.1, and glibc >= 2.35**, subject to the runner evidence.

ADR 0079 must decide:

1. AppImage-only v1 versus adding deb/rpm bundles.
2. The updater manifest key and channel policy if Linux distribution ships.
3. Where minisign signing of a CI-built Linux artifact occurs under D9b.
4. Whether the owner VM leg is sufficient for second-device Sync development or
   a separate Linux machine is required.
