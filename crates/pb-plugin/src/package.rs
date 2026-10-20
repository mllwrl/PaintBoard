//! `.pbplugin` files: a plugin's folder in one file, to send and install. It is an
//! ordinary zip (made by any zip tool, or by PaintBoard from a plugin being developed),
//! with `manifest.json` at its top or inside one folder at its top.
//!
//! Only what a plugin needs is read: plain files (stored or deflated), not encrypted, no
//! deeper folders, within size limits, each checked against its CRC.

use std::io::{Read, Write};

/// The extension, without the dot.
pub const EXTENSION: &str = "pbplugin";

/// The most a plugin's files may take unpacked, together.
const MAX_TOTAL: usize = 16 << 20;
const MAX_FILES: usize = 200;

/// A plugin's files by name, as unpacked from a `.pbplugin`.
pub type Files = Vec<(String, Vec<u8>)>;

fn u16_at(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| "the .pbplugin file is cut short".to_owned())
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "the .pbplugin file is cut short".to_owned())
}

/// A file name the plugin's folder can hold: letters, digits, `.`, `-`, `_`.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.len() <= 100
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The files of a `.pbplugin`.
pub fn unpack(bytes: &[u8]) -> Result<Files, String> {
    const END: u32 = 0x0605_4b50;
    const CENTRAL: u32 = 0x0201_4b50;
    const LOCAL: u32 = 0x0403_4b50;
    // The end record is in the last 64 KiB (it may carry a comment).
    let earliest = bytes.len().saturating_sub(22 + 0xFFFF);
    let end = (earliest..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| u32_at(bytes, i) == Ok(END))
        .ok_or("this is not a .pbplugin (zip) file")?;
    let count = u16_at(bytes, end + 10)? as usize;
    let mut at = u32_at(bytes, end + 16)? as usize;
    if count > MAX_FILES {
        return Err(format!("the plugin has over {MAX_FILES} files"));
    }
    let mut entries = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        if u32_at(bytes, at)? != CENTRAL {
            return Err("the .pbplugin file is damaged".into());
        }
        let flags = u16_at(bytes, at + 8)?;
        let method = u16_at(bytes, at + 10)?;
        let crc = u32_at(bytes, at + 16)?;
        let packed = u32_at(bytes, at + 20)? as usize;
        let size = u32_at(bytes, at + 24)? as usize;
        let name_len = u16_at(bytes, at + 28)? as usize;
        let extra_len = u16_at(bytes, at + 30)? as usize;
        let comment_len = u16_at(bytes, at + 32)? as usize;
        let local = u32_at(bytes, at + 42)? as usize;
        let name = bytes
            .get(at + 46..at + 46 + name_len)
            .ok_or("the .pbplugin file is cut short")?;
        let name = String::from_utf8_lossy(name).replace('\\', "/");
        at += 46 + name_len + extra_len + comment_len;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err("the .pbplugin file is encrypted".into());
        }
        total = total.saturating_add(size);
        if total > MAX_TOTAL {
            return Err("the plugin is too large (over 16 MB unpacked)".into());
        }
        if u32_at(bytes, local)? != LOCAL {
            return Err("the .pbplugin file is damaged".into());
        }
        let start =
            local + 30 + u16_at(bytes, local + 26)? as usize + u16_at(bytes, local + 28)? as usize;
        let data = bytes
            .get(start..start.saturating_add(packed))
            .ok_or("the .pbplugin file is cut short")?;
        let content = match method {
            0 => data.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(size.min(MAX_TOTAL));
                flate2::read::DeflateDecoder::new(data)
                    .take(size as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("the .pbplugin file is damaged: {e}"))?;
                out
            }
            _ => {
                return Err(format!(
                    "{name}: packed in a way PaintBoard does not read (use plain zip)"
                ));
            }
        };
        if content.len() != size || crc32fast::hash(&content) != crc {
            return Err(format!("{name}: damaged in the .pbplugin file"));
        }
        entries.push((name, content));
    }
    // Zipping a folder puts everything in it: take its contents.
    let top = entries
        .iter()
        .find(|(n, _)| n == "manifest.json" || n.ends_with("/manifest.json"))
        .map(|(n, _)| n.trim_end_matches("manifest.json").to_owned())
        .ok_or("the .pbplugin file has no manifest.json")?;
    Ok(entries
        .into_iter()
        .filter_map(|(name, content)| {
            let name = name.strip_prefix(&top)?;
            plain_name(name).then(|| (name.to_owned(), content))
        })
        .collect())
}

