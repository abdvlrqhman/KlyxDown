//! Asking the server about a link before downloading it.

use reqwest::header::{
    CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, HeaderMap, HeaderName,
    LAST_MODIFIED, RANGE,
};
use reqwest::{Client, RequestBuilder, StatusCode, Url};

use crate::{Error, Probe, Result, names};

pub(crate) fn parse_url(url: &str) -> Result<Url> {
    match Url::parse(url.trim()) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => Ok(u),
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
    url: &str,
    headers: &[(String, String)],
) -> Result<(Probe, Validators)> {
    let url = parse_url(url)?;
    let resp = with_headers(client.get(url), headers)
        .header(RANGE, "bytes=0-0")
        .send()
        .await
        .map_err(|e| Error::Network(describe(&e)))?;
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
    let mut cause: &dyn std::error::Error = e;
    while let Some(next) = cause.source() {
        cause = next;
    }
    cause.to_string()
}
