//! Reading web pages for links: the site explorer (a folder listing, like a file browser)
//! and the site grabber (follow a site's pages and collect links to files of some types).

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Duration;

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

use crate::ftp::Entry;
use crate::probe::describe;
use crate::{Error, Result};

const MAX_PAGE: u64 = 4 * 1024 * 1024;

/// What the grabber looks for, and how far.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabOptions {
    /// File extensions to collect, without dots ("jpg", "zip"). Empty collects every file.
    pub extensions: Vec<String>,
    /// How many links away from the first page to read (0 reads only that page).
    pub depth: u32,
    /// Only read pages on the first page's site.
    pub same_site: bool,
    /// Stop after reading this many pages.
    pub max_pages: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabProgress {
    pub pages: usize,
    pub found: usize,
}

/// Every link in a page: `href`, `src` and `srcset` attributes, made absolute (honoring
/// `<base href>`), without `#fragments`, in page order, once each.
pub fn links(html: &str, page: &Url) -> Vec<Url> {
    let mut base = page.clone();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(tag) = lower[at..].find('<').map(|i| at + i) {
        let end = lower[tag..].find('>').map_or(lower.len(), |i| tag + i);
        let inside = &html[tag + 1..end];
        let name = inside
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if name == "script" || name == "style" {
            // Skip code; its strings are not links on the page.
            let close = format!("</{name}");
            at = lower[end..].find(&close).map_or(lower.len(), |i| end + i);
            continue;
        }
        for (key, value) in attributes(inside) {
            let values: Vec<&str> = match key.as_str() {
                "href" | "src" | "data-src" => vec![value.as_str()],
                // "a.jpg 1x, b.jpg 2x"
                "srcset" => value
                    .split(',')
                    .filter_map(|c| c.split_whitespace().next())
                    .collect(),
                _ => continue,
            };
            for v in values {
                let Ok(mut url) = base.join(&decode_entities(v.trim())) else {
                    continue;
                };
                if name == "base" && key == "href" {
                    base = url;
                    break;
                }
                url.set_fragment(None);
                if matches!(url.scheme(), "http" | "https" | "ftp") && seen.insert(url.to_string())
                {
                    out.push(url);
                }
            }
        }
        at = end;
    }
    out
}

/// `key="value"`, `key='value'` and `key=value` pairs of a tag.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        let key = tag[start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let (from, to) = match bytes.get(i) {
            Some(q @ (b'"' | b'\'')) => {
                let close = tag[i + 1..]
                    .find(*q as char)
                    .map_or(tag.len(), |j| i + 1 + j);
                (i + 1, close)
            }
            _ => {
                let close = tag[i..]
                    .find(|c: char| c.is_whitespace())
                    .map_or(tag.len(), |j| i + j);
                (i, close)
            }
        };
        out.push((key, tag[from..to].to_string()));
        i = to + 1;
    }
    out
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

async fn page(client: &Client, url: &Url) -> Result<Option<String>> {
    let mut resp = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| Error::Network(describe(&e)))?;
    if !resp.status().is_success() {
        return Err(Error::Status(resp.status().as_u16()));
    }
    let html = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("html"));
    if !html {
        return Ok(None);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Network(describe(&e)))?
    {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > MAX_PAGE {
            break;
        }
    }
    Ok(Some(String::from_utf8_lossy(&body).into_owned()))
}

/// The last path part, decoded: the name a listing shows.
fn last_part(url: &Url) -> String {
    let path = url.path().trim_end_matches('/');
    let part = path.rsplit('/').next().unwrap_or_default();
    percent_encoding::percent_decode_str(part)
        .decode_utf8_lossy()
        .into_owned()
}

