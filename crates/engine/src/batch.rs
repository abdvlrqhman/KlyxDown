//! Batch links: `https://x.test/photo[001-120].jpg` stands for 120 links.

/// Most links one pattern may stand for.
pub const MAX_BATCH: usize = 10_000;

/// Expands every `[from-to]` (or `[from-to:step]`) group in a link: numbers keep the
/// width of `from` when it has leading zeros (`[001-120]`), letters go `[a-z]` or `[A-Z]`.
/// Several groups combine, the leftmost changing slowest. `None` when the link has no
/// group (or a group is malformed, which also covers IPv6 hosts like `[::1]`).
pub fn expand(pattern: &str) -> Option<Result<Vec<String>, String>> {
    let mut parts: Vec<Vec<String>> = Vec::new();
    let mut rest = pattern;
    let mut found = false;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|c| open + c) else {
            break;
        };
        match range(&rest[open + 1..close]) {
            Some(Ok(values)) => {
                parts.push(vec![rest[..open].to_string()]);
                parts.push(values);
                found = true;
            }
            Some(Err(e)) => return Some(Err(e)),
            None => parts.push(vec![rest[..=close].to_string()]),
        }
        rest = &rest[close + 1..];
    }
    if !found {
        return None;
    }
    parts.push(vec![rest.to_string()]);
    let total = parts
        .iter()
        .try_fold(1usize, |n, p| n.checked_mul(p.len()))
        .filter(|&n| n <= MAX_BATCH);
    if total.is_none() {
        return Some(Err(format!(
            "That pattern stands for more than {MAX_BATCH} links."
        )));
    }
    let mut out = vec![String::new()];
    for part in parts {
        out = out
            .iter()
            .flat_map(|prefix| part.iter().map(move |p| format!("{prefix}{p}")))
            .collect();
    }
    Some(Ok(out))
}

/// `001-120`, `1-20:2`, `a-z`. `None` if it isn't a range at all.
fn range(spec: &str) -> Option<Result<Vec<String>, String>> {
    let (span, step) = match spec.split_once(':') {
        Some((span, step)) => (span, step.parse::<u64>().ok().filter(|&s| s > 0)?),
        None => (spec, 1),
    };
    let (from, to) = span.split_once('-')?;
    let letters = |s: &str| s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic());
    if letters(from) && letters(to) {
        let (a, b) = (from.as_bytes()[0], to.as_bytes()[0]);
        if a.is_ascii_lowercase() != b.is_ascii_lowercase() || a > b {
            return Some(Err(format!("[{spec}] counts backwards or mixes cases.")));
        }
        return Some(Ok((a..=b)
            .step_by(step as usize)
            .map(|c| char::from(c).to_string())
            .collect()));
    }
    if from.is_empty() || !from.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (a, b) = (from.parse::<u64>().ok()?, to.parse::<u64>().ok()?);
    if a > b {
        return Some(Err(format!("[{spec}] counts backwards.")));
    }
    if (b - a) / step >= MAX_BATCH as u64 {
        return Some(Err(format!(
            "That pattern stands for more than {MAX_BATCH} links."
        )));
    }
    let width = if from.len() > 1 && from.starts_with('0') {
        from.len()
    } else {
        0
    };
    Some(Ok((a..=b)
        .step_by(step as usize)
        .map(|n| format!("{n:0width$}"))
        .collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(p: &str) -> Vec<String> {
        expand(p).unwrap().unwrap()
    }

    #[test]
    fn numbers_keep_their_padding() {
        let v = ok("https://x.test/photo[001-120].jpg");
        assert_eq!(v.len(), 120);
        assert_eq!(v[0], "https://x.test/photo001.jpg");
        assert_eq!(v[119], "https://x.test/photo120.jpg");
        assert_eq!(
            ok("https://x.test/[8-10].png"),
            [
                "https://x.test/8.png",
                "https://x.test/9.png",
                "https://x.test/10.png"
            ]
        );
    }

    #[test]
    fn letters_steps_and_several_groups() {
        assert_eq!(
            ok("https://x.test/[a-c]"),
            ["https://x.test/a", "https://x.test/b", "https://x.test/c"]
        );
        assert_eq!(
            ok("https://x.test/[0-10:5]"),
            ["https://x.test/0", "https://x.test/5", "https://x.test/10"]
        );
        let v = ok("https://x.test/s[1-2]/e[01-03].mkv");
        assert_eq!(v.len(), 6);
        assert_eq!(v[0], "https://x.test/s1/e01.mkv");
        assert_eq!(v[3], "https://x.test/s2/e01.mkv");
    }

    #[test]
    fn plain_links_and_ipv6_are_not_patterns() {
        assert!(expand("https://x.test/file.zip").is_none());
        assert!(expand("http://[::1]:8080/file.zip").is_none());
        assert!(expand("https://x.test/[draft]-notes.txt").is_none());
    }

    #[test]
    fn bad_ranges_explain_themselves() {
        assert!(expand("https://x.test/[9-1]").unwrap().is_err());
        assert!(expand("https://x.test/[a-Z]").unwrap().is_err());
        assert!(expand("https://x.test/[1-99999]").unwrap().is_err());
        assert!(expand("https://x.test/[1-200]/[1-200]").unwrap().is_err());
    }
}
