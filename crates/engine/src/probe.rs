//! Asking the server about a link before downloading it.

use reqwest::header::{
    CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, HeaderMap, HeaderName,
    LAST_MODIFIED, RANGE,
};
use reqwest::{Client, RequestBuilder, StatusCode, Url};

use crate::auth::AuthCache;
use crate::{Error, Probe, Result, names};

pub(crate) fn parse_url(url: &str) -> Result<Url> {
    match Url::parse(url.trim()) {
        Ok(u)
            if matches!(u.scheme(), "http" | "https" | "ftp" | "ftps" | "ftpes")
                && u.host_str().is_some() =>
        {
            Ok(u)
        }
        _ => Err(Error::InvalidUrl(url.to_string())),
    }
}

/// `Accept-Encoding` would get a compressed body written to disk against the uncompressed size.
const OWNED: &[&str] = &["range", "accept-encoding", "content-length", "host"];

/// Adds the browser-provided headers, minus the ones the engine owns.
pub(crate) fn with_headers(
    mut req: RequestBuilder,
    headers: &[(String, String)],
) -> RequestBuilder {
    for (k, v) in headers
        .iter()
        .filter(|(k, _)| !OWNED.iter().any(|o| k.eq_ignore_ascii_case(o)))
    {
        req = req.header(k.as_str(), v.as_str());
    }
    req
}

/// What a resumed range request must still match, sent as `If-Range`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Validators {
    pub(crate) etag: Option<String>,
    pub(crate) last_modified: Option<String>,
}

impl Validators {
    /// A strong ETag, else Last-Modified. Weak ETags are not allowed in `If-Range`.
    pub(crate) fn if_range(&self) -> Option<&str> {
        self.etag
            .as_deref()
            .filter(|e| !e.starts_with("W/"))
            .or(self.last_modified.as_deref())
    }
}

/// `GET` with `Range: bytes=0-0`, because servers lie on `HEAD`. Never reads the body.
pub(crate) async fn probe(
    client: &Client,
    auth: &AuthCache,
    url: &str,
    headers: &[(String, String)],
) -> Result<(Probe, Validators)> {
    let url = parse_url(url)?;
    if crate::ftp::is_ftp(url.as_str()) {
        return crate::ftp::probe(&url, auth).await;
    }
    let send = || {
        let mut req = with_headers(client.get(url.clone()), headers).header(RANGE, "bytes=0-0");
        if let Some(h) = auth.header("GET", &url) {
            req = req.header(reqwest::header::AUTHORIZATION, h);
        }
        req.send()
    };
    let mut resp = send().await.map_err(|e| Error::Network(describe(&e)))?;
    // The site asks for a sign-in: try once with the saved one.
    if resp.status() == StatusCode::UNAUTHORIZED
        && let Some(challenge) = www_authenticate(resp.headers())
        && auth.learn(&url, &challenge)
    {
        resp = send().await.map_err(|e| Error::Network(describe(&e)))?;
    }
    let status = resp.status();
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    let h = resp.headers();
    let resumable = status == StatusCode::PARTIAL_CONTENT;
    let size = if resumable {
        content_range(h).and_then(|r| r.2)
    } else {
        header_u64(h, CONTENT_LENGTH)
    };
    let disposition = h
        .get(CONTENT_DISPOSITION)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let mime = h
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let text = |name: HeaderName| h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let validators = Validators {
        etag: text(ETAG),
        last_modified: text(LAST_MODIFIED),
    };
    let final_url = resp.url().clone();
    // Dropping the response here closes the connection instead of reading a body the server
    // may be sending in full despite the Range header.
    drop(resp);
    let probe = Probe {
        name: names::pick(disposition.as_deref(), &final_url),
        host: final_url.host_str().unwrap_or_default().to_string(),
        url: final_url.into(),
        size,
        resumable,
        mime,
    };
    Ok((probe, validators))
}

/// Mirrors that serve the same file: same size, byte ranges, after redirects. Probed in
/// parallel, without the primary link's cookies (they belong to another site).
pub(crate) async fn matching_mirrors(
    client: &Client,
    primary: &Probe,
    mirrors: Vec<String>,
) -> Vec<String> {
    if mirrors.is_empty() || !primary.resumable || primary.size.is_none() {
        return Vec::new();
    }
    let mut checks = tokio::task::JoinSet::new();
    // Other sites: the primary's sign-in is not theirs to see.
    let none = std::sync::Arc::new(AuthCache::default());
    for (i, url) in mirrors.into_iter().enumerate().take(16) {
        let client = client.clone();
        let none = none.clone();
        checks.spawn(async move { (i, probe(&client, &none, &url, &[]).await) });
    }
    let mut ok = Vec::new();
    while let Some(Ok((i, result))) = checks.join_next().await {
        match result {
            Ok((p, _)) if p.resumable && p.size == primary.size && p.url != primary.url => {
                ok.push((i, p.url));
            }
            Ok((p, _)) => {
                tracing::info!(url = %p.url, "mirror skipped: different file or no ranges")
            }
            Err(e) => tracing::info!("mirror skipped: {e}"),
        }
    }
    ok.sort();
    ok.dedup_by(|a, b| a.1 == b.1);
    ok.into_iter().map(|(_, url)| url).collect()
}

/// All `WWW-Authenticate` values, joined (a server can offer several schemes).
pub(crate) fn www_authenticate(h: &HeaderMap) -> Option<String> {
    let all: Vec<&str> = h
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    (!all.is_empty()).then(|| all.join(", "))
}

/// `Content-Range: bytes first-last/total` as (first, last, total); total is `None` for `*`.
pub(crate) fn content_range(h: &HeaderMap) -> Option<(u64, u64, Option<u64>)> {
    let v = h
        .get(CONTENT_RANGE)?
        .to_str()
        .ok()?
        .trim()
        .strip_prefix("bytes ")?;
    let (range, total) = v.split_once('/')?;
    let (first, last) = range.split_once('-')?;
    Some((
        first.trim().parse().ok()?,
        last.trim().parse().ok()?,
        total.trim().parse().ok(),
    ))
}

pub(crate) fn header_u64(h: &HeaderMap, name: HeaderName) -> Option<u64> {
    h.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// The innermost cause, which is the human-readable part ("No such host is known", "timed out").
pub(crate) fn describe(e: &reqwest::Error) -> String {
    // The innermost cause of a timeout is tokio's bare "deadline has elapsed".
    if e.is_timeout() {
        return "it did not answer in time".into();
    }
    let mut cause: &dyn std::error::Error = e;
    while let Some(next) = cause.source() {
        cause = next;
    }
    cause.to_string()
}
