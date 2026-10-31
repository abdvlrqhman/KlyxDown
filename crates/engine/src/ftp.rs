//! FTP and FTPS: probing, ranged reads (`REST` then `RETR`) and folder listings.
//!
//! `ftp://` is plain, `ftpes://` asks for TLS on the normal port (explicit, `AUTH TLS`), and
//! `ftps://` speaks TLS from the first byte (implicit, port 990). Every connection is its own
//! control session, so segments download in parallel like HTTP ranges.

// shortcut: FTP ignores the proxy setting (suppaftp has no proxy support); tunnel through a
// SOCKS stream with `connect_with_stream` if users ask.

use std::sync::Arc;
use std::time::Duration;

use percent_encoding::percent_decode_str;
use reqwest::Url;
use serde::Serialize;
use suppaftp::list::ListParser;
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::tokio_rustls::rustls::{self, ClientConfig};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Status};

use crate::probe::Validators;
use crate::{Error, Probe, Result, names};

const TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) type Ftp = AsyncRustlsFtpStream;

pub fn is_ftp(url: &str) -> bool {
    ["ftp://", "ftps://", "ftpes://"].iter().any(|s| {
        url.get(..s.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(s))
    })
}

/// The server and file a link points at.
pub(crate) struct Target {
    host: String,
    port: u16,
    user: String,
    pass: String,
    /// Percent-decoded, starting with `/`.
    pub(crate) path: String,
    tls: Tls,
}

#[derive(PartialEq)]
enum Tls {
    None,
    Explicit,
    Implicit,
}

pub(crate) fn target(url: &Url) -> Result<Target> {
    let host = url
        .host_str()
        .ok_or_else(|| Error::InvalidUrl(url.to_string()))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let tls = match url.scheme() {
        "ftps" => Tls::Implicit,
        "ftpes" => Tls::Explicit,
        _ => Tls::None,
    };
    let decode = |s: &str| percent_decode_str(s).decode_utf8_lossy().into_owned();
    let user = decode(url.username());
    Ok(Target {
        port: url
            .port()
            .unwrap_or(if tls == Tls::Implicit { 990 } else { 21 }),
        pass: url.password().map(decode).unwrap_or_else(|| {
            if user.is_empty() {
                "anonymous@".into()
            } else {
                String::new()
            }
        }),
        user: if user.is_empty() {
            "anonymous".into()
        } else {
            user
        },
        path: decode(url.path()),
        host,
        tls,
    })
}

fn tls() -> Result<AsyncRustlsConnector> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|b| b.with_platform_verifier())
        .map_err(|e| Error::Network(format!("TLS setup failed: {e}")))?
        .with_no_client_auth();
    Ok(suppaftp::tokio_rustls::TlsConnector::from(Arc::new(config)).into())
}

/// Connects, signs in and switches to binary mode.
pub(crate) async fn connect(t: &Target) -> std::result::Result<Ftp, FtpError> {
    let addr = (t.host.as_str(), t.port);
    let session = async {
        let mut ftp: Ftp = if t.tls == Tls::Implicit {
            let connector = tls().map_err(|e| FtpError::SecureError(e.to_string()))?;
            Ftp::connect_secure_implicit(addr, connector, &t.host).await?
        } else {
            let ftp = Ftp::connect(addr).await?;
            if t.tls == Tls::Explicit {
                let connector = tls().map_err(|e| FtpError::SecureError(e.to_string()))?;
                ftp.into_secure(connector, &t.host).await?
            } else {
                ftp
            }
        };
        // Servers behind NAT often announce a private address for passive mode.
        ftp.set_passive_nat_workaround(true);
        ftp.login(&t.user, &t.pass).await?;
        ftp.transfer_type(FileType::Binary).await?;
        Ok(ftp)
    };
    tokio::time::timeout(TIMEOUT, session)
        .await
        .unwrap_or_else(|_| {
            Err(FtpError::ConnectionError(
                std::io::ErrorKind::TimedOut.into(),
            ))
        })
}

/// The status code of a refusal, if the server sent one.
pub(crate) fn code(e: &FtpError) -> Option<u32> {
    match e {
        FtpError::UnexpectedResponse(r) => Some(r.status.code()),
        _ => None,
    }
}