/// A `.pbplugin` of `files` (plain names), deflated.
pub fn pack(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let put16 = |v: &mut Vec<u8>, n: u16| v.extend_from_slice(&n.to_le_bytes());
    let put32 = |v: &mut Vec<u8>, n: u32| v.extend_from_slice(&n.to_le_bytes());
    for (name, content) in files {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(content).expect("writing to memory");
        let packed = encoder.finish().expect("writing to memory");
        let crc = crc32fast::hash(content);
        let offset = out.len() as u32;
        // Local header: version 2.0, UTF-8 names, deflate, no time.
        put32(&mut out, 0x0403_4b50);
        for n in [20, 0x0800, 8, 0, 0x21] {
            put16(&mut out, n);
        }
        put32(&mut out, crc);
        put32(&mut out, packed.len() as u32);
        put32(&mut out, content.len() as u32);
        put16(&mut out, name.len() as u16);
        put16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        put32(&mut central, 0x0201_4b50);
        for n in [20, 20, 0x0800, 8, 0, 0x21] {
            put16(&mut central, n);
        }
        put32(&mut central, crc);
        put32(&mut central, packed.len() as u32);
        put32(&mut central, content.len() as u32);
        put16(&mut central, name.len() as u16);
        for n in [0, 0, 0, 0] {
            put16(&mut central, n);
        }
        put32(&mut central, 0);
        put32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_at = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    put32(&mut out, 0x0605_4b50);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, files.len() as u16);
    put16(&mut out, files.len() as u16);
    put32(&mut out, central_len);
    put32(&mut out, central_at);
    put16(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Files {
        vec![
            ("manifest.json".into(), br#"{"id":"a.b"}"#.to_vec()),
            (
                "main.js".into(),
                "console.log('привет');\n".repeat(50).into_bytes(),
            ),
        ]
    }

    #[test]
    fn packs_and_unpacks() {
        let packed = pack(&files());
        assert_eq!(unpack(&packed).unwrap(), files());
        // Deflate made it smaller than the script alone.
        assert!(packed.len() < files()[1].1.len());
    }

    #[test]
    fn a_zipped_folder_is_read_from_inside_it() {
        let inside: Files = files()
            .into_iter()
            .map(|(n, c)| (format!("my-plugin/{n}"), c))
            .chain([("my-plugin/../evil.js".to_owned(), b"x".to_vec())])
            .chain([("other/deep.js".to_owned(), b"x".to_vec())])
            .collect();
        // Zip tools write folder paths like these (`pack` itself writes plain names).
        let mut zip = pack(&inside);
        assert_eq!(unpack(&zip).unwrap(), files());
        // Damage in a file's packed bytes is noticed.
        let data = 30 + "my-plugin/manifest.json".len() + 2;
        zip[data] ^= 0xFF;
        assert!(unpack(&zip).is_err());
    }

    /// Made by Windows' own Compress-Archive from `examples/number-stickies`.
    #[test]
    fn zips_made_by_windows_are_read() {
        let files = unpack(include_bytes!("testdata/compress-archive.pbplugin")).unwrap();
        let names: Vec<_> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"manifest.json") && names.contains(&"main.js"));
        let (_, manifest) = files.iter().find(|(n, _)| n == "manifest.json").unwrap();
        assert!(crate::Manifest::parse(&String::from_utf8_lossy(manifest)).is_ok());
    }

    #[test]
    fn other_files_are_refused() {
        assert!(unpack(b"not a zip at all").is_err());
        assert!(unpack(&[]).is_err());
        let no_manifest = pack(&[("main.js".into(), b"1".to_vec())]);
        assert!(unpack(&no_manifest).unwrap_err().contains("manifest.json"));
    }
}
