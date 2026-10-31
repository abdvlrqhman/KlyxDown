//! File checksums: MD5, SHA-1, SHA-256 and SHA-512, the ones download pages and Metalink use.

use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Algo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

/// An expected checksum: the algorithm and lowercase hex.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Checksum {
    pub algo: Algo,
    pub hex: String,
}

impl Checksum {
    /// Reads what people paste: `sha256:ABC…`, `SHA-256=abc…` or just the hex. The algorithm
    /// comes from the prefix, else from the length.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (named, hex) = match text.split_once([':', '=', ' ']) {
            Some((name, hex)) => (Algo::from_name(name), hex.trim()),
            None => (None, text),
        };
        let hex = hex.to_ascii_lowercase();
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let by_len = match hex.len() {
            32 => Algo::Md5,
            40 => Algo::Sha1,
            64 => Algo::Sha256,
            128 => Algo::Sha512,
            _ => return None,
        };
        named
            .is_none_or(|a| a == by_len)
            .then_some(Self { algo: by_len, hex })
    }
}

impl Algo {
    /// `md5`, `sha-1`, `SHA256`, `sha-512`, as Metalink and download pages write them.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().replace('-', "").as_str() {
            "md5" => Some(Self::Md5),
            "sha1" => Some(Self::Sha1),
            "sha256" => Some(Self::Sha256),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }
}

/// Hex digest of a file. Blocking: call it off the async runtime.
pub fn file_hash(path: &Path, algo: Algo) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    match algo {
        Algo::Md5 => digest::<md5::Md5>(file),
        Algo::Sha1 => digest::<sha1::Sha1>(file),
        Algo::Sha256 => digest::<sha2::Sha256>(file),
        Algo::Sha512 => digest::<sha2::Sha512>(file),
    }
}

fn digest<D: Digest>(mut file: impl Read) -> io::Result<String> {
    let mut hasher = D::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_checksums() {
        let sha = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
        assert_eq!(
            Checksum::parse(&format!("SHA-256: {sha}")),
            Some(Checksum {
                algo: Algo::Sha256,
                hex: sha.to_lowercase()
            })
        );
        assert_eq!(
            Checksum::parse("d41d8cd98f00b204e9800998ecf8427e").map(|c| c.algo),
            Some(Algo::Md5)
        );
        assert_eq!(Checksum::parse("md5:abc"), None);
        assert_eq!(Checksum::parse(&format!("sha1:{sha}")), None);
        assert_eq!(Checksum::parse("not a checksum"), None);
    }

    #[test]
    fn hashes_files() {
        let path = std::env::temp_dir().join(format!("kd-hash-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let hash = |a| file_hash(&path, a).unwrap();
        assert_eq!(hash(Algo::Md5), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hash(Algo::Sha1), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hash(Algo::Sha256),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(hash(Algo::Sha512).starts_with("ddaf35a193617aba"));
        let _ = std::fs::remove_file(path);
    }
}
