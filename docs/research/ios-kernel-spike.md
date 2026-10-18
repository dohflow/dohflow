# iOS kernel spike (MOB-0)

- **Bead:** `personal-cfo-ngkyf`
- **Date:** 2026-10-18
- **Scope:** research only. Nothing in this change builds an app target or edits a
  Cargo manifest. The throwaway probe crates and the SwiftUI scaffold lived outside
  the repository and are described below so they can be reproduced.
- **Feeds:** ADR 0080 (mobile architecture, D16, `personal-cfo-iuxhq`) and the
  standalone iOS app beads under epic `personal-cfo-i93n3`.

The question this spike answers is whether the Rust kernel can run on an iPhone
before the Sync format freeze, so that being wrong is cheap. The short answer is
yes, with no source changes and no special build environment. The part that is
still open is the UI transport (D16): the native-UI path is proven end to end, the
Tauri-on-iOS path is not.

## Verdict

**GO** for a standalone iOS app that runs the whole kernel on the device.

- The persistence stack, the [U] this spike was created to resolve, cross-compiles
  for `aarch64-apple-ios` and `aarch64-apple-ios-sim` with the pinned engine. The
  simulator reports SQLCipher `4.14.0 community` on SQLite `3.51.3`, byte for byte
  the versions `docs/architecture/stack.md` records for desktop.
- Vault creation (Argon2id, then SQLCipher), unlock, wrong-password rejection, a
  write and a Future Cash read all run on the iOS simulator. 23 existing kernel
  integration tests pass there unmodified.
- D16 is **narrowed, not closed** (see "D16 recommendation"). One bounded follow-up,
  proposed here as MOB-0b, decides between the two transports before ADR 0080 is
  written.

## Owner leg (AC 5)

