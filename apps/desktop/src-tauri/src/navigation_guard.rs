//! Top-level navigation guard for every webview (personal-cfo-2rf, ADR 0010).
//!
//! The CSP governs what a page may *load*; it does not govern where the page may
//! *go*. Without this guard, a single `location.href = "https://…"` (or a plain
//! `<a href>` click) from a compromised or buggy renderer would replace the
//! trusted app UI with a remote page inside the main window. ADR 0010: "No in-app
//! window is granted remote-navigation" — external pages open in the system
//! browser through the scoped opener grant, never in a WebView.
//!
//! The guard allows exactly the origin Tauri itself serves the app from:
//!
//! * the bundled-asset protocol — `tauri://localhost` on macOS/Linux, or its wry
//!   workaround `http(s)://tauri.localhost` on Windows/Android only (elsewhere that
//!   host is just loopback, where any local server could answer), never with a port;
//! * in a dev build only (`tauri::is_dev()`, the same switch Tauri uses to load
//!   `build.devUrl` instead of the bundle), the dev server's origin, so HMR's full
//!   reloads keep working under `pnpm tauri dev`.
//!
//! Everything else — remote `http(s)`, `file:`, `data:`, `blob:`, `javascript:`,
//! and look-alike hosts — is cancelled.

use tauri::plugin::TauriPlugin;
use tauri::{Manager, Runtime, Url};

/// Whether this platform serves the bundle from wry's `http(s)://tauri.localhost`
/// workaround rather than `tauri://localhost` — the same rule Tauri applies.
const HTTP_APP_ORIGIN: bool = cfg!(windows) || cfg!(target_os = "android");

/// Whether a webview may navigate to `url`. `http_app_origin` is whether the bundle
/// is served over wry's `http(s)://tauri.localhost` workaround ([`HTTP_APP_ORIGIN`]);
/// `dev_url` is `build.devUrl`, passed only when the running build actually loads
/// from it (`tauri::is_dev()`).
#[must_use]
pub fn is_allowed_navigation(url: &Url, http_app_origin: bool, dev_url: Option<&Url>) -> bool {
    let bundled_app = url.port().is_none()
        && match url.scheme() {
            "tauri" => !http_app_origin && url.host_str() == Some("localhost"),
            "http" | "https" => http_app_origin && url.host_str() == Some("tauri.localhost"),
            _ => false,
        };
    bundled_app
        || dev_url.is_some_and(|dev| {
            matches!(dev.scheme(), "http" | "https") && url.origin() == dev.origin()
        })
}

/// The guard as a plugin, so it applies to every webview the app creates —
/// including windows declared in `tauri.conf.json`, which have no builder to hook.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|webview, url| {
            let dev_url = if tauri::is_dev() {
                webview.config().build.dev_url.as_ref()
            } else {
                None
            };
            let allowed = is_allowed_navigation(url, HTTP_APP_ORIGIN, dev_url);
            if !allowed {
                // Scheme + host only: a path or query string may carry user data.
                tracing::warn!(
                    webview = webview.label(),
                    scheme = url.scheme(),
                    host = url.host_str().unwrap_or(""),
                    "blocked webview navigation outside the app origin"
                );
            }
            allowed
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("test URL parses")
    }

    const DEV: &str = "http://localhost:1420";

    /// Navigations blocked on every platform and in every build.
    const HOSTILE: [&str; 11] = [
        "https://dohflow.app/",
        "https://example.com/",
        "http://example.com/",
        "https://tauri.localhost.evil.example/",
        "https://evil.example/?next=tauri://localhost",
        "tauri://evil.example/",
        "file:///etc/passwd",
        "data:text/html,<script>alert(1)</script>",
        "blob:tauri://localhost/6f1c",
        "javascript:alert(1)",
        "about:blank",
    ];

    #[test]
    fn macos_and_linux_allow_only_the_tauri_scheme_origin() {
        for u in [
            "tauri://localhost",
            "tauri://localhost/",
            "tauri://localhost/index.html#/settings",
        ] {
            assert!(
                is_allowed_navigation(&url(u), false, None),
                "{u} must be allowed"
            );
        }
        // There `tauri.localhost` is plain loopback: any local server could answer.
        for u in [
            "http://tauri.localhost/",
            "https://tauri.localhost/",
            "tauri://localhost:8080/",
        ] {
            assert!(
                !is_allowed_navigation(&url(u), false, None),
                "{u} must be blocked"
            );
        }
    }

    #[test]
    fn windows_and_android_allow_only_the_wry_workaround_origin() {
        for u in [
            "http://tauri.localhost/",
            "https://tauri.localhost/accounts?tab=1",
        ] {
            assert!(
                is_allowed_navigation(&url(u), true, None),
                "{u} must be allowed"
            );
        }
        for u in ["tauri://localhost/", "http://tauri.localhost:8080/"] {
            assert!(
                !is_allowed_navigation(&url(u), true, None),
                "{u} must be blocked"
            );
        }
    }

    #[test]
    fn remote_and_non_app_schemes_are_blocked_everywhere() {
        let dev = url(DEV);
        for u in HOSTILE.into_iter().chain(["ws://localhost:1420/"]) {
            for http_app_origin in [false, true] {
                for dev_url in [None, Some(&dev)] {
                    assert!(
                        !is_allowed_navigation(&url(u), http_app_origin, dev_url),
                        "{u} must be blocked (http_app_origin={http_app_origin}, dev={dev_url:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn the_dev_server_is_allowed_only_when_the_build_loads_from_it() {
        let dev = url(DEV);
        assert!(is_allowed_navigation(
            &url("http://localhost:1420/"),
            false,
            Some(&dev)
        ));
        assert!(is_allowed_navigation(
            &url("http://localhost:1420/src/main.tsx"),
            false,
            Some(&dev)
        ));
        // A release build passes no dev URL, so the dev origin is just another host.
        assert!(!is_allowed_navigation(
            &url("http://localhost:1420/"),
            false,
            None
        ));
        // Same host, different port or scheme, is a different origin.
        assert!(!is_allowed_navigation(
            &url("http://localhost:1421/"),
            false,
            Some(&dev)
        ));
        assert!(!is_allowed_navigation(
            &url("https://localhost:1420/"),
            false,
            Some(&dev)
        ));
    }

    #[test]
    fn a_non_web_dev_url_never_widens_the_policy() {
        let odd = url("file:///tmp/dist/index.html");
        assert!(!is_allowed_navigation(
            &url("file:///tmp/dist/index.html"),
            false,
            Some(&odd)
        ));
    }

    #[test]
    fn this_platform_uses_the_origin_tauri_serves_the_bundle_from() {
        // Mirrors tauri's `tauri_protocol_url` (manager/mod.rs, tauri 2.11.2).
        assert_eq!(
            HTTP_APP_ORIGIN,
            cfg!(windows) || cfg!(target_os = "android")
        );
    }
}
