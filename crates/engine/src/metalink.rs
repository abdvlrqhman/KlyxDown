//! Metalink files (`.meta4`, RFC 5854, and the older `.metalink` 3.0): one or more files,
//! each with its mirrors in order of preference and a checksum.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::Serialize;

use crate::checksum::{Algo, Checksum};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MetalinkFile {
    /// The last path part only; the engine makes it safe for the disk.
    pub name: String,
    pub size: Option<u64>,
    /// Best first. Only http, https and ftp links.
    pub urls: Vec<String>,
    /// The strongest checksum the file lists.
    pub checksum: Option<Checksum>,
}

#[derive(Default)]
struct File {
    name: String,
    size: Option<u64>,
    /// (rank, url); lower ranks first.
    urls: Vec<(i64, String)>,
    hashes: Vec<Checksum>,
}

pub fn parse(xml: &str) -> Result<Vec<MetalinkFile>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut files = Vec::new();
    let mut file: Option<File> = None;
    // The element whose text comes next, with its rank or hash type.
    let mut open: Option<(String, i64, Option<Algo>)> = None;
    let mut is_metalink = false;
    let mut text = String::new();
    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("This is not a readable Metalink file: {e}")),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                text.clear();
                let name = local(&e);
                match name.as_str() {
                    "metalink" => is_metalink = true,
                    "file" => {
                        file = Some(File {
                            name: attr(&e, "name").unwrap_or_default(),
                            ..File::default()
                        });
                    }
                    "url" => {
                        // v4: priority 1 is best. v3: preference 100 is best.
                        let rank = attr(&e, "priority")
                            .and_then(|p| p.parse().ok())
                            .or_else(|| {
                                attr(&e, "preference")
                                    .and_then(|p| p.parse::<i64>().ok())
                                    .map(|p| 101 - p)
                            })
                            .unwrap_or(999_999);
                        open = Some((name, rank, None));
                    }
                    "hash" => {
                        let algo = attr(&e, "type").and_then(|t| Algo::from_name(&t));
                        open = Some((name, 0, algo));
                    }
                    "size" => open = Some((name, 0, None)),
                    _ => {}
                }
            }
            // quick-xml reports `&amp;` and `&#38;` apart from the text around them.
            Ok(Event::Text(t)) => text.push_str(&t.xml_content().unwrap_or_default()),
            Ok(Event::GeneralRef(r)) => match r.resolve_char_ref() {
                Ok(Some(c)) => text.push(c),
                _ => text.push_str(
                    r.decode()
                        .ok()
                        .and_then(|name| quick_xml::escape::resolve_predefined_entity(&name))
                        .unwrap_or_default(),
                ),
            },
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let value = std::mem::take(&mut text);
                let value = value.trim();
                if let (Some(f), Some((tag, rank, algo))) = (file.as_mut(), open.take())
                    && tag == name
                {
                    match tag.as_str() {
                        "size" => f.size = value.parse().ok(),
                        "url" if is_link(value) => f.urls.push((rank, value.to_string())),
                        "hash" => {
                            if let Some(c) =
                                algo.and_then(|a| Checksum::parse(value).filter(|c| c.algo == a))
                            {
                                f.hashes.push(c);
                            }
                        }
                        _ => {}
                    }
                }
                if name == "file"
                    && let Some(f) = file.take()
                {
                    files.push(finish(f));
                }
            }
            Ok(_) => {}
        }
    }
    if !is_metalink {
        return Err("This is not a Metalink file.".into());
    }
    files.retain(|f| !f.urls.is_empty());
    if files.is_empty() {
        return Err("The Metalink file lists no web or FTP links.".into());
    }
    Ok(files)
}

fn finish(mut f: File) -> MetalinkFile {
    f.urls.sort_by_key(|(rank, _)| *rank);
    let mut urls: Vec<String> = Vec::new();
    for (_, u) in f.urls {
        if !urls.contains(&u) {
            urls.push(u);
        }
    }
    let strength = |a: Algo| match a {
        Algo::Sha512 => 4,
        Algo::Sha256 => 3,
        Algo::Sha1 => 2,
        Algo::Md5 => 1,
    };
    MetalinkFile {
        name: f
            .name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_string(),
        size: f.size,
        urls,
        checksum: f.hashes.into_iter().max_by_key(|c| strength(c.algo)),
    }
}

fn local(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == key.as_bytes())
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

fn is_link(s: &str) -> bool {
    ["http://", "https://", "ftp://", "ftps://"]
        .iter()
        .any(|p| s.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_metalink_4() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <published>2026-10-01T00:00:00Z</published>
  <file name="dir/example.iso">
    <size>14471447</size>
    <hash type="sha-1">a97fcf6ba9358f8a6f62beee4421863d3e52b080</hash>
    <hash type="sha-256">f0ad929cd259957e160ea442eb80986b5f01d1b3a2b9b2ed3e6ba15dd6c3ba3a</hash>
    <url location="de" priority="2">https://mirror2.test/example.iso</url>
    <url location="us" priority="1">https://mirror1.test/example.iso?a=1&amp;b=2</url>
    <url priority="3">gopher://old.test/example.iso</url>
    <metaurl mediatype="torrent" priority="1">https://x.test/example.torrent</metaurl>
  </file>
</metalink>"#;
        let files = parse(xml).unwrap();
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.name, "example.iso");
        assert_eq!(f.size, Some(14_471_447));
        assert_eq!(
            f.urls,
            [
                "https://mirror1.test/example.iso?a=1&b=2",
                "https://mirror2.test/example.iso"
            ]
        );
        assert_eq!(f.checksum.as_ref().unwrap().algo, Algo::Sha256);
    }

    #[test]
    fn reads_metalink_3() {
        let xml = r#"<?xml version="1.0"?>
<metalink version="3.0" xmlns="http://www.metalinker.org/">
  <files>
    <file name="a.zip">
      <size>10</size>
      <verification><hash type="md5">900150983cd24fb0d6963f7d28e17f72</hash></verification>
      <resources>
        <url type="ftp" preference="50">ftp://ftp.test/a.zip</url>
        <url type="http" preference="100">http://www.test/a.zip</url>
      </resources>
    </file>
    <file name="nothing.bin"><resources></resources></file>
  </files>
</metalink>"#;
        let files = parse(xml).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].urls,
            ["http://www.test/a.zip", "ftp://ftp.test/a.zip"]
        );
        assert_eq!(files[0].checksum.as_ref().unwrap().algo, Algo::Md5);
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse("<html><body>hi</body></html>").is_err());
        assert!(parse("not xml at all <<<").is_err());
    }
}
