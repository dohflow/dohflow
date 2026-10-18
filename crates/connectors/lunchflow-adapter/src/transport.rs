//! The HTTP seam. Everything network-shaped goes through [`Transport`] so the
//! adapter's tests run entirely on fixtures; [`UreqTransport`] is the one real
//! implementation (blocking `ureq`, rustls).
//!
//! LEAK RULE: the API key is passed to [`Transport::get`] separately from the
//! URL and travels only in the `x-api-key` header, so no URL, error or log line
//! can carry it. A [`TransportError`] names a failure *kind* only.
//!
//! Hardening mirrors simplefin-adapter: no redirects (a 3xx surfaces as itself,
//! and the key header is never replayed to another host), a 60s overall
//! timeout (refresh runs on vault open and must never hang the app), and
//! `https_only`.

use std::sync::OnceLock;
use std::time::Duration;

/// A completed HTTP exchange. Error statuses are returned as a normal
/// response — LunchFlow puts `{error, message}` bodies on them — never as a
/// transport error.
#[derive(Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

// Bodies are the user's financial data — never Debug-print them.
impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HttpResponse {{ status: {}, body: [{} bytes] }}",
            self.status,
            self.body.len()
        )
    }
}

/// A transport-level failure (DNS, TLS, timeout, connect). Carries a failure
/// *class* only.
#[derive(Debug)]
pub struct TransportError(pub String);

/// The adapter's whole network surface: an authenticated GET.
pub trait Transport: Sync {
    /// `GET url?query` with the key in the `x-api-key` header. The key is
    /// passed explicitly — never embedded in `url` — so nothing that logs a
    /// URL can leak it.
    ///
    /// # Errors
    /// [`TransportError`] on network-level failure only.
    fn get(
        &self,
        url: &str,
        api_key: &str,
        query: &[(String, String)],
    ) -> Result<HttpResponse, TransportError>;
}

// A borrowed transport is a transport — lets tests hand the adapter
// `&FixtureTransport` and keep inspecting the fixture's recorded requests.
impl<T: Transport + ?Sized> Transport for &T {
    fn get(
        &self,
        url: &str,
        api_key: &str,
        query: &[(String, String)],
    ) -> Result<HttpResponse, TransportError> {
        (**self).get(url, api_key, query)
    }
}

/// The production transport. Const-constructible (the agent builds lazily in
/// a `OnceLock`) for the static registry entry.
pub struct UreqTransport {
    agent: OnceLock<ureq::Agent>,
}

impl UreqTransport {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            agent: OnceLock::new(),
        }
    }

    fn agent(&self) -> &ureq::Agent {
        self.agent.get_or_init(|| {
            ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Duration::from_secs(60))
                .https_only(true)
                .build()
        })
    }

    fn read(resp: ureq::Response) -> Result<HttpResponse, TransportError> {
        let status = resp.status();
        let body = resp
            .into_string() // ureq caps this at 10 MiB — bounded by default
            .map_err(|_| TransportError("unreadable response body".to_owned()))?;
        Ok(HttpResponse { status, body })
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for UreqTransport {
    fn get(
        &self,
        url: &str,
        api_key: &str,
        query: &[(String, String)],
    ) -> Result<HttpResponse, TransportError> {
        let mut request = self
            .agent()
            .get(url)
            .set("x-api-key", api_key)
            .set("Accept", "application/json");
        for (key, value) in query {
            request = request.query(key, value);
        }
        match request.call() {
            Ok(resp) => Self::read(resp),
            Err(ureq::Error::Status(_, resp)) => Self::read(resp),
            // Kind only — a ureq transport Display can embed the URL.
            Err(ureq::Error::Transport(t)) => Err(TransportError(t.kind().to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_response_debug_never_prints_the_body() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"accounts":[{"name":"Everyday Checking"}]}"#.to_owned(),
        };
        let debug = format!("{response:?}");
        assert!(!debug.contains("Checking"), "body leaked: {debug}");
    }
}
