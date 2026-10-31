//! One file out of a remote ZIP: read the archive's table of contents with two or three
//! range requests, then download just that entry's bytes (as a normal, resumable download)
//! and unpack them when it completes.

use std::io::{self, BufReader, Read, Write};
use std::path::Path;

use reqwest::Client;
use reqwest::header::RANGE;
use serde::{Deserialize, Serialize};

use crate::probe::{self, with_headers};
use crate::{Error, Probe, Result};

/// The table of contents is read whole; bigger ones are not worth the round trip.
const MAX_DIRECTORY: u64 = 64 * 1024 * 1024;
/// End-of-directory record, the longest comment, and the ZIP64 locator before it.
const TAIL: u64 = 22 + 65_535 + 20;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZipEntry {
    /// Path inside the archive, with `/` separators.
    pub name: String,
    pub size: u64,
    pub compressed: u64,
    pub dir: bool,
    /// Password-protected entries can't be unpacked here.
    pub encrypted: bool,
    /// Stored (0) and deflate (8) are supported.
    pub method: u16,
    #[serde(skip)]
    crc: u32,
    #[serde(skip)]
    header: u64,
}

impl ZipEntry {
    pub fn supported(&self) -> bool {
        !self.dir && !self.encrypted && matches!(self.method, 0 | 8)
    }
}

/// How to turn the downloaded bytes into the file, kept with the download.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZipPart {
    /// Where the entry's compressed bytes start in the archive.
    pub(crate) offset: u64,
    pub(crate) compressed: u64,
    pub(crate) size: u64,
    pub(crate) method: u16,
    pub(crate) crc: u32,
}

