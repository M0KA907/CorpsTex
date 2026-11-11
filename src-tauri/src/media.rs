//! Bounded raster image previews; no remote fetching or arbitrary file bridge.
use crate::runtime::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
#[cfg(target_os = "linux")]
use std::{fs::OpenOptions, path::Component};

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
    if source.starts_with("file://")
        && !source.starts_with("file:///")
        && !source.starts_with("file://localhost/")
    {
        return None;
    }
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
        // Decode once. Residual percent escapes and alternate separators/schemes
        // are rejected rather than interpreted differently on another layer.
        .filter(|path| {
            !path.is_empty()
                && !path.starts_with("//")
                && !path.contains(['\\', ':', '%'])
                && !path.chars().any(char::is_control)
                && !path.split('/').any(|part| part == "..")
        })
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub path: String,
    pub external_path: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Preview {
    Image { url: String },
    External { path: String },
}

pub enum OpenPreview {
    Image(File),
    External(String),
}

pub fn image_path(thread: &Value, root: &str, path: &str) -> Result<PathBuf, Error> {
    if path.is_empty() || path.len() > 4096 || !reported(thread, path) {
        return Err(Error::InvalidRequest);
    }
    Ok(Path::new(root).join(normalized(path).ok_or(Error::InvalidRequest)?))
}

pub fn open(thread: &Value, root: &str, selection: &Selection) -> Result<OpenPreview, Error> {
    let path = image_path(thread, root, &selection.path)?;
    let root = Path::new(root);
    if !root.is_absolute() || !path.is_absolute() {
        return Err(Error::InvalidRequest);
    }
    if !path.starts_with(root) && selection.external_path.as_deref() != path.to_str() {
        return Ok(OpenPreview::External(
            path.to_str().ok_or(Error::InvalidRequest)?.into(),
        ));
    }
    Ok(OpenPreview::Image(open_file(&path)?))
}

// Hold every ancestor; each subsequent open is relative to its descriptor.
// No checked pathname is reopened. This also rejects final/intermediate symlinks.
#[cfg(target_os = "linux")]
fn open_file(path: &Path) -> Result<File, Error> {
    open_file_at(path, || {})
}

#[cfg(target_os = "linux")]
fn open_file_at(path: &Path, before_final: impl FnOnce()) -> Result<File, Error> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let mut components = path.components().peekable();
    if components.next() != Some(Component::RootDir) {
        return Err(Error::InvalidRequest);
    }
    let mut directory = File::open("/").map_err(|_| Error::InvalidRequest)?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(Error::InvalidRequest);
        };
        let target = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name);
        let final_part = components.peek().is_none();
        if final_part {
            before_final();
            return OpenOptions::new()
                .read(true)
                // O_NOFOLLOW | O_NONBLOCK: a FIFO replacement cannot hang open.
                .custom_flags(0x20000 | 0x800)
                .open(target)
                .map_err(|_| Error::InvalidRequest);
        }
        directory = OpenOptions::new()
            .read(true)
            // O_NOFOLLOW | O_DIRECTORY
            .custom_flags(0x20000 | 0x10000)
            .open(target)
            .map_err(|_| Error::InvalidRequest)?;
    }
    Err(Error::InvalidRequest)
}

#[cfg(not(target_os = "linux"))]
fn open_file(_: &Path) -> Result<File, Error> {
    // Other platforms need equivalent descriptor-relative containment first.
    Err(Error::InvalidRequest)
}