/// A web server's folder listing (Apache, nginx, IIS style index pages): the files and
/// folders directly inside `folder`, folders first.
pub(crate) async fn explore(client: &Client, folder: &Url) -> Result<Vec<Entry>> {
    let mut folder = folder.clone();
    if !folder.path().ends_with('/') {
        folder.set_path(&format!("{}/", folder.path()));
    }
    let html = page(client, &folder)
        .await?
        .ok_or_else(|| Error::Network("That is a file, not a folder.".into()))?;
    let mut entries: Vec<Entry> = links(&html, &folder)
        .into_iter()
        .filter(|u| u.host_str() == folder.host_str() && u.query().is_none())
        .filter_map(|u| {
            // Only what sits directly in this folder (not the parent, not deeper).
            let rest = u.path().strip_prefix(folder.path())?;
            let rest = rest.trim_end_matches('/');
            (!rest.is_empty() && !rest.contains('/')).then(|| Entry {
                name: last_part(&u),
                dir: u.path().ends_with('/'),
                size: None,
                url: u.to_string(),
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries.dedup_by(|a, b| a.url == b.url);
    Ok(entries)
}

/// Pages are links without a file extension, or with a page one.
fn looks_like_page(url: &Url) -> bool {
    let name = last_part(url).to_ascii_lowercase();
    match name.rsplit_once('.') {
        None => true,
        Some((_, ext)) => matches!(
            ext,
            "html" | "htm" | "php" | "asp" | "aspx" | "jsp" | "shtml" | "cgi"
        ),
    }
}

fn extension(url: &Url) -> Option<String> {
    let name = last_part(url);
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())
}

/// The `Disallow` rules for every robot (`User-agent: *`) in a site's robots.txt.
pub(crate) fn disallowed(robots: &str) -> Vec<String> {
    let mut rules = Vec::new();
    let mut ours = false;
    for line in robots
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim())
    {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "user-agent" => ours = value.trim() == "*",
            "disallow" if ours && !value.trim().is_empty() => rules.push(value.trim().to_string()),
            _ => {}
        }
    }
    rules
}

/// Follows a site's pages breadth first and collects links to files with the wanted
/// extensions. Reads one page at a time with a short pause (sites are shared by everyone),
/// honors robots.txt, and stops early when `stop` is set.
pub(crate) async fn grab(
    client: &Client,
    start: &Url,
    opts: &GrabOptions,
    stop: &AtomicBool,
    progress: &(dyn Fn(GrabProgress) + Send + Sync),
) -> Result<Vec<String>> {
    let wanted: HashSet<String> = opts
        .extensions
        .iter()
        .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
        .collect();
    let rules = match start.join("/robots.txt") {
        Ok(u) => match client.get(u).timeout(Duration::from_secs(10)).send().await {
            Ok(r) if r.status().is_success() => disallowed(&r.text().await.unwrap_or_default()),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    let allowed = |u: &Url| {
        u.host_str() != start.host_str() || !rules.iter().any(|r| u.path().starts_with(r.as_str()))
    };

    let mut queue = VecDeque::from([(start.clone(), 0u32)]);
    let mut seen = HashSet::from([start.to_string()]);
    let mut found: Vec<String> = Vec::new();
    let mut found_set = HashSet::new();
    let mut state = GrabProgress::default();
    while let Some((url, depth)) = queue.pop_front() {
        if stop.load(Relaxed) || state.pages >= opts.max_pages {
            break;
        }
        let html = match page(client, &url).await {
            Ok(Some(html)) => html,
            Ok(None) => continue,
            Err(e) if state.pages == 0 => return Err(e),
            Err(e) => {
                tracing::debug!(%url, "grabber skipped a page: {e}");
                continue;
            }
        };
        state.pages += 1;
        for link in links(&html, &url) {
            let ext = extension(&link);
            let is_page = looks_like_page(&link);
            let file_wanted =
                !is_page && (wanted.is_empty() || ext.as_ref().is_some_and(|e| wanted.contains(e)));
            if file_wanted {
                if found_set.insert(link.to_string()) {
                    found.push(link.to_string());
                }
            } else if is_page
                && depth < opts.depth
                && (!opts.same_site || link.host_str() == start.host_str())
                && allowed(&link)
                && seen.insert(link.to_string())
            {
                queue.push_back((link, depth + 1));
            }
        }
        state.found = found.len();
        progress(state.clone());
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_in_pages() {
        let page = Url::parse("https://site.test/gallery/index.html").unwrap();
        let html = r#"<html><head><style>a{background:url(x.png)}</style>
            <script>var s = "<a href='/not-a-link.zip'>";</script></head><body>
            <a href="photo%20one.jpg">1</a> <a href='../up.zip#part'>2</a>
            <A HREF=/abs.pdf>3</A> <img src="//cdn.test/i.png" srcset="s.jpg 1x, l.jpg 2x">
            <a href="mailto:x@y.test">m</a> <a href="?page=2&amp;sort=1">next</a>
            <a href="photo%20one.jpg">dup</a></body></html>"#;
        let got: Vec<String> = links(html, &page).iter().map(Url::to_string).collect();
        assert_eq!(
            got,
            [
                "https://site.test/gallery/photo%20one.jpg",
                "https://site.test/up.zip",
                "https://site.test/abs.pdf",
                "https://cdn.test/i.png",
                "https://site.test/gallery/s.jpg",
                "https://site.test/gallery/l.jpg",
                "https://site.test/gallery/index.html?page=2&sort=1",
            ]
        );
    }

    #[test]
    fn base_href_moves_relative_links() {
        let page = Url::parse("https://site.test/a/b.html").unwrap();
        let got = links(
            r#"<base href="https://files.test/dl/"><a href="x.zip">x</a>"#,
            &page,
        );
        assert_eq!(got[0].as_str(), "https://files.test/dl/x.zip");
    }

    #[test]
    fn robots_rules_for_everyone() {
        let robots = "User-agent: Googlebot\nDisallow: /only-google\n\nUser-agent: *\nDisallow: /private # no\nDisallow:\nAllow: /x\n";
        assert_eq!(disallowed(robots), ["/private"]);
    }

    #[test]
    fn pages_and_files() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(looks_like_page(&u("https://s.test/about")));
        assert!(looks_like_page(&u("https://s.test/a/index.php?x=1")));
        assert!(!looks_like_page(&u("https://s.test/a/pic.JPG")));
        assert_eq!(
            extension(&u("https://s.test/a/pic.JPG")).as_deref(),
            Some("jpg")
        );
    }
}
