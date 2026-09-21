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
required check. The successful evidence run from the repaired candidate is:

- **Run URL:** <https://github.com/dohflow/dohflow/actions/runs/35546349584>
- **Artifact:** <https://github.com/dohflow/dohflow/actions/runs/35546349584/artifacts/10615929425>
  (`linux-appimage-spike-35546349584`; contains the AppImage, build
  log/warnings, `linux-spike-apt-packages.txt`, `linux-spike-versions.txt`,
  SQLCipher probe, Argon2 timing, launch log, and `vault-screen.png`)

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

The run resolved `ubuntu-latest` to Ubuntu 24.04.5 LTS (Noble), x86_64, with
glibc 2.39, WebKitGTK 4.1 package 2.52.6-0ubuntu0.24.04.1, GTK 3 package
3.24.41-4ubuntu1.3, Node v22.23.2, pnpm 11.5.2, and Rust 1.96.0. The exact
`/etc/os-release`, `uname -a`, package versions, and toolchain output are in the
artifact's `linux-spike-versions.txt`; the exact apt list is in
`linux-spike-apt-packages.txt`.

## Build and launch procedure

The job runs `pnpm install --frozen-lockfile`, `pnpm build`, then:

```text
pnpm tauri build --bundles appimage \
  --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

It recorded 422.26 seconds of build time and an 87,366,136-byte AppImage. The
captured warning lines include the existing frontend chunk-size warning; no
icon or desktop-entry failure was reported. The AppImage is launched with a
temporary `PCFO_DATA_DIR` under `xvfb-run`; the script waits for a live window
named `DohFlow`, captures it with ImageMagick, and appends the marker
`DohFlow vault screen detected` to the launch log. ImageMagick capture retries
transient X11 mapping failures for up to ten seconds before reporting a real
smoke failure. The run produced an 800x600 `vault-screen.png` and the marker
`DohFlow vault screen detected: window=2097155 title=DohFlow process=62217`.
This is a fresh no-vault run, so the visible first screen is the vault gate and
no real data is used.

## Evidence captured by the run

### SQLCipher

The job records the active `rusqlite` feature
(`bundled-sqlcipher-vendored-openssl`) and runs a temporary Finance Kernel
probe. The linked versions were `PRAGMA cipher_version=4.5.7 community` and
`rusqlite sqlite_version=3.45.3`. The temporary source is removed before the
job ends; no production code is added.

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
records the median wall-clock time with memory, time-cost, and parallelism. On
this runner it measured `median_unlock_ms=1747.21` with
`memory_kib=65536`, `time_cost=3`, and `parallelism=1`. It uses the production
`vault-crypto` implementation and deletes the probe before the job ends.

### Owner-only arm64 VM leg

The owner VM leg is **DEFERRED**: `personal-cfo-zz7f7` (MACH-1) is still open,
and the bead explicitly says to defer this leg until the Mac Mini cutover if
the spike runs first. No arm64 VM claim is inferred from the hosted x86_64
runner.

## Verdict for ADR 0079

**GO for the hosted x86_64 AppImage floor; DEFER the owner arm64 VM leg.** The
measured floor is Ubuntu 24.04.5 LTS, WebKitGTK 4.1, and glibc 2.39 on
`ubuntu-latest`. This is a CI/build-spike verdict only: Linux is not a release
tier, no Linux updater channel is claimed, and ADR 0079 must still decide the
distribution and signing policy before any product release decision. The
candidate compatibility floor remains **Ubuntu 22.04 LTS or newer, WebKitGTK
4.1, and glibc >= 2.35**, subject to a future older-runner check.
If a future run cannot reproduce the AppImage or vault-screen smoke, record a
`NO-GO` for that runner instead of widening the floor silently.

ADR 0079 must decide:

1. AppImage-only v1 versus adding deb/rpm bundles.
2. The updater manifest key and channel policy if Linux distribution ships.
3. Where minisign signing of a CI-built Linux artifact occurs under D9b.
4. Whether the owner VM leg is sufficient for second-device Sync development or
   a separate Linux machine is required.
