//! The HTTP seam. Everything network-shaped goes through [`Transport`] so the
//! adapter's tests run entirely on fixtures; [`UreqTransport`] is the one real
//! implementation (blocking `ureq` 3, rustls).
//!
//! LEAK RULE (connector-core `ConnectorError` docs): for SimpleFIN the request
//! URL **is** the credential, so a [`TransportError`]'s message must never
//! contain a URL — [`UreqTransport`] maps failures to their error *kind* only.
//! The claim response body is the access URL itself, so [`HttpResponse`]'s
//! `Debug` never prints the body.
//!
//! Hardening (adversarial-review findings): the agent follows **no
//! redirects** — a followed redirect strips `Authorization` (a Bridge
//! host migration would silently 403 and misread as an expired credential)
//! and rewrite the one-time claim `POST` into a `GET`. A 3xx surfaces to the
//! caller as an explicit status instead. An overall 60s timeout bounds a
//! stalled peer (sync runs on vault open — it must never hang the app), and
//! `https_only` refuses plaintext by construction.

use std::sync::OnceLock;
use std::time::Duration;

/// The largest response body read; a longer one is refused (ureq's own default, stated here
/// so the limit is this crate's and is tested).
pub const MAX_RESPONSE_BYTES: u64 = 10 * 1024 * 1024;

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
        self.agent
            .get_or_init(|| ureq::Agent::new_with_config(agent_config(true)))
    }
}

/// The failure *class* of a ureq error, and nothing else. ureq's own `Display` can embed the
/// URL (for SimpleFIN the URL is the credential), so it is never used. The strings are the
/// ones the ureq 2 transport produced (`ErrorKind`'s names), so what a user sees is unchanged.
fn failure_class(error: &ureq::Error) -> &'static str {
    match error {
        ureq::Error::HostNotFound => "Dns Failed",
        ureq::Error::ConnectionFailed => "Connection Failed",
        // ureq 3 reports a refused or dropped connection as an I/O error; ureq 2 named it.
        ureq::Error::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
            ) =>
        {
            "Connection Failed"
        }
        ureq::Error::BadUri(_) | ureq::Error::RequireHttpsOnly(_) => "Invalid URL",
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => "Too Many Redirects",
        _ => "Network Error",
    }
}

/// Read a response into text. Bodies up to and including [`MAX_RESPONSE_BYTES`] are read; a
/// longer one is refused.
///
/// The cap is applied to the bytes THIS function reads (`take`), after any content decoding,
/// so it bounds memory whatever the server sends. (ureq 3's own `limit` counts the raw bytes
/// and decodes above it, and the `gzip` feature is off for the same reason; this stays the
/// bound even if a feature is ever turned back on.)
fn read_response(
    response: ureq::http::Response<ureq::Body>,
) -> Result<HttpResponse, TransportError> {
    use std::io::Read as _;
    let unreadable = || TransportError("unreadable response body".to_owned());
    let (parts, mut body) = response.into_parts();
    let mut bytes = Vec::new();
    body.as_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(unreadable());
    }
    let text = String::from_utf8(bytes).map_err(|_| unreadable())?;
    Ok(HttpResponse {
        status: parts.status.as_u16(),
        body: text,
    })
}

/// The agent's configuration, shared by production and the stub-server tests (the tests
/// switch off `https_only` only, because a local stub has no certificate).
fn agent_config(https_only: bool) -> ureq::config::Config {
    ureq::Agent::config_builder()
        // No redirect following: a followed redirect strips Authorization and turns a POST
        // into a GET. A 3xx must surface as itself, as a normal response.
        .max_redirects(0)
        .max_redirects_will_error(false)
        // HTTP error statuses are normal responses: their bodies carry meaning.
        .http_status_as_error(false)
        // Bound a stalled peer; sync runs on vault open.
        .timeout_global(Some(Duration::from_secs(60)))
        .https_only(https_only)
        // ureq 3 reads proxy settings from the environment by default; ureq 2 did not. A
        // credential must not ride through an ambient proxy, so there is none.
        .proxy(None)
        .build()
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for UreqTransport {
    fn post_empty(&self, url: &str) -> Result<HttpResponse, TransportError> {
        finish(
            self.agent()
                .post(url)
                .header("Content-Length", "0")
                .send_empty(),
        )
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
            request = request.header("Authorization", format!("Basic {}", token.as_str()));
        }
        for (key, value) in query {
            request = request.query(key, value);
        }
        finish(request.call())
    }
}

