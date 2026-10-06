//! [`Transport`] over HTTPS, with the same stack and proxy resolution the
//! installer and the update check use.
//!
//! Blocking — every call runs on a worker, never on the UI thread.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use ureq::http::Method;

use super::api::{ApiError, Reply, Transport, classify, link_has_next};
use super::token::Token;

const API: &str = "https://api.github.com";

/// Interactive reads: long enough for a slow link, short enough that a dead
/// one is reported rather than sat on.
const TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A pull request's file list with every patch inlined is the largest answer
/// the panel asks for; GitHub itself caps a patch well under this.
const MAX_BODY: u64 = 32 * 1024 * 1024;

/// Past this many remembered answers the cache starts over.
const ETAG_CAP: usize = 512;

pub struct HttpTransport {
    agent: ureq::Agent,
    token: Option<Token>,
    etags: Etags,
}

/// The last answer to each plain GET, by ETag. Re-asking with `If-None-Match`
/// costs nothing when it comes back 304: a signed-in 304 is not counted
/// against the rate limit, which is what keeps the panel's revalidations and
/// checks poll from eating the hourly allowance it shares with `gh`.
#[derive(Default)]
struct Etags(Mutex<HashMap<String, (String, Reply)>>);

impl Etags {
    fn tag(&self, path: &str) -> Option<String> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(path).map(|(tag, _)| tag.clone())
    }

    fn cached(&self, path: &str) -> Option<Reply> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(path).map(|(_, reply)| reply.clone())
    }

    fn store(&self, path: &str, tag: String, reply: &Reply) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // ponytail: wholesale clear, an LRU if a session ever cycles past the cap
        if map.len() >= ETAG_CAP && !map.contains_key(path) {
            map.clear();
        }
        map.insert(path.to_string(), (tag, reply.clone()));
    }
}

impl HttpTransport {
    /// `manual_proxy` is the `http_proxy` setting, as for tty7's other
    /// downloads.
    pub fn new(token: Option<Token>, manual_proxy: Option<&str>) -> HttpTransport {
        let mut builder = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            // 4xx/5xx come back as responses, so their headers (the rate
            // limit's reset time) can be read.
            .http_status_as_error(false)
            .user_agent(concat!("tty7/", env!("CARGO_PKG_VERSION")));
        if let Some(proxy) = crate::daemon::install::proxy::resolve(API, manual_proxy) {
            builder = builder.proxy(Some(proxy));
        }
        HttpTransport {
            agent: builder.build().into(),
            token,
            etags: Etags::default(),
        }
    }
}

impl Transport for HttpTransport {
    fn get(&self, path: &str) -> Result<Reply, ApiError> {
        self.request(path, "application/vnd.github+json", None)
    }

    fn get_full(&self, path: &str) -> Result<Reply, ApiError> {
        self.request(path, "application/vnd.github.full+json", None)
    }

    fn post(&self, path: &str, body: &[u8]) -> Result<Reply, ApiError> {
        self.request(
            path,
            "application/vnd.github+json",
            Some((Method::POST, body)),
        )
    }

    fn put(&self, path: &str, body: &[u8]) -> Result<Reply, ApiError> {
        self.request(
            path,
            "application/vnd.github+json",
            Some((Method::PUT, body)),
        )
    }

    fn authenticated(&self) -> bool {
        self.token.is_some()
    }
}

impl HttpTransport {
    /// A GET, or a POST or PUT of `body` as JSON.
    fn request(
        &self,
        path: &str,
        accept: &str,
        body: Option<(Method, &[u8])>,
    ) -> Result<Reply, ApiError> {
        let url = format!("{API}{path}");
        // Only a plain GET is revalidated: `get_full` carries signed
        // attachment URLs that expire, so its old body must not be replayed.
        let cacheable = body.is_none() && accept == "application/vnd.github+json";
        let tag = cacheable.then(|| self.etags.tag(path)).flatten();
        let response = match body {
            None => {
                let request = self.headers(self.agent.get(url), accept);
                match &tag {
                    Some(tag) => request.header("If-None-Match", tag).call(),
                    None => request.call(),
                }
            }
            Some((method, body)) => {
                let request = if method == Method::PUT {
                    self.agent.put(url)
                } else {
                    self.agent.post(url)
                };
                self.headers(request, accept)
                    .header("Content-Type", "application/json")
                    .send(body)
            }
        };
        // ureq's error text names the URL and the transport failure, never
        // the request headers, so it is safe to show.
        let mut response = response.map_err(|e| ApiError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        if status == 304
            && tag.is_some()
            && let Some(reply) = self.etags.cached(path)
        {
            return Ok(reply);
        }
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(|e| ApiError::Network(e.to_string()))?;
        classify(status, &header, &body)?;
        let reply = Reply {
            has_next: header("link").is_some_and(|l| link_has_next(&l)),
            body,
        };
        if cacheable && let Some(tag) = header("etag") {
            self.etags.store(path, tag, &reply);
        }
        Ok(reply)
    }

    fn headers<B>(
        &self,
        request: ureq::RequestBuilder<B>,
        accept: &str,
    ) -> ureq::RequestBuilder<B> {
        let request = request
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28");
        match &self.token {
            Some(token) => request.header("Authorization", format!("Bearer {}", token.expose())),
            None => request,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(body: &str) -> Reply {
        Reply {
            body: body.as_bytes().to_vec(),
            has_next: true,
        }
    }

    #[test]
    fn a_stored_answer_is_replayed_by_its_tag() {
        let etags = Etags::default();
        assert_eq!(etags.tag("/a"), None);
        etags.store("/a", "W/\"1\"".into(), &reply("one"));
        etags.store("/a", "W/\"2\"".into(), &reply("two"));
        assert_eq!(etags.tag("/a").as_deref(), Some("W/\"2\""));
        let back = etags.cached("/a").unwrap();
        assert_eq!((back.body.as_slice(), back.has_next), (&b"two"[..], true));
    }

    #[test]
    fn the_cache_starts_over_past_its_cap() {
        let etags = Etags::default();
        for i in 0..ETAG_CAP {
            etags.store(&format!("/{i}"), "t".into(), &reply(""));
        }
        etags.store("/0", "t2".into(), &reply(""));
        assert!(etags.tag("/1").is_some());
        etags.store("/new", "t".into(), &reply(""));
        assert_eq!(
            (etags.tag("/1"), etags.tag("/new").as_deref()),
            (None, Some("t"))
        );
    }
}
