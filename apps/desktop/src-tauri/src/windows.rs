//! Window topology (personal-cfo-2no, ADR 0010 and its 2026-09-27 addendum).
//!
//! Three trust classes, one label each:
//!
//! * [`MAIN`] — the trusted app UI. Declared in `tauri.conf.json` with
//!   `"create": false` and built here by [`build_main`], so it gets the same
//!   builder hooks as the shells.
//! * [`IsolatedSurface::DocumentPreview`] / [`IsolatedSurface::AgentReport`] —
//!   untrusted display-only shells, opened on demand by [`open_isolated_shell`].
//!
//! Tauri serves one invoke handler to every webview, so an untrusted shell is not
//! isolated by lacking the handler: it is isolated by its capability file
//! (`capabilities/document-preview.json`, `capabilities/agent-report.json`), which
//! grants nothing — the app ACL then rejects every command from that label in Rust
//! before dispatch. Navigation outside the app origin is cancelled for every
//! webview by `navigation_guard`; new-window requests are denied here, per window.
//!
//! No IPC command opens a shell yet: the features that render into them
//! (personal-cfo-vrng, personal-cfo-z6pn) add their own opener with their own
//! review, so no unfinished surface is reachable from the UI.

use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{Manager, Runtime, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Label of the trusted main window — the only label `capabilities/default.json`
/// and `capabilities/destructive.json` target.
pub const MAIN: &str = "main";

/// The static, script-free page every untrusted shell loads from the app bundle
/// (`apps/desktop/public/`). Content delivery is decided by vrng/z6pn under their
/// own ADR 0010 addendum.
pub const SHELL_PAGE: &str = "isolated-shell.html";

/// An untrusted, display-only window. Its label is the one its empty capability
/// file targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsolatedSurface {
    /// Imported-document preview (content: personal-cfo-vrng).
    DocumentPreview,
    /// LLM agent report (content: personal-cfo-z6pn).
    AgentReport,
}

impl IsolatedSurface {
    /// Every untrusted surface.
    pub const ALL: [Self; 2] = [Self::DocumentPreview, Self::AgentReport];

    /// The window label, which its capability file targets.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DocumentPreview => "document_preview",
            Self::AgentReport => "agent_report",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::DocumentPreview => "Document preview",
            Self::AgentReport => "Agent report",
        }
    }
}

/// The new-window hook every window is built with: `window.open` and
/// `target="_blank"` never create a webview or popup. wry already creates none on
/// macOS/Linux without a handler, but WebView2 on Windows opens one by default.
fn deny_new_window<R: Runtime>(url: Url, _features: NewWindowFeatures) -> NewWindowResponse<R> {
    // Scheme + host only: a path or query string may carry user data.
    tracing::warn!(
        scheme = url.scheme(),
        host = url.host_str().unwrap_or(""),
        "denied a webview new-window request"
    );
    NewWindowResponse::Deny
}

/// Build the trusted main window from its `tauri.conf.json` entry.
///
/// # Errors
/// Returns [`tauri::Error::WindowNotFound`] if `tauri.conf.json` declares no
/// `main` window, or an error if the window cannot be created.
pub fn build_main<R: Runtime, M: Manager<R>>(manager: &M) -> tauri::Result<WebviewWindow<R>> {
    let config = manager
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN)
        .cloned()
        .ok_or(tauri::Error::WindowNotFound)?;
    WebviewWindowBuilder::from_config(manager, &config)?
        .on_new_window(deny_new_window)
        .build()
}

/// Open an untrusted shell, or focus it if it is already open (one window per
/// label). Closing the window destroys it; the next call builds a fresh one.
///
/// # Errors
/// Returns an error if the window cannot be created or focused.
pub fn open_isolated_shell<R: Runtime, M: Manager<R>>(
    manager: &M,
    surface: IsolatedSurface,
) -> tauri::Result<WebviewWindow<R>> {
    if let Some(existing) = manager.get_webview_window(surface.label()) {
        existing.set_focus()?;
        return Ok(existing);
    }
    WebviewWindowBuilder::new(manager, surface.label(), WebviewUrl::App(SHELL_PAGE.into()))
        .title(surface.title())
        .inner_size(720.0, 800.0)
        .on_new_window(deny_new_window)
        .build()
}

/// Debug-build smoke fixture (personal-cfo-2no): with `PCFO_SMOKE_OPEN_SHELLS=1`,
/// open every untrusted shell at launch so a reviewer can probe real WebViews —
/// invoke a command, navigate away, call `window.open` — from the inspector.
/// Compiled out of release builds entirely, so it is not a production switch.
///
/// # Errors
/// Returns an error if a shell cannot be opened.
#[cfg(debug_assertions)]
pub fn open_smoke_shells_if_requested<R: Runtime, M: Manager<R>>(manager: &M) -> tauri::Result<()> {
    if std::env::var("PCFO_SMOKE_OPEN_SHELLS").as_deref() == Ok("1") {
        for surface in IsolatedSurface::ALL {
            open_isolated_shell(manager, surface)?;
        }
    }
    Ok(())
}