| Item | Result |
|---|---|
| Xcode | 27.0, build 27A266a, at `/Applications/Xcode.app` (`xcode-select -p` prints the path as `XCode.app`; the volume is case-insensitive); Apple clang 21.0.0 (clang-2100.3.34.2) |
| License | accepted (`xcodebuild -license check` exits 0) |
| SDKs | `iPhoneOS27.0.sdk` and `iPhoneSimulator27.0.sdk` present |
| Simulator runtime | iOS 27.0 (24A434); iPhone 17 simulator used |
| Rust toolchain | 1.99.0 (pinned by `rust-toolchain.toml`) |
| rustup targets | `aarch64-apple-ios` and `aarch64-apple-ios-sim` were not installed; added with `rustup target add aarch64-apple-ios aarch64-apple-ios-sim` |
| Apple developer membership | **Individual** (owner's answer, 2026-10-18). Any later transfer to an organization account, if an entity is later formed, is tracked as `personal-cfo-dyobc` [U] |

## Leg 1: the kernel on iOS

Command, run from the repository root: `cargo check -p <crate> --target aarch64-apple-ios`.
No environment variables were set for these runs. Times are wall clock on this Mac
Mini and include dependency compilation, so the first crate to pull a dependency
pays for it.

| Crate | Result | First error | Time |
|---|---|---|---|
| core-money | pass | none | 3 s |
| core-ids | pass | none | 1 s |
| core-ledger | pass | none | 1 s |
| vault-crypto | pass | none | 3 s |
| forecast-engine | pass | none | 1 s |
| db-worker | pass | none | 19 s (includes building OpenSSL and SQLCipher) |
| finance-kernel | pass | none | 3 s |

Every other workspace crate that the desktop app links also passes, with no first
error: categorization, pay-schedule, job-runtime, observability, importer-core,
csv-importer, ofx-importer, source-presets, connector-core, simplefin-adapter,
lunchflow-adapter and synthetic-data. The two connector adapters use `ureq` with
rustls, and both compile for iOS.

`cargo check -p db-worker -p finance-kernel --target aarch64-apple-ios-sim` also
passes (19 s, OpenSSL rebuilt for the simulator slice).

### Corrections to the bead's premises

- The workspace pins `rusqlite = "=0.40.2"` (with `libsqlite3-sys` 0.38.2 and
  `openssl-src` 300.6.0+3.6.2), not 0.32.1. The feature set is the one the bead
  names: `bundled-sqlcipher-vendored-openssl`.
- `finance-kernel` no longer pulls `connector-core` or `ureq`. The network adapters
  are reached only from the desktop crate, so the kernel needs no slimmer facade
  crate to drop them.

### The SQLCipher and OpenSSL answer (AC 2)

**The [U] is resolved: the vendored build cross-compiles for iOS and the engine pin
holds.** The build needs nothing beyond Xcode and the rustup targets.

- `openssl-sys` builds OpenSSL from `openssl-src` with Xcode's clang, using the
  iPhoneOS SDK at
  `/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS27.0.sdk`.
  The simulator slice uses the matching `iPhoneSimulator27.0.sdk`. No `OPENSSL_*`,
  `CC` or `SDKROOT` override was needed.
- The outputs are arm64 archives (`lipo -info` reports non-fat arm64):
  `libcrypto.a` 24.0 MB, `libssl.a` 6.1 MB and `libsqlcipher.a` 4.3 MB in a
  debug `cargo check` build. These are build intermediates, not shipped size; see
  "Size" for what ends up in an app.
- The engine is the pinned one. A probe linked against the same crates reported
  `PRAGMA cipher_version` = `4.14.0 community` and `sqlite_version()` = `3.51.3` on
  the iOS simulator and on the Mac. ADR 0065/0068's engine pin therefore needs no
  exception for iOS, and the fallback to system OpenSSL was never needed.

**Gotcha to carry into any real build: set `IPHONEOS_DEPLOYMENT_TARGET`.** With it
unset, the two layers disagree. The C objects (SQLCipher, OpenSSL) are stamped
`minos 27.0`, the SDK version, while the Rust objects use rustc's default (`minos
14.0` for the simulator). The linker did not warn, but an app built that way
advertises the wrong minimum. Setting `IPHONEOS_DEPLOYMENT_TARGET=17.0` was honored
by both layers (`minos 17.0` in the Rust binary and in `libsqlcipher.a`). Whatever
build entry point ADR 0080 chooses must set it explicitly.

### Size

Release profile with the workspace defaults (no LTO, no `opt-level = "z"`):

| Artifact | Size |
|---|---|
| `libkernel_ffi.a` (UniFFI static library of the whole kernel), device | 64.1 MB archive |
| same, simulator | 63.2 MB archive |
| SwiftUI hello app binary linked against it with `-dead_strip` | 9.6 MB, 8.5 MB after `strip -x` |

The archive size is not what ships; the linked, dead-stripped binary is. About
8.5 MB for the full kernel plus a UI stub is small enough that binary size is not a
discriminator between the two UI options. LTO and size-optimized profiles were not
tried.

## Running on the simulator (new question from the 2026-10-18 scope note)

Vault creation runs on the iOS simulator. A throwaway probe crate (path
dependencies on `finance-kernel` and `vault-crypto`, release build for
`aarch64-apple-ios-sim`) ran under `xcrun simctl spawn` on an iPhone 17 simulator
and, for comparison, natively on the Mac.

What the probe exercised: `Kernel::create_vault`, `dispatch(CreateAccount)`,
`future_cash_forecast(90, &[])`, lock, `Kernel::unlock_vault`, and a wrong-password
unlock. All behaved as on desktop (account count 1 after reopen, wrong password
rejected, forecast read ok).

| Measure | iOS simulator (steady state) | iOS simulator (first run after boot) | Mac native |
|---|---|---|---|
| Argon2id `InteractiveDefault` (64 MiB, t=3, p=1) | 65 ms | 210 ms median | 64 ms |
| Argon2id `HighSecurity` (256 MiB, t=4, p=1) | 372 ms | 1,078 ms median | 368 ms |
| Argon2id `LegacyCompatibility` (19 MiB, t=2) | 11 ms | 26 ms | 10 ms |
| `create_vault` (KDF, SQLCipher open, migrations, seed) | 111 to 118 ms | 285 ms | 99 ms |
| `unlock_vault` | 68 ms | 149 ms | 64 ms |
| `future_cash_forecast(90)` on a one-account vault | 2.7 to 4.3 ms | 4.3 ms | 0.8 ms |
| Peak RSS (includes the 256 MiB profile run) | 284 MiB | 284 MiB | 355 MiB |

Median of five derivations per profile. Steady state is two runs after the
simulator was warm.

**How to read these numbers for `personal-cfo-0sqk` (device classes): they are not
phone numbers.** The simulator runs the same arm64 code on the Mac's own CPU, which
is why it matches the native column once warm. Real iPhone timings need a physical
device, which this spike did not have. What the run does establish is that the KDF
is bounded and cheap relative to a phone's budget on current Apple silicon (64 MiB
for under a tenth of a second here), that the `HighSecurity` profile's 256 MiB
working set is the memory-limit risk to check on low-RAM devices, and that the
first unlock after a cold start is several times slower than steady state.

The repository's own tests also run on the simulator, unmodified. The test
binaries were built with `cargo test --no-run --target aarch64-apple-ios-sim` and
executed with `xcrun simctl spawn <udid> <binary>`:

| Test target | Result |
|---|---|
| `finance-kernel` `vault_lifecycle` | 6 passed |
| `finance-kernel` `vault_state_machine` | 16 passed |
| `finance-kernel` `restore_drill` | 1 passed |

## Leg 2: the command surface (AC 3, widened by scope note 2)

The phone app reuses the whole surface, reads and writes, so the inventory covers
every IPC command. The numbers below were produced from the code and the headline
counts were re-checked by hand (162 registered commands, 49 `WriteCommand`
variants, 222 frontend source files).

### What the "kernel API" actually is

The surface the UI talks to is not `finance-kernel`. It is the desktop crate's
`apps/desktop/src-tauri/src/ipc/commands.rs` (6,219 lines) plus `ipc/dto.rs`
(4,242 lines of specta-derived DTOs), registered once in
`apps/desktop/src-tauri/src/lib.rs:45-210`. `finance-kernel` is an internal Rust
boundary of about 123 public methods with no DTOs. The `*_impl(&AppState, ...)`
functions in `commands.rs` are Tauri-free; Tauri types appear only in the thin
wrappers and in six `AppHandle` / `spawn_blocking` sites. No crate under `crates/`
depends on Tauri, specta or tokio (checked by `Cargo.toml` and source grep).

### Counts

| Measure | Count |
|---|---|
| IPC commands registered | **162** (150 in the app permission set, 12 in the destructive set; the list, the permission files and `expected-capabilities.toml` agree exactly) |
| Read-only | 55 |
| No vault state (static, OS, in-memory, log bridge) | 8 |
| Writes | 99 |
| of which through `Kernel::dispatch` only | 51 |
| of which composite (dispatch plus direct worker calls: `import_batch`, `connector_create_mapped_account`, `connector_sync`, `connector_auto_sync`) | 4 |
| of which direct `DbWorker` methods with no `WriteCommand` (settings, scenarios, assumptions, attachments, audit, balance assertions, manual entries) | 29 |
| of which vault lifecycle, key and registry | 11 |
| of which write a file outside the vault | 4 |
| `WriteCommand` variants | 49, at `crates/db-worker/src/lib.rs:255-733` |
| Portable as-is | 147 |
| Portable Rust, but takes a path chosen by a desktop file dialog | 7 |
| Portable Rust, but does network I/O | 3 (`connector_link`, `connector_sync`, `connector_auto_sync`) |
| Desktop-only | 3 (`check_for_update`, `apply_update`, `relaunch_app`) |
| Never called by the frontend | 8 |

The `WriteCommand` enum is not the whole write surface: any native wrapper has to
cover the bus, the 29 direct worker writes and the 11 lifecycle commands.

Per feature area: Future Cash 30, accounts 20, transactions 18, bills and
recurring 16, vault 12, settings 9, categories and tags 9, connectors 9, Money
Inbox 8, import 7, income 7, backup and restore 7, diagnostics 4, updater 4,
rekey 1, window/OS 1. Dropping the desktop-only, updater-adjacent and never-called
commands leaves about 149 commands, about 133 of them with no caveat once
connectors and path-bound backup commands are set aside as policy decisions.

The full per-command table (class, area, Rust entry point with line, portability,
whether the UI calls it) was produced for this spike and is summarized here
rather than reproduced; it can be regenerated from `collect_commands!` and the two
permission files.

### The first phone screens (reads and the quick-add write)

Entry points are the kernel methods; the desktop-crate impl wraps each.

| Screen | IPC command | Kernel entry point |
|---|---|---|
| Unlock gate | `vault_status`, `create_vault`, `unlock_vault`, `lock_vault`, `no_reset_warning`, `acknowledge_no_reset_warning` | `VaultController::state`, `create` (`vault.rs:628`), `unlock` (`vault.rs:651`), `lock` |
| Future Cash | `future_cash_forecast(horizon_days, scenario_ids)`; optionally `future_cash_by_account`, `cash_flow_history` | `Kernel::future_cash_forecast` (`lib.rs:3919`), then `forecast::compute` (`crates/db-worker/src/forecast/aggregate.rs:81`) |
| Safe to spend | `cash_availability`, `comfort_band`, `forecast_readiness` | `lib.rs:3588`, `:3596`, `:3619` |
| Account balances | `account_list`, `cash_tiers` | `Kernel::account_views` (`lib.rs:3568`), `cash_tiers` (`:3577`) |
| Upcoming bills | forecast events filtered to bill kinds, `recurring_bill_list`, `unconfirmed_past_due` | `lib.rs:3794`, `:3991`; mark paid via `confirm_obligation_early` |
| Backup freshness | `backup_history`, `backup_schedule_settings` | `backup_jobs.rs:178`, `:76` |
| Money Inbox | `money_inbox_list` plus the triage writes | `Kernel::money_inbox_list` (`lib.rs:4342`) |
| Quick add | `record_transaction` (needs `account_list`, `category_list`) | `Kernel::dispatch(RecordTransaction)` (`lib.rs:2756`) |

The minimum first slice is 19 commands. Future Cash is computed live on every read
(`forecast::compute` is called per request), which the simulator probe confirms is
cheap: 2.7 to 4.3 ms on a small vault.

### Dependencies that would not carry over to iOS as-is

| Concern | What the code does | Impact |
|---|---|---|
| Network | `ureq`/rustls only in `crates/connectors/*`, reached only from the desktop crate | none for the kernel; the adapters compile for iOS |
| Single-process lock | `fs2` exclusive lock on `<vault.db>.runner.lock` for the worker's lifetime (`db-worker/src/lib.rs:3963-3980`) | works on iOS, but a widget or share extension cannot open the vault while the app has it unlocked |
| `mlock` | best-effort, `#[cfg(unix)]`, failure is non-fatal (`vault-crypto/src/secret.rs:98-127`) | compiles and ran on the simulator; success inside a device sandbox is unverified |
| Backup destination | absolute, canonicalized folder path stored in the encrypted job payload (`finance-kernel/src/backup_jobs.rs:120-140`) | does not fit iOS container paths or security-scoped URLs; needs a destination abstraction |
| Background work | no timers; the kernel spawns no threads; durable jobs run only in the post-unlock sweep | matches foreground-only iOS use; no background-task scheduling needed |
| Read cost | each read opens a fresh keyed SQLCipher connection (`db-worker/src/lib.rs:1316`) | a home screen issuing 6 to 8 reads re-opens that many connections; an aggregate read avoids it |
| Link-time registries | `inventory` registries for importers, presets and connectors; the desktop crate keeps them alive with `use x as _;` anchors | the same dead-strip hazard exists in a static library; the anchors must move with the host crate |
| Keychain and biometrics | no implementation anywhere; unlock is password to Argon2id KEK to the unwrapped DEK in `<vault.db>.envelope` | Face ID needs a second key-wrapping path and an ADR; nothing exists to build on |
| Data directory | `data_dir.rs` resolves the vault directory from the Tauri app-data path | needs an iOS equivalent |

### Facade proposal (follow-up beads, none created or implemented here)

The seam belongs one layer above `finance-kernel`: at the `*_impl(&AppState, ...)`
functions and DTOs that live in the desktop crate today. Cargo features on
`finance-kernel` alone would gain little, because it already has no network or
Tauri dependency. Proposed work items, in dependency order:

1. **MOB-0b: close D16.** The bounded Tauri-on-iOS proof described under "D16
   recommendation". Blocks ADR 0080.
2. **`app-core` extraction.** Move `state.rs`, `vault_registry.rs`,
   `connector_refresh.rs`, `backup_job.rs`, `ipc/error.rs`, the `*_impl`
   functions and `ipc/dto.rs` into a Tauri-free workspace crate. The desktop crate
   keeps the `#[tauri::command]` wrappers, plugin setup and windowing. These files
   have no Tauri references today (per-file grep); the move touches about 11k lines
   plus `tests/ipc_commands.rs`. Keep specta behind a feature so the same DTOs can
   serve a native binding generator.
3. **Host seam.** Replace `tauri::async_runtime` calls (the six wrappers plus the
   post-unlock job sweep) and `app_data_dir()` with a small host trait for data
   directory, blocking spawn and build info.
4. **`connectors` cargo feature.** Gate the connector crates and the nine
   `connector_*` commands so a phone build can ship without network crates if the
   standalone app does not link banks.
5. **`home_snapshot` aggregate read.** One call for vault status, balances, the
   forecast summary, upcoming obligations, inbox count and backup freshness.
6. **`BackupSink` abstraction.** Replace "absolute folder path stored forever" with
   a sink and source (app container, document picker, share sheet). Keep the
   existing package format so a desktop backup restores onto a phone.
7. **Biometric unlock ADR and design.** A second DEK wrapper behind a Keychain or
   Secure Enclave key with a biometric access control, alongside the password
   envelope, without weakening ADR 0002's no-reset posture.
8. **Process model.** Decide up front whether widgets or extensions exist; the
   runner lock makes the vault single-process.
9. **Mobile capability set.** Exclude the updater commands, the OS probe in
   `build_info` and the eight never-called commands from the phone surface.
10. **CI.** Add an iOS `cargo check` of the kernel crates and a simulator run of
    the vault tests, so the property this spike proved cannot regress silently.
11. **Phone UI shell**, only if the Tauri path is chosen: see below.

## Leg 3: D16

The bead allows either leg. I ran the cheaper, native one fully and attempted the
Tauri one to the point where it stopped.

### Option (b), run: UniFFI plus a SwiftUI scaffold

A throwaway crate (`kernel_ffi`, `crate-type = ["staticlib", "lib"]`, `uniffi`
0.32.2 in proc-macro mode) wrapped `finance_kernel::Kernel` in one UniFFI object
with `open_or_create`, `add_account`, `summary` and `lock`. `uniffi-bindgen
generate --library <libkernel_ffi.a> --language swift` produced the Swift bindings
straight from the iOS static library. A short SwiftUI view was compiled with
`swiftc -target arm64-apple-ios17.0-simulator`, bundled by hand, installed with
`xcrun simctl install` and launched on the iPhone 17 simulator.

Result: the app created a SQLCipher vault in its Documents directory in 142 ms,
added an account, read back one account and a 90-day Future Cash forecast (2.7 ms),
and locked the vault. `Kernel` is `Send`, so a `Mutex<Option<Kernel>>` satisfied
UniFFI's object requirements with no change to the kernel. The build produced no
errors and the only warning was a sysroot notice from the generated module map.

### Option (a), attempted: Tauri v2 on iOS

Not proven, and the reason matters for the decision.

- The desktop crate already has mobile preparation: `crate-type` includes
  `staticlib` and `cdylib`, and `run()` carries `tauri::mobile_entry_point`. It has
  no `gen/apple` project yet, and it is a standalone workspace.
- `cargo check --target aarch64-apple-ios-sim` on `apps/desktop/src-tauri` fails in
  `tauri` 2.11.2's build script. `swift-rs` 1.0.7 fails to compile Tauri's own
  `mobile/ios-api` Swift package with `unable to resolve module dependency: 'UIKit'`
  (and `AppKit`, `WebKit`), because the Swift invocation mixes the iPhoneSimulator
  SDK with a `MacOSX27.0.sdk` and an `arm64-apple-macos12.0` target. A second
  attempt with `SDKROOT` pinned to the simulator SDK failed identically, so I
  stopped there per the two-attempt rule.
- This is evidence, not a verdict. A bare `cargo check` is not how Tauri builds for
  iOS; the supported entry is `tauri ios init` / `tauri ios build`, which drives
  Xcode. That path needs `xcodegen`, which is not installed on this Mac, and I did
  not install tooling on the owner's machine unasked. The Tauri CLI itself is
  present (`@tauri-apps/cli` ~2.11.2 in `node_modules`). Whether the failure is a
  Tauri 2.11.2 / Xcode 27 incompatibility or only an artifact of calling cargo
  directly is the one open question.

### Trade-offs the plan names

| Dimension | Tauri v2 iOS (reuse the React UI) | SwiftUI over UniFFI (native) |
|---|---|---|
| UI reuse | Reuses the typed command layer and `bindings.ts` as-is, plus the data hooks, schemas and forms. The layout does not carry over: a fixed 224 px sidebar, 20 responsive class uses in 127 components, tables at 560 to 900 px minimum width. A phone shell and card-list variants are real work (about 20k lines of desktop-density screens in `future-cash/`, `transactions/`, `money-inbox/` and `accounts/`). | Reuses none of the frontend: 37,747 lines in 221 files, tests and the generated `bindings.ts` excluded (222 files and 40,520 lines with it). All 12 tabs and the vault screens are rebuilt in SwiftUI. UniFFI bindings must also be written for about 133 commands and their DTOs (`dto.rs` is 4,242 lines of specta derives, which would need UniFFI derives alongside). |
| Native feel | WKWebView. Achievable, but it is a web UI. | Fully native. |
| Binary size | Not measured. The kernel alone linked to 8.5 MB; add the WebView shell and the built frontend. | 8.5 MB measured for kernel plus UI stub. |
| Background fetch | Not needed for the standalone app: the kernel has no timers and jobs run in the post-unlock sweep. Same for both. | Same. |
| Face ID and Secure Enclave | No code exists today. From a WebView app this goes through a Tauri plugin or a custom Swift plugin [I: not verified in this spike]. | Direct `LocalAuthentication` and Keychain access from Swift; the Secure Enclave wrapping key is a natural fit. |
| App Store posture | Guideline 4.2.7(e) is not triggered by either option, because the kernel runs on the device. A WebView-heavy app draws more minimum-functionality scrutiny than a native one [I; the guideline text must be re-fetched before MOB-1]. | Lowest-risk posture. |
| Desktop and phone in lockstep (Sync) | One UI codebase and one DTO set for both platforms. | Two UIs that must be kept in step by hand. |
| Proven here | No. Blocked at the toolchain, see above. | Yes, end to end on the simulator. |

### D16 recommendation

**Decide after MOB-0b, with Tauri-on-iOS as the leading hypothesis and SwiftUI over
UniFFI as the proven fallback.** This is a judgment, not something the spike
measured, and ADR 0080 is the owner's decision.

The reasoning: the phone needs a new shell under either option, so the real
comparison is what survives the rewrite. Under Tauri it is the typed command layer,
the data hooks and the form logic, which is most of the 37,747 lines' value and is
the part that must stay in lockstep with desktop once Sync lands. Under SwiftUI it
is only the kernel. For a single maintainer, that is the deciding difference, as
long as Tauri can actually build and run on iOS. The spike could not establish
that, so recommending it unconditionally would be recommending an unproven path
over a proven one.

**MOB-0b (bounded, about a day):**

1. Owner step: install `xcodegen` (the Tauri CLI needs it for `tauri ios init`):
   `brew install xcodegen`. It is the only missing piece.
2. In a scratch copy of `apps/desktop` (never in the repository), run
   `pnpm tauri ios init`, then `pnpm tauri ios build` for the simulator, and open
   the existing vault screen with the kernel linked.
3. Record whether the `swift-rs` failure above reproduces under the supported
   entry point, and whether the real shell reaches the unlock screen on the
   simulator.
4. If it builds and runs, choose Tauri and write ADR 0080 around the phone shell
   work. If it does not, or if the fix is not small, choose SwiftUI over UniFFI and
   write ADR 0080 around the binding work and a native UI plan.

## Risks and open items

- **UI transport unproven for Tauri (D16).** Closed by MOB-0b.
- **Deployment target.** Must be set explicitly (see Leg 1); a silent 14.0 versus
  27.0 mismatch is the default.
- **Toolchain age.** Everything here ran on Xcode 27.0 build 27A266a. The Tauri
  failure above may be specific to this Xcode.
- **No physical device.** Simulator timings are Mac timings. Argon2 profile
  calibration for `personal-cfo-0sqk`, `mlock` behavior in a device sandbox and
  memory pressure under the 256 MiB profile all need a real iPhone.
- **Face ID and Keychain.** No code exists; a second key-wrapping path needs an ADR.
- **Process model.** The runner lock rules out an extension reading the vault while
  the app is unlocked.
- **Backup destinations** are absolute paths today.
- **App Store.** Guideline 4.2.7(e) must be re-checked against current text; the
  in-app purchase rules (3.1.1 and 3.1.3) must be re-fetched before MOB-1; the
  encryption export-compliance declaration for a bundled OpenSSL and SQLCipher
  needs confirming [I].
- **Apple account.** Individual membership today; any later transfer is tracked as
  `personal-cfo-dyobc` [U].

## Reproducing this

All commands ran from the repository root on `main` at `6d8c79a` unless noted.

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo check -p finance-kernel --target aarch64-apple-ios      # and each crate above
IPHONEOS_DEPLOYMENT_TARGET=17.0 \
  cargo test -p finance-kernel --test vault_lifecycle --no-run --target aarch64-apple-ios-sim
xcrun simctl boot <udid> && xcrun simctl spawn <udid> <test-binary>
```

The probe and the UniFFI scaffold were scratch crates outside the repository with
path dependencies on `finance-kernel` and `vault-crypto`, built with
`cargo +1.99.0 build --release --target aarch64-apple-ios-sim`. They are not kept;
their shape is described above and in the facade proposal.

## Time box

Leg 1 and the simulator runs took roughly 20 minutes of wall clock, most of it
the first OpenSSL build. The UniFFI scaffold took about 15 minutes end to end. The
Tauri attempt was stopped after two failures of the same kind, about 10 minutes.
The command-surface inventory ran in parallel and took about 10 minutes.