/// HTTP-like meaning for FTP refusals: 530 is "sign in", 550 is "no such file".
pub(crate) fn as_http(code: u32) -> u16 {
    match code {
        530 | 532 => 401,
        550 => 404,
        _ => 500,
    }
}

fn error(e: FtpError) -> Error {
    match code(&e) {
        Some(c @ 500..) => Error::Status(as_http(c)),
        _ => Error::Network(e.to_string()),
    }
}

pub(crate) async fn probe(url: &Url) -> Result<(Probe, Validators)> {
    let t = target(url)?;
    let mut ftp = connect(&t).await.map_err(error)?;
    let size = ftp.size(&t.path).await.map_err(error)? as u64;
    let modified = ftp.mdtm(&t.path).await.ok().map(|m| m.to_string());
    // Nearly every server resumes; the few that don't refuse REST, and the worker says so.
    let resumable = match ftp.feat().await {
        Ok(features) => features.keys().any(|k| k.eq_ignore_ascii_case("REST")),
        Err(_) => true,
    };
    let _ = ftp.quit().await;
    let name = t.path.rsplit('/').next().unwrap_or_default();
    Ok((
        Probe {
            url: url.to_string(),
            name: names::sanitize(name),
            host: t.host.clone(),
            size: Some(size),
            resumable,
            mime: None,
        },
        Validators {
            etag: None,
            last_modified: modified,
        },
    ))
}

/// Last-modified time, compared before resuming so a changed file is never mixed in.
pub(crate) async fn modified(ftp: &mut Ftp, path: &str) -> Option<String> {
    ftp.mdtm(path).await.ok().map(|m| m.to_string())
}

pub(crate) fn transient(e: &FtpError) -> bool {
    match code(e) {
        // 421 too many connections, 425/426 data connection trouble, 450/451 busy.
        Some(c) => (400..500).contains(&c),
        None => true,
    }
}

pub(crate) fn not_resumable(e: &FtpError) -> bool {
    matches!(e, FtpError::UnexpectedResponse(r) if r.status == Status::CommandNotImplemented || r.status.code() == 504)
}

/// A file or folder in a listing.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub url: String,
    pub dir: bool,
    pub size: Option<u64>,
}

/// Lists an FTP folder (MLSD, else LIST).
pub(crate) async fn list(url: &Url) -> Result<Vec<Entry>> {
    let t = target(url)?;
    let mut ftp = connect(&t).await.map_err(error)?;
    let lines = match ftp.mlsd(Some(&t.path)).await {
        Ok(lines) => lines,
        Err(_) => ftp.list(Some(&t.path)).await.map_err(error)?,
    };
    let _ = ftp.quit().await;
    let mut base = url.clone();
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    let mut entries: Vec<Entry> = lines
        .iter()
        .filter_map(|l| {
            ListParser::parse_mlsd(l)
                .or_else(|_| ListParser::parse_posix(l))
                .or_else(|_| ListParser::parse_dos(l))
                .ok()
        })
        .filter(|f| !matches!(f.name(), "." | ".."))
        .filter_map(|f| {
            let dir = f.is_directory();
            let mut url = base
                .join(&percent_encoding::utf8_percent_encode(f.name(), SEGMENT).to_string())
                .ok()?;
            if dir {
                url.set_path(&format!("{}/", url.path()));
            }
            Some(Entry {
                name: f.name().to_string(),
                url: url.to_string(),
                dir,
                size: (!dir).then_some(f.size() as u64),
            })
        })
        .collect();
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

/// Characters escaped in one path segment.
const SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'?')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'{')
    .add(b'}');

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_targets() {
        let t = target(&Url::parse("ftp://ftp.test/pub/a%20b.iso").unwrap()).unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.user.as_str()),
            ("ftp.test", 21, "anonymous")
        );
        assert_eq!(t.path, "/pub/a b.iso");
        let t = target(&Url::parse("ftps://me:p%40ss@ftp.test/x").unwrap()).unwrap();
        assert_eq!(
            (t.port, t.user.as_str(), t.pass.as_str()),
            (990, "me", "p@ss")
        );
        assert!(t.tls == Tls::Implicit);
        let t = target(&Url::parse("ftpes://ftp.test:2121/x").unwrap()).unwrap();
        assert!(t.tls == Tls::Explicit && t.port == 2121);
        assert!(is_ftp("FTP://x.test/a") && !is_ftp("https://x.test/ftp://"));
    }
}
