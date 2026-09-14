//! The HTTP seam. Everything network-shaped goes through [`Transport`] so the
//! adapter's tests run entirely on fixtures; [`UreqTransport`] is the one real
//! implementation (blocking `ureq`, rustls).
//!
//! LEAK RULE (connector-core `ConnectorError` docs): for SimpleFIN the request
//! URL **is** the credential, so a [`TransportError`]'s message must never
//! contain a URL — [`UreqTransport`] maps failures to their error *kind* only.
//! The claim response body is the access URL itself, so [`HttpResponse`]'s
//! `Debug` never prints the body.
//!
//! Hardening (adversarial-review findings): the agent follows **no
//! redirects** — ureq's defaults strip `Authorization` on redirect (a Bridge
//! host migration would silently 403 and misread as an expired credential)
//! and rewrite the one-time claim `POST` into a `GET`. A 3xx surfaces to the
//! caller as an explicit status instead. An overall 60s timeout bounds a
//! stalled peer (sync runs on vault open — it must never hang the app), and
//! `https_only` refuses plaintext by construction.

use std::sync::OnceLock;
use std::time::Duration;

/// A completed HTTP exchange. Error statuses (3xx/402/403/429/…) are returned
/// as a normal response — SimpleFIN puts meaningful bodies on them (a Bridge
/// 403 body is itself an AccountSet) — never as a transport error.
#[derive(Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

// The claim exchange's body IS the access URL — never Debug-print it.
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
/// *class* only — never the URL.
#[derive(Debug, thiserror::Error)]
#[error("transport failure: {0}")]
pub struct TransportError(pub String);

/// The adapter's whole network surface: an empty-bodied POST (the claim) and
/// an authenticated GET (everything else).
pub trait Transport: Sync {
    /// `POST` with an explicit `Content-Length: 0` (the Bridge's claim
    /// endpoint expects it) and no body.
    ///
    /// # Errors
    /// [`TransportError`] on network-level failure only.
    fn post_empty(&self, url: &str) -> Result<HttpResponse, TransportError>;

    /// `GET` with optional HTTP Basic credentials and query parameters.
    /// Credentials are passed explicitly — never embedded in `url` — so the
    /// transport can log/trace nothing secret even by accident.
    ///
    /// # Errors
    /// [`TransportError`] on network-level failure only.
    fn get(
        &self,
        url: &str,
        basic: Option<(&str, &str)>,
        query: &[(String, String)],
    ) -> Result<HttpResponse, TransportError>;
}

// A borrowed transport is a transport — lets tests hand the adapter
// `&FixtureTransport` and keep inspecting the fixture's recorded requests.
impl<T: Transport + ?Sized> Transport for &T {
    fn post_empty(&self, url: &str) -> Result<HttpResponse, TransportError> {
        (**self).post_empty(url)
    }

    fn get(
        &self,
        url: &str,
        basic: Option<(&str, &str)>,
        query: &[(String, String)],
    ) -> Result<HttpResponse, TransportError> {
        (**self).get(url, basic, query)
    }
}

/// The production transport. Const-constructible (the agent builds lazily in
/// a `OnceLock`) for the static registry entry; one shared agent serves the
/// ≤24 requests/day budget comfortably.
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
                // No redirect following: ureq strips Authorization on
                // redirect and turns the claim POST into a GET — a 3xx must
                // surface as itself, not as a mystery 403.
                .redirects(0)
                // Bound a stalled peer; sync runs on vault open.
                .timeout(Duration::from_secs(60))
                .https_only(true)
                .build()
        })
    }

    fn finish(result: Result<ureq::Response, ureq::Error>) -> Result<HttpResponse, TransportError> {
        match result {
            Ok(resp) => Self::read(resp),
            // HTTP error statuses carry meaningful SimpleFIN bodies.
            Err(ureq::Error::Status(_, resp)) => Self::read(resp),
            // Kind only — a ureq transport Display can embed the URL.
            Err(ureq::Error::Transport(t)) => Err(TransportError(t.kind().to_string())),
        }
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
    fn post_empty(&self, url: &str) -> Result<HttpResponse, TransportError> {
        Self::finish(self.agent().post(url).set("Content-Length", "0").call())
    }

    fn get(
        &self,
        url: &str,
        basic: Option<(&str, &str)>,
        query: &[(String, String)],
    ) -> Result<HttpResponse, TransportError> {
        use base64::Engine as _;
        let mut request = self.agent().get(url);
        if let Some((user, pass)) = basic {
            // The scratch strings hold the credential — zeroize on drop. The
            // header copy ureq owns is the accepted residual (nit-tracked).
            let joined = zeroize::Zeroizing::new(format!("{user}:{pass}"));
            let token =
                zeroize::Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(&*joined));
            request = request.set("Authorization", &format!("Basic {}", token.as_str()));
        }
        for (key, value) in query {
            request = request.query(key, value);
        }
        Self::finish(request.call())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_response_debug_never_prints_the_body() {
        let response = HttpResponse {
            status: 200,
            body: "https://user:secret@bridge.example/simplefin".to_owned(),
        };
        let debug = format!("{response:?}");
        assert!(!debug.contains("secret"), "body leaked: {debug}");
        assert_eq!(debug, "HttpResponse { status: 200, body: [44 bytes] }");
    }
}