fn finish(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<HttpResponse, TransportError> {
    match result {
        Ok(response) => read_response(response),
        // Class only — never ureq's Display, which can embed the URL.
        Err(error) => Err(TransportError(failure_class(&error).to_owned())),
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

    // ---- a local stub server for the wire-level properties (personal-cfo-pxi.23) ----------
    // The production agent is https-only, so these tests build the SAME agent configuration
    // with https_only off (the stub has no certificate). Everything else is production.
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread::JoinHandle;
    use std::time::Instant;

    fn plain_http_transport() -> UreqTransport {
        let transport = UreqTransport::new();
        transport
            .agent
            .set(ureq::Agent::new_with_config(agent_config(false)))
            .ok();
        transport
    }

    /// Serve ONE exchange: read the request head, run `respond`, then watch briefly for a
    /// second connection. Returns the request head and how many further connections came.
    fn stub(
        respond: impl FnOnce(&mut TcpStream) + Send + 'static,
    ) -> (u16, JoinHandle<(String, usize)>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("addr").port();
        let handle = std::thread::spawn(move || {
            let started = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(_) if started.elapsed() < Duration::from_secs(30) => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return (String::new(), 0),
                }
            };
            stream.set_nonblocking(false).expect("blocking stream");
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                head.push(byte[0]);
            }
            respond(&mut stream);
            let _ = stream.flush();
            drop(stream);
            let watch = Instant::now();
            let mut extra = 0;
            while watch.elapsed() < Duration::from_millis(400) {
                if listener.accept().is_ok() {
                    extra += 1;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            (String::from_utf8_lossy(&head).into_owned(), extra)
        });
        (port, handle)
    }

    fn body_of(length: usize) -> impl FnOnce(&mut TcpStream) + Send + 'static {
        move |stream| {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
            );
            let chunk = vec![b'a'; 64 * 1024];
            let mut left = length;
            while left > 0 {
                let n = left.min(chunk.len());
                if stream.write_all(&chunk[..n]).is_err() {
                    return;
                }
                left -= n;
            }
        }
    }

    #[test]
    fn a_body_at_the_cap_is_read_and_one_byte_over_is_refused() {
        let cap = usize::try_from(MAX_RESPONSE_BYTES).unwrap();
        let (port, at_cap) = stub(body_of(cap));
        let t = plain_http_transport();
        let response = t
            .get(
                &format!("http://127.0.0.1:{port}/accounts"),
                Some(("user", "pa55")),
                &[("version".to_owned(), "2".to_owned())],
            )
            .expect("a body at the cap is read");
        assert_eq!(response.status, 200);
        assert_eq!(response.body.len(), cap);
        at_cap.join().unwrap();

        let (port, over) = stub(body_of(cap + 1));
        let t = plain_http_transport();
        let refused = t.get(
            &format!("http://127.0.0.1:{port}/accounts"),
            Some(("user", "pa55")),
            &[("version".to_owned(), "2".to_owned())],
        );
        assert!(
            matches!(refused, Err(TransportError(ref m)) if m == "unreadable response body"),
            "{refused:?}"
        );
        over.join().unwrap();
    }

    #[test]
    fn a_redirect_is_returned_as_itself_and_never_followed() {
        let (port, server) = stub(|stream| {
            let _ = stream.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let t = plain_http_transport();
        let response = t
            .get(
                &format!("http://127.0.0.1:{port}/accounts"),
                Some(("user", "pa55")),
                &[("version".to_owned(), "2".to_owned())],
            )
            .expect("a 3xx is a response");
        let (head, extra) = server.join().unwrap();
        assert_eq!(response.status, 302);
        assert!(head.starts_with("GET /accounts"), "{head}");
        assert_eq!(extra, 0, "the redirect target must never be requested");
    }

    #[test]
    fn an_error_status_is_a_normal_response_with_its_body() {
        let (port, server) = stub(|stream| {
            let _ = stream.write_all(
                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 5\r\nConnection: close\r\n\r\nnope!",
            );
        });
        let t = plain_http_transport();
        let response = t
            .get(
                &format!("http://127.0.0.1:{port}/accounts"),
                Some(("user", "pa55")),
                &[("version".to_owned(), "2".to_owned())],
            )
            .expect("a 4xx is a response");
        server.join().unwrap();
        assert_eq!((response.status, response.body.as_str()), (403, "nope!"));
    }

    #[test]
    fn the_production_agent_refuses_plain_http_without_connecting() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let refused = UreqTransport::new().get(
            &format!("http://127.0.0.1:{port}/accounts"),
            Some(("user", "pa55")),
            &[],
        );
        assert!(
            matches!(refused, Err(TransportError(ref m)) if m == "Invalid URL"),
            "{refused:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            listener.accept().is_err(),
            "a connection was made to a plain-http URL"
        );
    }

    #[test]
    fn a_failure_names_its_class_and_never_the_url_or_credential() {
        // A port nothing listens on: bind, note the port, drop the listener.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let failure = plain_http_transport()
            .get(
                &format!("http://127.0.0.1:{port}/secret-path-token"),
                Some(("user-secret", "pa55-secret")),
                &[],
            )
            .expect_err("nothing is listening");
        assert_eq!(failure.0, "Connection Failed");
        for secret in [
            "secret-path-token",
            "user-secret",
            "pa55-secret",
            "key-not-for-logs",
        ] {
            assert!(!failure.0.contains(secret) && !format!("{failure:?}").contains(secret));
        }
    }

    #[test]
    fn a_redirected_post_is_not_followed_or_rewritten_into_a_get() {
        let (port, server) = stub(|stream| {
            let _ = stream.write_all(
                b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let response = plain_http_transport()
            .post_empty(&format!("http://127.0.0.1:{port}/claim"))
            .expect("a 3xx is a response");
        let (head, extra) = server.join().unwrap();
        assert_eq!(response.status, 307);
        assert!(head.starts_with("POST /claim "), "{head}");
        assert!(
            head.to_ascii_lowercase().contains("content-length: 0"),
            "{head}"
        );
        assert_eq!(extra, 0, "the redirect target must never be requested");
    }

    #[test]
    fn credentials_and_query_travel_as_before() {
        let (port, server) = stub(|stream| {
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        });
        let t = plain_http_transport();
        let response = t
            .get(
                &format!("http://127.0.0.1:{port}/accounts"),
                Some(("user", "pa55")),
                &[("version".to_owned(), "2".to_owned())],
            )
            .expect("ok");
        let (head, _) = server.join().unwrap();
        assert_eq!(response.body, "{}");
        assert!(head.starts_with("GET /accounts?version=2 "), "{head}");
        // base64("user:pa55")
        assert!(head.contains("dXNlcjpwYTU1"), "{head}");
    }

    #[test]
    fn the_agent_has_no_proxy_even_when_the_environment_names_one() {
        // ureq 3 reads proxy settings from the environment by default; ureq 2 did not. A
        // bank credential must never ride through an ambient proxy, so the agent has none.
        assert!(agent_config(true).proxy().is_none());
        assert!(agent_config(false).proxy().is_none());

        // Behavior: with every proxy variable pointing at a listener we control, a request
        // to the stub still goes straight to the stub. (The variables are process-wide,
        // but every transport in this crate's tests builds its agent with no proxy.)
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let proxy_url = format!("http://127.0.0.1:{}", proxy.local_addr().unwrap().port());
        let names = [
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
        ];
        let saved: Vec<_> = names.iter().map(std::env::var_os).collect();
        for name in names {
            std::env::set_var(name, &proxy_url);
        }
        let (port, server) = stub(|stream| {
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        });
        let t = plain_http_transport();
        let response = t.get(
            &format!("http://127.0.0.1:{port}/accounts"),
            Some(("user", "pa55")),
            &[],
        );
        // Put the environment back exactly as it was.
        for (name, before) in names.iter().zip(saved) {
            match before {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        let (head, _) = server.join().unwrap();
        assert_eq!(response.expect("direct request").body, "{}");
        assert!(
            head.starts_with("GET /accounts"),
            "the stub did not get the request: {head}"
        );
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            proxy.accept().is_err(),
            "a request went to the environment's proxy"
        );
    }

    #[test]
    fn a_small_gzip_reply_cannot_expand_past_the_cap() {
        // 11 KB of gzip that decodes to 11 MiB (over the 10 MiB cap). ureq 3 applies its own
        // body limit to the raw bytes and decodes above it, so with gzip support a reply like
        // this would decode in full. The transport neither asks for gzip nor decodes it.
        let bomb: &'static [u8] = include_bytes!("../tests/fixtures/gzip-bomb-11mib.gz");
        let (port, server) = stub(move |stream| {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bomb.len()
            );
            let _ = stream.write_all(bomb);
        });
        let t = plain_http_transport();
        let refused = t.get(
            &format!("http://127.0.0.1:{port}/accounts"),
            Some(("user", "pa55")),
            &[],
        );
        let (head, _) = server.join().unwrap();
        assert!(
            !head.to_ascii_lowercase().contains("accept-encoding"),
            "the request must not offer to accept a compressed body: {head}"
        );
        assert!(
            matches!(refused, Err(TransportError(ref m)) if m == "unreadable response body"),
            "{refused:?}"
        );
    }
}
