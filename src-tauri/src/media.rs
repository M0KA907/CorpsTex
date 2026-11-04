//! Bounded raster image previews; no remote fetching or arbitrary file bridge.
use crate::runtime::Error;
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_DATA: usize = 8 * 1024 * 1024;
const MAX_BYTES: u64 = 12 * 1024 * 1024;

pub fn valid_data(url: &str) -> bool {
    let Some((header, data)) = url.split_once(',') else {
        return false;
    };
    matches!(
        header,
        "data:image/png;base64"
            | "data:image/jpeg;base64"
            | "data:image/gif;base64"
            | "data:image/webp;base64"
    ) && !data.is_empty()
        && url.len() <= MAX_DATA
        && data.len().is_multiple_of(4)
        && data
            .trim_end_matches('=')
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/')
        && data.len() - data.trim_end_matches('=').len() <= 2
}

fn normalized(source: &str) -> Option<String> {
    let source = source
        .strip_prefix("sandbox:")
        .or_else(|| source.strip_prefix("file://localhost"))
        .or_else(|| source.strip_prefix("file://"))
        .unwrap_or(source);
    if source.starts_with("//") || source.starts_with("\\\\") {
        return None;
    }
    let mut bytes = Vec::with_capacity(source.len());
    let mut input = source.as_bytes().iter().copied().peekable();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let a = input.next()?;
            let b = input.next()?;
            bytes.push((a as char).to_digit(16)? as u8 * 16 + (b as char).to_digit(16)? as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes)
        .ok()
        .filter(|path| !path.contains('\0'))
}

fn markdown_paths(text: &str) -> impl Iterator<Item = &str> {
    text.split("](").skip(1).filter_map(|rest| {
        let rest = rest.trim_start();
        if let Some(angle) = rest.strip_prefix('<') {
            return angle.split_once('>').map(|(path, _)| path);
        }
        let mut depth = 0u32;
        let mut escaped = false;
        for (index, ch) in rest.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '(' => depth += 1,
                ')' if depth > 0 => depth -= 1,
                ')' => return Some(&rest[..index]),
                _ if ch.is_whitespace() && depth == 0 => return Some(&rest[..index]),
                _ => {}
            }
        }
        None
    })
}

fn reported(thread: &Value, path: &str) -> bool {
    let Some(wanted) = normalized(path) else {
        return false;
    };
    let matches =
        |candidate: &str| candidate == path || normalized(candidate).as_ref() == Some(&wanted);
    thread["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|turn| turn["items"].as_array().into_iter().flatten())
        .any(|item| match item["type"].as_str() {
            Some("imageGeneration") => item["savedPath"].as_str().is_some_and(matches),
            Some("imageView") => item["path"].as_str().is_some_and(matches),
            Some("userMessage") => item["content"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|part| {
                    part["type"] == "localImage" && part["path"].as_str().is_some_and(matches)
                }),
            Some("agentMessage") => item["text"]
                .as_str()
                .is_some_and(|text| markdown_paths(text).any(matches)),
            _ => false,
        })
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

pub fn image_path(thread: &Value, root: &str, path: &str) -> Result<PathBuf, Error> {
    if path.is_empty() || path.len() > 4096 || !reported(thread, path) {
        return Err(Error::InvalidRequest);
    }
    Ok(Path::new(root).join(normalized(path).ok_or(Error::InvalidRequest)?))
}

pub fn read(path: &Path) -> Result<String, Error> {
    // Reject devices, directories and oversized files before reading, then bound reads against growth.
    if !std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_BYTES) {
        return Err(Error::InvalidRequest);
    }
    let file = File::open(path).map_err(|_| Error::InvalidRequest)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::InvalidRequest)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Error::InvalidRequest);
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err(Error::InvalidRequest);
    };
    Ok(format!("data:{mime};base64,{}", base64(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn preview(thread: &Value, root: &str, path: &str) -> Result<String, Error> {
        read(&image_path(thread, root, path)?)
    }
    #[test]
    fn data_validation_and_encoding() {
        for (input, expected) in [(b"f".as_slice(), "Zg=="), (b"fo", "Zm8="), (b"foo", "Zm9v")] {
            assert_eq!(base64(input), expected);
        }
        assert!(valid_data("data:image/png;base64,Zm9v"));
        for invalid in [
            "data:image/svg+xml;base64,Zm9v",
            "https://example.com/a.png",
            "data:image/png;base64,",
            "data:image/png;base64,foo",
            "data:image/png;base64,Zm=v",
            "data:image/png;base64,====",
        ] {
            assert!(!valid_data(invalid));
        }
    }
    #[test]
    fn markdown_paths_handle_encoded_spaces_titles_and_local_uri_forms() {
        let thread = json!({"turns":[{"items":[{"type":"agentMessage","text":"![a](<output/my image.png> \"title\") [Save](sandbox:/tmp/picture.png) ![nested](output/test(v2).png)"}]}]});
        assert!(reported(&thread, "output/my%20image.png"));
        assert!(reported(&thread, "/tmp/picture.png"));
        assert!(reported(&thread, "output/test(v2).png"));
        assert!(!reported(&thread, "output/other.png"));
        assert_eq!(
            normalized("file:///tmp/my%20image.png"),
            Some("/tmp/my image.png".into())
        );
        assert_eq!(normalized("/tmp/%00.png"), None);
        assert_eq!(normalized("//example.com/image.png"), None);
    }

    #[test]
    fn local_previews_require_reported_path_and_raster_signature() {
        let dir = std::env::temp_dir().join(format!("corpstex-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nfixture").unwrap();
        let root = dir.to_str().unwrap();
        let thread = json!({"turns":[{"items":[{"type":"imageGeneration","savedPath":path}]}]});
        assert!(preview(&thread, root, path.to_str().unwrap())
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(preview(&thread, root, "fixture.png").is_err());
        assert!(preview(&json!({"turns":[]}), root, path.to_str().unwrap()).is_err());
        std::fs::write(&path, b"secret text").unwrap();
        assert!(preview(&thread, root, path.to_str().unwrap()).is_err());
        let markdown =
            json!({"turns":[{"items":[{"type":"agentMessage","text":"![preview](fixture.png)"}]}]});
        assert!(reported(&markdown, "fixture.png"));
        assert!(!reported(&markdown, "other.png"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
