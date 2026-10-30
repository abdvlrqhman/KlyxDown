//! File names: from server headers or the URL, made safe for Windows.

use percent_encoding::percent_decode_str;
use reqwest::Url;

/// Picks the name a browser would: `Content-Disposition`, else the last URL path segment.
pub(crate) fn pick(disposition: Option<&str>, url: &Url) -> String {
    let raw = disposition
        .and_then(from_disposition)
        .or_else(|| {
            let last = url.path_segments()?.next_back()?;
            Some(percent_decode_str(last).decode_utf8_lossy().into_owned())
        })
        .unwrap_or_default();
    sanitize(&raw)
}

/// `filename*=UTF-8''...` wins over `filename="..."` (RFC 6266).
fn from_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    for param in split_params(value) {
        let Some((key, val)) = param.split_once('=') else {
            continue;
        };
        let val = val.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                // charset'language'percent-encoded
                let mut parts = val.trim_matches('"').splitn(3, '\'');
                if let (Some(_), Some(_), Some(encoded)) =
                    (parts.next(), parts.next(), parts.next())
                {
                    return Some(percent_decode_str(encoded).decode_utf8_lossy().into_owned());
                }
            }
            "filename" => {
                let unquoted = val
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(val);
                plain = Some(unquoted.replace("\\\"", "\""));
            }
            _ => {}
        }
    }
    plain
}

/// Splits on `;` outside double quotes.
fn split_params(value: &str) -> Vec<&str> {
    let (mut out, mut quoted, mut from) = (Vec::new(), false, 0);
    for (i, c) in value.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(&value[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(&value[from..]);
    out
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
const MAX_CHARS: usize = 180;

pub(crate) fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control()
        })
        .collect();
    let mut name = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if name.is_empty() {
        return "download".into();
    }
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        name.insert(0, '_');
    }
    if name.chars().count() > MAX_CHARS {
        let ext = numbered_parts(&name).1.to_string();
        let keep = MAX_CHARS.saturating_sub(ext.chars().count());
        name = name.chars().take(keep).collect::<String>() + &ext;
    }
    name
}

/// `name.ext` -> `name (n).ext`; `n == 0` returns the name unchanged.
pub(crate) fn numbered(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_string();
    }
    let (stem, ext) = numbered_parts(name);
    format!("{stem} ({n}){ext}")
}

/// Splits at the last dot, keeping the dot with the extension. Dotfiles have no extension.
fn numbered_parts(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn disposition() {
        let u = url("https://x.test/dl?id=1");
        assert_eq!(
            pick(Some(r#"attachment; filename="sample file.bin""#), &u),
            "sample file.bin"
        );
        assert_eq!(
            pick(Some("attachment; filename=plain.zip"), &u),
            "plain.zip"
        );
        assert_eq!(
            pick(
                Some(
                    r#"attachment; filename="fallback.txt"; filename*=UTF-8''r%C3%A9sum%C3%A9%20v2.pdf"#
                ),
                &u
            ),
            "résumé v2.pdf"
        );
        assert_eq!(
            pick(Some(r#"attachment; filename="a;b.txt""#), &u),
            "a;b.txt"
        );
    }

    #[test]
    fn from_url() {
        assert_eq!(
            pick(None, &url("https://x.test/files/my%20song.mp3?x=1")),
            "my song.mp3"
        );
        assert_eq!(pick(None, &url("https://x.test/")), "download");
        assert_eq!(pick(Some("inline"), &url("https://x.test/a.iso")), "a.iso");
    }

    #[test]
    fn windows_safe() {
        assert_eq!(sanitize(r#"a<b>c:d"e/f\g|h?i*j.txt"#), "abcdefghij.txt");
        assert_eq!(sanitize("name. . "), "name");
        assert_eq!(sanitize("tab\there.txt"), "tabhere.txt");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("nul.txt"), "_nul.txt");
        assert_eq!(sanitize("console.txt"), "console.txt");
        assert_eq!(sanitize("..."), "download");
        let long = sanitize(&format!("{}.mkv", "x".repeat(300)));
        assert_eq!(long.chars().count(), MAX_CHARS);
        assert!(long.ends_with(".mkv"));
    }

    #[test]
    fn numbering() {
        assert_eq!(numbered("sample file.bin", 1), "sample file (1).bin");
        assert_eq!(numbered("README", 2), "README (2)");
        assert_eq!(numbered(".bashrc", 1), ".bashrc (1)");
        assert_eq!(numbered("a.bin", 0), "a.bin");
    }
}