async fn range(
    client: &Client,
    url: &str,
    headers: &[(String, String)],
    start: u64,
    len: u64,
) -> Result<Vec<u8>> {
    let resp = with_headers(client.get(url), headers)
        .header(RANGE, format!("bytes={start}-{}", start + len - 1))
        .send()
        .await
        .map_err(|e| Error::Network(probe::describe(&e)))?;
    if resp.status().as_u16() != 206 {
        return Err(Error::Status(resp.status().as_u16()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Network(probe::describe(&e)))?;
    if bytes.len() as u64 != len {
        return Err(Error::Network("the server sent a short part".into()));
    }
    Ok(bytes.to_vec())
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

fn not_zip() -> Error {
    Error::Network("This is not a ZIP archive, or it is damaged.".into())
}

/// Where the central directory is: (offset, size), from the archive's last bytes.
fn directory(tail: &[u8], tail_start: u64) -> Option<Directory> {
    let eocd = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| tail[i..].starts_with(b"PK\x05\x06"))?;
    let (size, offset) = (u32_at(tail, eocd + 12)?, u32_at(tail, eocd + 16)?);
    if size != u32::MAX && offset != u32::MAX && u16_at(tail, eocd + 10)? != u16::MAX {
        return Some(Directory::At(u64::from(offset), u64::from(size)));
    }
    // ZIP64: a locator right before the record points at the 64-bit record.
    let locator = eocd.checked_sub(20)?;
    tail[locator..]
        .starts_with(b"PK\x06\x07")
        .then(|| u64_at(tail, locator + 8))
        .flatten()
        .filter(|&at| at < tail_start + tail.len() as u64)
        .map(Directory::Zip64)
}

enum Directory {
    At(u64, u64),
    /// Offset of the ZIP64 end record, which has the real numbers.
    Zip64(u64),
}

fn entries(cd: &[u8]) -> Option<Vec<ZipEntry>> {
    let mut out = Vec::new();
    let mut i = 0;
    while cd.get(i..i + 4) == Some(b"PK\x01\x02") {
        let flags = u16_at(cd, i + 8)?;
        let (nlen, elen, clen) = (
            u16_at(cd, i + 28)? as usize,
            u16_at(cd, i + 30)? as usize,
            u16_at(cd, i + 32)? as usize,
        );
        let raw = cd.get(i + 46..i + 46 + nlen)?;
        // Bit 11: UTF-8 names. Older tools use CP437; ASCII is the same either way.
        let name = String::from_utf8_lossy(raw).into_owned();
        let mut size = u64::from(u32_at(cd, i + 24)?);
        let mut compressed = u64::from(u32_at(cd, i + 20)?);
        let mut header = u64::from(u32_at(cd, i + 42)?);
        // ZIP64 extra field: 64-bit values for the fields that are 0xFFFFFFFF, in this order.
        let extra = cd.get(i + 46 + nlen..i + 46 + nlen + elen)?;
        let mut j = 0;
        while j + 4 <= extra.len() {
            let (id, len) = (u16_at(extra, j)?, u16_at(extra, j + 2)? as usize);
            if id == 1 {
                let mut k = j + 4;
                for field in [&mut size, &mut compressed, &mut header] {
                    if *field == u64::from(u32::MAX) {
                        *field = u64_at(extra, k)?;
                        k += 8;
                    }
                }
            }
            j += 4 + len;
        }
        out.push(ZipEntry {
            dir: name.ends_with('/'),
            name,
            size,
            compressed,
            encrypted: flags & 1 != 0,
            method: u16_at(cd, i + 10)?,
            crc: u32_at(cd, i + 16)?,
            header,
        });
        i += 46 + nlen + elen + clen;
    }
    Some(out)
}

/// Lists a remote archive without downloading it.
pub(crate) async fn list(
    client: &Client,
    probe: &Probe,
    headers: &[(String, String)],
) -> Result<Vec<ZipEntry>> {
    let size = match probe.size {
        Some(s) if probe.resumable => s,
        _ => {
            return Err(Error::Network(
                "This server can't send parts of the file, so the archive can't be opened \
                 without downloading all of it."
                    .into(),
            ));
        }
    };
    let tail_start = size.saturating_sub(TAIL);
    let tail = range(client, &probe.url, headers, tail_start, size - tail_start).await?;
    let (offset, len) = match directory(&tail, tail_start).ok_or_else(not_zip)? {
        Directory::At(o, l) => (o, l),
        Directory::Zip64(at) => {
            let rec = range(client, &probe.url, headers, at, 56).await?;
            if !rec.starts_with(b"PK\x06\x06") {
                return Err(not_zip());
            }
            (
                u64_at(&rec, 48).ok_or_else(not_zip)?,
                u64_at(&rec, 40).ok_or_else(not_zip)?,
            )
        }
    };
    if len > MAX_DIRECTORY || offset.saturating_add(len) > size {
        return Err(not_zip());
    }
    // The directory is usually inside the tail already.
    let cd = if offset >= tail_start {
        let from = (offset - tail_start) as usize;
        tail.get(from..from + len as usize)
            .ok_or_else(not_zip)?
            .to_vec()
    } else {
        range(client, &probe.url, headers, offset, len).await?
    };
    entries(&cd).ok_or_else(not_zip)
}

/// Finds the entry and where its data starts (after its local header, whose lengths may
/// differ from the directory's).
pub(crate) async fn part(
    client: &Client,
    probe: &Probe,
    headers: &[(String, String)],
    name: &str,
) -> Result<(ZipEntry, ZipPart)> {
    let entry = list(client, probe, headers)
        .await?
        .into_iter()
        .find(|e| e.name == name)
        .ok_or_else(|| Error::Network(format!("{name} is not in the archive.")))?;
    if !entry.supported() {
        return Err(Error::Network(if entry.encrypted {
            format!("{name} is password-protected.")
        } else {
            format!("{name} uses a compression method KlyxDown can't unpack.")
        }));
    }
    let local = range(client, &probe.url, headers, entry.header, 30).await?;
    if !local.starts_with(b"PK\x03\x04") {
        return Err(not_zip());
    }
    let skip = 30
        + u64::from(u16_at(&local, 26).unwrap_or(0))
        + u64::from(u16_at(&local, 28).unwrap_or(0));
    let part = ZipPart {
        offset: entry.header + skip,
        compressed: entry.compressed,
        size: entry.size,
        method: entry.method,
        crc: entry.crc,
    };
    Ok((entry, part))
}

/// Turns the downloaded bytes into the file and checks its CRC. Blocking.
pub(crate) fn unpack(raw: &Path, target: &Path, part: &ZipPart) -> io::Result<()> {
    let input = BufReader::new(std::fs::File::open(raw)?);
    let mut reader: Box<dyn Read> = match part.method {
        8 => Box::new(flate2::read::DeflateDecoder::new(input)),
        _ => Box::new(input),
    };
    let mut out = std::fs::File::create_new(target)?;
    let mut crc = flate2::Crc::new();
    let mut buf = vec![0u8; 1 << 20];
    let unpacked = (|| -> io::Result<()> {
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            crc.update(&buf[..n]);
            out.write_all(&buf[..n])?;
        }
        out.sync_all()?;
        if crc.sum() != part.crc || u64::from(crc.amount()) != part.size & 0xffff_ffff {
            return Err(io::Error::other(
                "The file from the archive is damaged (its checksum does not match).",
            ));
        }
        Ok(())
    })();
    if unpacked.is_err() {
        drop(out);
        let _ = std::fs::remove_file(target);
    } else {
        let _ = std::fs::remove_file(raw);
    }
    unpacked
}

/// Writes a minimal ZIP (local headers, central directory, end record). For tests.
#[doc(hidden)]
pub fn build_for_tests(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, data, deflate) in files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let packed = if *deflate {
            let mut e =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        } else {
            data.to_vec()
        };
        let method: u16 = if *deflate { 8 } else { 0 };
        let at = out.len() as u32;
        let head = |sig: &[u8], central: bool| {
            let mut h = sig.to_vec();
            if central {
                h.extend(20u16.to_le_bytes());
            }
            h.extend(20u16.to_le_bytes());
            h.extend(0u16.to_le_bytes());
            h.extend(method.to_le_bytes());
            h.extend([0u8; 4]);
            h.extend(crc.sum().to_le_bytes());
            h.extend((packed.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes());
            if central {
                h.extend([0u8; 10]);
                h.extend(at.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        out.extend(head(b"PK\x03\x04", false));
        out.extend(&packed);
        cd.extend(head(b"PK\x01\x02", true));
    }
    let cd_at = out.len() as u32;
    out.extend(&cd);
    out.extend(b"PK\x05\x06");
    out.extend([0u8; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((cd.len() as u32).to_le_bytes());
    out.extend(cd_at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_table_of_contents() {
        let zip = build_for_tests(&[
            ("a.txt", b"hello", false),
            ("dir/b.txt", &b"abc".repeat(200), true),
        ]);
        let Some(Directory::At(offset, len)) = directory(&zip, 0) else {
            panic!("no directory");
        };
        let list = entries(&zip[offset as usize..(offset + len) as usize]).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(
            (list[0].name.as_str(), list[0].size, list[0].method),
            ("a.txt", 5, 0)
        );
        assert_eq!(
            (list[1].name.as_str(), list[1].size, list[1].method),
            ("dir/b.txt", 600, 8)
        );
        assert!(list[1].compressed < 600 && list.iter().all(ZipEntry::supported));
        assert!(directory(b"not a zip at all, nothing to see here", 0).is_none());
    }

    #[test]
    fn unpacks_and_checks() {
        let dir = std::env::temp_dir().join(format!("kd-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data = b"abc".repeat(200);
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&data).unwrap();
        let raw = dir.join("raw");
        std::fs::write(&raw, e.finish().unwrap()).unwrap();
        let mut crc = flate2::Crc::new();
        crc.update(&data);
        let mut part = ZipPart {
            method: 8,
            size: 600,
            crc: crc.sum(),
            ..ZipPart::default()
        };
        unpack(&raw, &dir.join("b.txt"), &part).unwrap();
        assert_eq!(std::fs::read(dir.join("b.txt")).unwrap(), data);
        assert!(!raw.exists());

        std::fs::write(&raw, b"hello").unwrap();
        part = ZipPart {
            method: 0,
            size: 5,
            crc: 0xdead_beef,
            ..part
        };
        assert!(unpack(&raw, &dir.join("a.txt"), &part).is_err());
        assert!(!dir.join("a.txt").exists() && raw.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
