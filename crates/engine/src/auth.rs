//! Signing in to sites that ask (HTTP 401): Basic and Digest (RFC 7617, RFC 7616).
//!
//! The app supplies passwords per site (it keeps them in the OS keychain); the engine only
//! holds them in memory, per site, after a server asked. Nothing here is ever saved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use reqwest::Url;
use sha2::Digest as _;

/// Looks up the user name and password for a site (its host name).
pub type Credentials = Arc<dyn Fn(&str) -> Option<(String, String)> + Send + Sync>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Challenge {
    realm: String,
    nonce: String,
    opaque: Option<String>,
    /// "MD5", "MD5-sess", "SHA-256", "SHA-256-sess".
    algorithm: String,
    /// The server offered `qop=auth`.
    qop_auth: bool,
}

enum Known {
    Basic(String),
    Digest {
        user: String,
        pass: String,
        challenge: Challenge,
        /// Requests made with this nonce so far.
        count: u32,
    },
}

#[derive(Default)]
pub(crate) struct AuthCache {
    credentials: Option<Credentials>,
    known: Mutex<HashMap<String, Known>>,
}

impl AuthCache {
    pub(crate) fn new(credentials: Option<Credentials>) -> Self {
        Self {
            credentials,
            known: Mutex::default(),
        }
    }

    /// The saved sign-in for a site, if the app has one.
    pub(crate) fn lookup(&self, host: &str) -> Option<(String, String)> {
        self.credentials.as_ref().and_then(|c| c(host))
    }

    /// The `Authorization` header for a request, once the site has asked for one.
    pub(crate) fn header(&self, method: &str, url: &Url) -> Option<String> {
        let host = url.host_str()?;
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        match known.get_mut(host)? {
            Known::Basic(header) => Some(header.clone()),
            Known::Digest {
                user,
                pass,
                challenge,
                count,
            } => {
                *count += 1;
                Some(digest(
                    challenge,
                    user,
                    pass,
                    method,
                    url,
                    *count,
                    &cnonce(),
                ))
            }
        }
    }

    /// The server answered 401 with this `WWW-Authenticate`. Remembers how to sign in and
    /// returns true when retrying makes sense (there is a password for the site).
    pub(crate) fn learn(&self, url: &Url, www_authenticate: &str) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        let Some((user, pass)) = self.lookup(host) else {
            return false;
        };
        let entry = match parse(www_authenticate) {
            Some(challenge) => Known::Digest {
                user,
                pass,
                challenge,
                count: 0,
            },
            None if www_authenticate
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("basic") =>
            {
                Known::Basic(basic(&user, &pass))
            }
            None => return false,
        };
        self.known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(host.to_string(), entry);
        true
    }
}

fn basic(user: &str, pass: &str) -> String {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
    format!("Basic {token}")
}

/// The Digest challenge among `WWW-Authenticate` values (servers may offer several schemes).
pub(crate) fn parse(header: &str) -> Option<Challenge> {
    let start = header.to_ascii_lowercase().find("digest ")?;
    let mut c = Challenge {
        algorithm: "MD5".into(),
        ..Challenge::default()
    };
    let mut rest = &header[start + 7..];
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq]
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        let after = rest[eq + 1..].trim_start();
        let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            (&quoted[..end], &quoted[(end + 1).min(quoted.len())..])
        } else {
            let end = after.find(',').unwrap_or(after.len());
            (after[..end].trim(), &after[end..])
        };
        match key.as_str() {
            "realm" => c.realm = value.into(),
            "nonce" => c.nonce = value.into(),
            "opaque" => c.opaque = Some(value.into()),
            "algorithm" => c.algorithm = value.into(),
            "qop" => {
                c.qop_auth = value
                    .split(',')
                    .any(|q| q.trim().eq_ignore_ascii_case("auth"))
            }
            _ => {}
        }
        // Another scheme ("Basic realm=…") after this one ends the Digest parameters.
        rest = next;
        if rest
            .trim_start()
            .trim_start_matches(',')
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("basic ")
        {
            break;
        }
    }
    (!c.nonce.is_empty()).then_some(c)
}

fn hash(algorithm: &str, data: &str) -> String {
    let bytes: Vec<u8> = if algorithm.to_ascii_uppercase().starts_with("SHA-256") {
        sha2::Sha256::digest(data.as_bytes()).to_vec()
    } else {
        md5::Md5::digest(data.as_bytes()).to_vec()
    };
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn cnonce() -> String {
    let mut b = [0u8; 8];
    // A fresh client nonce per request; randomness only needs to be unpredictable enough.
    getrandom::fill(&mut b).unwrap_or_default();
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn digest(
    c: &Challenge,
    user: &str,
    pass: &str,
    method: &str,
    url: &Url,
    count: u32,
    cnonce: &str,
) -> String {
    let uri = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let alg = &c.algorithm;
    let mut ha1 = hash(alg, &format!("{user}:{}:{pass}", c.realm));
    if alg.to_ascii_lowercase().ends_with("-sess") {
        ha1 = hash(alg, &format!("{ha1}:{}:{cnonce}", c.nonce));
    }
    let ha2 = hash(alg, &format!("{method}:{uri}"));
    let nc = format!("{count:08x}");
    let response = if c.qop_auth {
        hash(alg, &format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", c.nonce))
    } else {
        hash(alg, &format!("{ha1}:{}:{ha2}", c.nonce))
    };
    let mut h = format!(
        r#"Digest username="{user}", realm="{}", nonce="{}", uri="{uri}", algorithm={alg}, response="{response}""#,
        c.realm, c.nonce
    );
    if c.qop_auth {
        h.push_str(&format!(r#", qop=auth, nc={nc}, cnonce="{cnonce}""#));
    }
    if let Some(o) = &c.opaque {
        h.push_str(&format!(r#", opaque="{o}""#));
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_2617_example() {
        let c = parse(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        )
        .unwrap();
        assert!(c.qop_auth);
        let url = Url::parse("http://host.com/dir/index.html").unwrap();
        let h = digest(&c, "Mufasa", "Circle Of Life", "GET", &url, 1, "0a4f113b");
        assert!(
            h.contains(r#"response="6629fae49393a05397450978507c4ef1""#),
            "{h}"
        );
        assert!(
            h.contains("nc=00000001") && h.contains(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#)
        );
    }

    #[test]
    fn rfc_7616_sha256_example() {
        let c = parse(
            r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=SHA-256, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#,
        )
        .unwrap();
        let url = Url::parse("http://www.example.org/dir/index.html").unwrap();
        let h = digest(
            &c,
            "Mufasa",
            "Circle of Life",
            "GET",
            &url,
            1,
            "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ",
        );
        assert!(
            h.contains(
                r#"response="753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1""#
            ),
            "{h}"
        );
    }

    #[test]
    fn learns_only_with_a_password() {
        let url = Url::parse("https://files.test/a.zip").unwrap();
        let none = AuthCache::new(None);
        assert!(!none.learn(&url, r#"Basic realm="x""#));
        let creds: Credentials =
            Arc::new(|host| (host == "files.test").then(|| ("me".into(), "pw".into())));
        let cache = AuthCache::new(Some(creds));
        assert!(cache.header("GET", &url).is_none());
        assert!(cache.learn(&url, r#"Basic realm="x""#));
        assert_eq!(cache.header("GET", &url).as_deref(), Some("Basic bWU6cHc="));
        let other = Url::parse("https://else.test/a.zip").unwrap();
        assert!(!cache.learn(&other, r#"Basic realm="x""#));
        assert!(parse(r#"Basic realm="only basic""#).is_none());
    }
}