pub fn read(file: File) -> Result<String, Error> {
    if !file
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_BYTES)
    {
        return Err(Error::InvalidRequest);
    }
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
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "corpstex-preview-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn root(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn reported_image(path: &str) -> Value {
        json!({"turns":[{"items":[{"type":"imageGeneration","savedPath":path}]}]})
    }
    #[test]
    fn traversal_and_ambiguous_uri_forms_are_rejected_after_one_decode() {
        for path in [
            "../out.png",
            "a/../out.png",
            "%2e%2e/out.png",
            "a/%2E%2E%2fout.png",
            "file:///a/../out.png",
            "sandbox:/a/%2e%2e/out.png",
            "%252e%252e/out.png",
            "%2f%2fhost/out.png",
            "file://host/out.png",
            "file://localhost.evil/out.png",
            "file:/out.png",
            "https://host/out.png",
            "a%5cout.png",
            "a%00.png",
            "a%0a.png",
            "a%GG.png",
            "a%.png",
            "C:/out.png",
        ] {
            assert!(normalized(path).is_none(), "{path}");
        }
        assert_eq!(
            normalized("sandbox:/tmp/my%20image.png"),
            Some("/tmp/my image.png".into())
        );
        assert_eq!(
            normalized("file://localhost/tmp/image.png"),
            Some("/tmp/image.png".into())
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn project_images_and_external_consent_have_separate_scopes() {
        let project = Temp::new();
        let outside = Temp::new();
        let bytes = b"\x89PNG\r\n\x1a\nfixture";
        std::fs::write(project.0.join("my image.png"), bytes).unwrap();
        for path in [
            "my%20image.png".to_owned(),
            project.0.join("my image.png").to_str().unwrap().to_owned(),
        ] {
            assert!(preview(&reported_image(&path), project.root(), &path).is_ok());
        }
        let external = outside.0.join("generated.png").to_str().unwrap().to_owned();
        std::fs::write(&external, bytes).unwrap();
        for item in [
            json!({"type":"imageGeneration","savedPath":external}),
            json!({"type":"imageView","path":external}),
            json!({"type":"agentMessage","text":format!("![image]({external})")}),
        ] {
            let thread = json!({"turns":[{"items":[item]}]});
            let selection = Selection {
                path: external.clone(),
                external_path: None,
            };
            assert!(
                matches!(open(&thread, project.root(), &selection), Ok(OpenPreview::External(p)) if p == external)
            );
            assert!(matches!(
                open(
                    &thread,
                    project.root(),
                    &Selection {
                        external_path: Some("/wrong.png".into()),
                        ..selection
                    }
                ),
                Ok(OpenPreview::External(_))
            ));
            let selected = Selection {
                path: external.clone(),
                external_path: Some(external.clone()),
            };
            let OpenPreview::Image(file) = open(&thread, project.root(), &selected).unwrap() else {
                panic!("explicit consent")
            };
            assert!(read(file).is_ok());
            assert!(open(&json!({}), project.root(), &selected).is_err());
            // One confirmed call creates no future permission.
            assert!(preview(&thread, project.root(), &external).is_err());
        }
        // A sibling with a common root name is still external (component containment).
        let sibling = format!("{}-sibling/image.png", project.root());
        assert!(matches!(
            open(
                &reported_image(&sibling),
                project.root(),
                &Selection {
                    path: sibling,
                    external_path: None
                }
            ),
            Ok(OpenPreview::External(_))
        ));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn held_handles_prevent_symlink_and_replacement_redirection() {
        use std::os::unix::fs::symlink;
        let project = Temp::new();
        let outside = Temp::new();
        std::fs::create_dir(project.0.join("images")).unwrap();
        let target = project.0.join("images/image.png");
        let bytes = b"\x89PNG\r\n\x1a\noriginal";
        std::fs::write(&target, bytes).unwrap();
        std::fs::write(outside.0.join("image.png"), b"\x89PNG\r\n\x1a\noutside").unwrap();
        symlink(&outside.0, project.0.join("escape")).unwrap();
        symlink(&target, project.0.join("inside.png")).unwrap();
        symlink(outside.0.join("image.png"), project.0.join("outside.png")).unwrap();
        for path in ["escape/image.png", "inside.png", "outside.png"] {
            assert!(preview(&reported_image(path), project.root(), path).is_err());
        }
        let file = open_file_at(&target, || {
            std::fs::rename(project.0.join("images"), project.0.join("held")).unwrap();
            symlink(&outside.0, project.0.join("images")).unwrap();
        })
        .unwrap();
        assert_eq!(
            read(file).unwrap(),
            format!("data:image/png;base64,{}", base64(bytes))
        );
        let target = project.0.join("held/image.png");
        assert!(open_file_at(&target, || {
            std::fs::remove_file(&target).unwrap();
            symlink(outside.0.join("image.png"), &target).unwrap();
        })
        .is_err());
        let opened = open_file(&outside.0.join("image.png")).unwrap();
        std::fs::rename(outside.0.join("image.png"), outside.0.join("saved.png")).unwrap();
        std::fs::write(outside.0.join("image.png"), b"not the opened file").unwrap();
        assert!(read(opened)
            .unwrap()
            .ends_with(&base64(b"\x89PNG\r\n\x1a\noutside")));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn descriptor_reads_preserve_type_size_and_signature_limits() {
        let project = Temp::new();
        assert!(read(File::open(&project.0).unwrap()).is_err());
        assert!(read(open_file(Path::new("/dev/null")).unwrap()).is_err());
        let path = project.0.join("image.png");
        for bytes in [
            b"\x89PNG\r\n\x1a\n".as_slice(),
            b"\xff\xd8\xff",
            b"GIF87a",
            b"GIF89a",
            b"RIFF0000WEBP",
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(read(open_file(&path).unwrap()).is_ok());
        }
        std::fs::write(&path, b"<svg>not raster</svg>").unwrap();
        assert!(read(open_file(&path).unwrap()).is_err());
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        assert!(read(open_file(&path).unwrap()).is_err());
        // Growth after open is validated on the same descriptor, not a pathname.
        file.set_len(8).unwrap();
        let opened = open_file(&path).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        assert!(read(opened).is_err());
        let fifo = project.0.join("pipe.png");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        assert!(read(open_file(&fifo).unwrap()).is_err());
    }
    fn preview(thread: &Value, root: &str, path: &str) -> Result<String, Error> {
        match open(
            thread,
            root,
            &Selection {
                path: path.into(),
                external_path: None,
            },
        )? {
            OpenPreview::Image(file) => read(file),
            OpenPreview::External(_) => Err(Error::InvalidRequest),
        }
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
    #[cfg(target_os = "linux")]
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
