//! Editor-only filesystem authority. No runtime requests, inference or durable buffers.
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    hash::BuildHasher,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

const LIMIT: u64 = 32 * 1024 * 1024;
#[derive(Default)]
pub struct Editor {
    roots: Mutex<HashMap<String, HashMap<PathBuf, Arc<File>>>>,
    dirty: Mutex<HashSet<String>>,
    gate: Mutex<()>,
    pub exit_pending: AtomicBool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub kind: String,
    pub message: String,
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn error(kind: &str, message: &str) -> Error {
    Error {
        kind: kind.into(),
        message: message.into(),
    }
}
fn io(_: std::io::Error) -> Error {
    error(
        "filesystem",
        "Filesystem operation failed. Check permissions and whether the file still exists.",
    )
}
pub fn root_label(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{unc}")
    } else {
        value.strip_prefix("\\\\?\\").unwrap_or(&value).to_owned()
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    Authorize,
    List {
        path: String,
    },
    Read {
        path: String,
        large: bool,
    },
    Inspect {
        path: String,
    },
    Poll {
        path: String,
        expected: String,
    },
    Save {
        path: String,
        expected: String,
        text: String,
        encoding: String,
        bom: bool,
    },
    Create {
        path: String,
        directory: bool,
    },
    Rename {
        path: String,
        destination: String,
    },
    Duplicate {
        path: String,
        destination: String,
    },
    Delete {
        path: String,
        expected: Option<String>,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub text: String,
    pub revision: String,
    pub encoding: String,
    pub bom: bool,
    pub bytes: usize,
}
fn revision(bytes: &[u8]) -> String {
    // Two independently keyed process-local hashes; no file content in revision tokens.
    static KEYS: OnceLock<[RandomState; 2]> = OnceLock::new();
    let keys = KEYS.get_or_init(|| [RandomState::new(), RandomState::new()]);
    format!(
        "{:016x}{:016x}-{}",
        keys[0].hash_one(bytes),
        keys[1].hash_one(bytes),
        bytes.len()
    )
}
pub fn decode(bytes: &[u8]) -> Result<Document> {
    let (encoding, bom, text) = if bytes.starts_with(&[0xff, 0xfe])
        || bytes.starts_with(&[0xfe, 0xff])
    {
        if !bytes.len().is_multiple_of(2) {
            return Err(error("encoding", "Invalid UTF-16. File was not changed."));
        }
        let le = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if le {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        (
            if le { "utf16le" } else { "utf16be" },
            true,
            String::from_utf16(&units)
                .map_err(|_| error("encoding", "Invalid UTF-16. File was not changed."))?,
        )
    } else {
        let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
        ("utf8", bom, std::str::from_utf8(if bom {&bytes[3..]} else {bytes}).map_err(|_| error("encoding", "Unsupported encoding or binary file. Convert explicitly outside CorpsTex before editing."))?.to_owned())
    };
    if text.contains('\0') {
        return Err(error("binary", "Binary files cannot be edited as text."));
    }
    // Monaco normalizes line endings; refuse lossy round trips.
    let crlf = text.matches("\r\n").count();
    if text.matches('\r').count() != crlf || (crlf > 0 && text.matches('\n').count() != crlf) {
        return Err(error("lineEndings", "Mixed or standalone CR line endings require an explicit external conversion before editing."));
    }
    Ok(Document {
        text,
        revision: revision(bytes),
        encoding: encoding.into(),
        bom,
        bytes: bytes.len(),
    })
}
fn encode(text: &str, encoding: &str, bom: bool) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    match encoding {
        "utf8" => {
            if bom {
                bytes.extend([0xef, 0xbb, 0xbf]);
            }
            bytes.extend(text.as_bytes());
        }
        "utf16le" | "utf16be" if bom => {
            bytes.extend(if encoding == "utf16le" {
                [0xff, 0xfe]
            } else {
                [0xfe, 0xff]
            });
            for unit in text.encode_utf16() {
                bytes.extend(if encoding == "utf16le" {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
        }
        _ => return Err(error("encoding", "Unsupported encoding conversion.")),
    }
    if bytes.len() as u64 > LIMIT {
        return Err(error("largeFile", "Saving is limited to 32 MiB."));
    }
    Ok(bytes)
}
fn relative(path: &str) -> Result<Vec<String>> {
    if path.is_empty() || path.contains('\\') || path.contains(':') || path.contains('\0') {
        return Err(error("path", "Use a nonempty project-relative path."));
    }
    let mut parts = Vec::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(name) => {
                let name = name
                    .to_str()
                    .ok_or_else(|| error("path", "Unsupported filename."))?;
                if name.ends_with(['.', ' '])
                    || name
                        .chars()
                        .any(|c| c.is_control() || "<>\"|?*".contains(c))
                    || matches!(
                        name.split('.')
                            .next()
                            .unwrap_or("")
                            .to_ascii_uppercase()
                            .as_str(),
                        "CON"
                            | "PRN"
                            | "AUX"
                            | "NUL"
                            | "COM1"
                            | "COM2"
                            | "COM3"
                            | "COM4"
                            | "COM5"
                            | "COM6"
                            | "COM7"
                            | "COM8"
                            | "COM9"
                            | "LPT1"
                            | "LPT2"
                            | "LPT3"
                            | "LPT4"
                            | "LPT5"
                            | "LPT6"
                            | "LPT7"
                            | "LPT8"
                            | "LPT9"
                    )
                {
                    return Err(error("path", "Filename is not portable to Windows."));
                }
                if name.eq_ignore_ascii_case(".git") || name.starts_with(".corpstex-save-") {
                    return Err(error(
                        "path",
                        "Git metadata and editor temporary files are protected.",
                    ));
                }
                parts.push(name.to_owned());
            }
            _ => return Err(error("path", "Traversal and absolute paths are forbidden.")),
        }
    }
    if parts.is_empty() {
        return Err(error("path", "Project root cannot be modified."));
    }
    Ok(parts)
}
fn checked_meta(path: &Path) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path).map_err(io)?;
    if meta.file_type().is_symlink() {
        return Err(error(
            "symlink",
            "Symlinks are not editable. Open the real project instead.",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(error("symlink", "Reparse points are not editable."));
        }
    }
    Ok(meta)
}
fn open_directory(path: &Path) -> Result<File> {
    checked_meta(path)?;
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(0x20000 | 0x10000);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(0x02000000 | 0x00200000).share_mode(3);
    }
    let file = opts.open(path).map_err(io)?;
    let meta = file.metadata().map_err(io)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(error("symlink", "Reparse points are not editable."));
        }
    }
    if !meta.is_dir() {
        return Err(error("path", "Expected a directory."));
    }
    Ok(file)
}
// On Linux, hold each directory and address children relative to that open handle.
// Renaming/replacing ancestors cannot redirect operations outside the selected root.
struct Parent {
    _handles: Vec<File>,
    path: PathBuf,
}
fn parent(root: &Path, parts: &[String]) -> Result<Parent> {
    let mut handles = vec![open_directory(root)?];
    #[cfg(not(target_os = "linux"))]
    let mut base = root.to_path_buf();
    #[cfg(target_os = "linux")]
    let mut base: PathBuf;
    for part in &parts[..parts.len() - 1] {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            base = PathBuf::from(format!(
                "/proc/self/fd/{}",
                handles.last().unwrap().as_raw_fd()
            ));
        }
        base = base.join(part);
        handles.push(open_directory(&base)?);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        base = PathBuf::from(format!(
            "/proc/self/fd/{}",
            handles.last().unwrap().as_raw_fd()
        ));
    }
    Ok(Parent {
        _handles: handles,
        path: base.join(parts.last().unwrap()),
    })
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let meta = checked_meta(path)?;
    if !meta.is_file() {
        return Err(error("path", "Only regular files may be edited."));
    }
    if meta.len() > LIMIT {
        return Err(error("largeFile", "Files over 32 MiB are not opened."));
    }
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(0x20000 | 0x800);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(0x00200000);
    }
    let file = opts.open(path).map_err(io)?;
    let meta = file.metadata().map_err(io)?;
    if !meta.is_file() {
        return Err(error("path", "Only regular files may be edited."));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(error("symlink", "Reparse points are not editable."));
        }
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > LIMIT {
        return Err(error("largeFile", "File grew beyond 32 MiB."));
    }
    Ok(bytes)
}
fn move_no_replace(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        unsafe extern "C" {
            fn renameat2(
                oldfd: i32,
                old: *const std::ffi::c_char,
                newfd: i32,
                new: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let old = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| error("path", "Invalid path."))?;
        let new = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| error("path", "Invalid path."))?;
        // RENAME_NOREPLACE is atomic and never clobbers the destination.
        if unsafe { renameat2(-100, old.as_ptr(), -100, new.as_ptr(), 1) } != 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
        }
        let old: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let new: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe { MoveFileExW(old.as_ptr(), new.as_ptr(), 8) } == 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (source, destination);
        Err(error(
            "platform",
            "Atomic no-clobber moves are unsupported on this platform.",
        ))
    }
}
fn replace_checked(temp: &Path, target: &Path, original: &[u8], relative: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        unsafe extern "C" {
            fn renameat2(
                oldfd: i32,
                old: *const std::ffi::c_char,
                newfd: i32,
                new: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let old = CString::new(temp.as_os_str().as_bytes())
            .map_err(|_| error("path", "Invalid path."))?;
        let new = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| error("path", "Invalid path."))?;
        // Atomically retain the displaced file. A concurrent writer cannot vanish between
        // the final precondition check and replacement without leaving inspectable bytes.
        if unsafe { renameat2(-100, old.as_ptr(), -100, new.as_ptr(), 2) } != 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        if !matches!(read(temp),Ok(ref bytes) if bytes.as_slice()==original) {
            let retained = format!(
                "{}.corpstex-conflict-{}",
                relative
                    .rsplit_once('/')
                    .map(|(parent, _)| format!("{parent}/"))
                    .unwrap_or_default(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            let destination = target
                .parent()
                .unwrap()
                .join(Path::new(&retained).file_name().unwrap());
            if move_no_replace(temp, &destination).is_err() {
                return Err(error("saveRace",&format!("A simultaneous change was detected. Displaced disk content remains in temporary file {}. Keep it for manual recovery; your buffer is at the original path.",temp.file_name().unwrap_or_default().to_string_lossy())));
            }
            return Err(error("saveRace",&format!("A simultaneous disk change was detected. Your buffer is at the original path; displaced disk content was retained at {retained}. Review both files before continuing.")));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn ReplaceFileW(
                replaced: *const u16,
                replacement: *const u16,
                backup: *const u16,
                flags: u32,
                exclude: *mut std::ffi::c_void,
                reserved: *mut std::ffi::c_void,
            ) -> i32;
        }
        let retained = format!(
            "{}.corpstex-conflict-{}",
            relative
                .rsplit_once('/')
                .map(|(parent, _)| format!("{parent}/"))
                .unwrap_or_default(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let backup = target
            .parent()
            .unwrap()
            .join(Path::new(&retained).file_name().unwrap());
        let wide = |p: &Path| {
            p.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<u16>>()
        };
        let (old, new, bak) = (wide(target), wide(temp), wide(&backup));
        if unsafe {
            ReplaceFileW(
                old.as_ptr(),
                new.as_ptr(),
                bak.as_ptr(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io(std::io::Error::last_os_error()));
        }
        if !matches!(read(&backup),Ok(ref bytes) if bytes.as_slice()==original) {
            return Err(error("saveRace",&format!("A simultaneous disk change was detected. Your buffer is saved; displaced disk content was retained at {retained}. Review both files.")));
        }
        fs::remove_file(backup).map_err(io)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (temp, target, original, relative);
        Err(error(
            "platform",
            "Atomic checked saves are unsupported on this platform.",
        ))
    }
}
impl Editor {
    pub fn authorized(&self, window: &str, root: &str) -> Result<PathBuf> {
        let roots = self
            .roots
            .lock()
            .map_err(|_| error("busy", "Editor state unavailable."))?;
        roots
            .get(window)
            .and_then(|r| r.keys().find(|r| root_label(r) == root))
            .cloned()
            .ok_or_else(|| error("project", "Select this project in the editor first."))
    }
    pub fn dirty(&self, window: &str, dirty: bool) -> Result<()> {
        let mut windows = self
            .dirty
            .lock()
            .map_err(|_| error("busy", "Editor state unavailable."))?;
        if dirty {
            windows.insert(window.into());
        } else {
            windows.remove(window);
        }
        Ok(())
    }
    pub fn has_dirty(&self) -> bool {
        self.dirty.lock().map(|d| !d.is_empty()).unwrap_or(true)
    }
    pub fn detach(&self, window: &str) {
        if let Ok(mut r) = self.roots.lock() {
            r.remove(window);
        }
        if let Ok(mut d) = self.dirty.lock() {
            d.remove(window);
        }
    }
    pub fn request(&self, window: &str, root: &str, action: Action) -> Result<serde_json::Value> {
        if matches!(action, Action::Authorize) {
            let path = Path::new(root);
            if !path.is_absolute() {
                return Err(error("project", "Select an absolute project folder."));
            }
            let canonical = fs::canonicalize(path).map_err(io)?;
            let handle = Arc::new(open_directory(&canonical)?);
            self.roots
                .lock()
                .map_err(|_| error("busy", "Editor state unavailable."))?
                .entry(window.into())
                .or_default()
                .insert(canonical.clone(), handle);
            return Ok(serde_json::json!({"root":root_label(&canonical)}));
        }
        let root = self.authorized(window, root)?;
        let _lease = self
            .roots
            .lock()
            .map_err(|_| error("busy", "Editor state unavailable."))?
            .get(window)
            .and_then(|r| r.get(&root))
            .cloned()
            .ok_or_else(|| error("project", "Project was closed."))?;
        #[cfg(target_os = "linux")]
        let root = {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!("/proc/self/fd/{}/.", _lease.as_raw_fd()))
        };
        let _guard = self
            .gate
            .lock()
            .map_err(|_| error("busy", "Editor is busy."))?;
        match action {
            Action::Authorize => unreachable!(),
            Action::List { path } => {
                let held;
                let target = if path.is_empty() {
                    root.clone()
                } else {
                    held = parent(&root, &relative(&format!("{path}/placeholder"))?)?;
                    held.path.parent().unwrap().to_path_buf()
                };
                let mut entries = Vec::new();
                let mut truncated = false;
                for (i, item) in fs::read_dir(target).map_err(io)?.enumerate() {
                    if i >= 5000 {
                        truncated = true;
                        break;
                    }
                    if let Ok(item) = item {
                        if let (Some(name), Ok(meta)) =
                            (item.file_name().to_str(), item.file_type())
                        {
                            entries.push(serde_json::json!({"name":name,"directory":meta.is_dir(),"symlink":meta.is_symlink(),"path":if path.is_empty(){name.to_owned()}else{format!("{path}/{name}")}}));
                        }
                    }
                }
                entries.sort_by_key(|e| {
                    (
                        !e["directory"].as_bool().unwrap_or(false),
                        e["name"].as_str().unwrap_or("").to_lowercase(),
                    )
                });
                Ok(serde_json::json!({"entries":entries,"truncated":truncated}))
            }
            Action::Read { path, large } => {
                let target = parent(&root, &relative(&path)?)?;
                if !large && checked_meta(&target.path)?.len() > 2 * 1024 * 1024 {
                    return Err(error(
                        "largeFile",
                        "File exceeds 2 MiB. Confirm opening with expensive features disabled.",
                    ));
                }
                serde_json::to_value(decode(&read(&target.path)?)?)
                    .map_err(|_| error("encoding", "Cannot encode file response."))
            }
            Action::Inspect { path } => {
                let target = parent(&root, &relative(&path)?)?;
                let meta = checked_meta(&target.path)?;
                if meta.is_dir() {
                    Ok(serde_json::json!({"directory":true,"bytes":0,"revision":null}))
                } else {
                    let bytes = read(&target.path)?;
                    Ok(
                        serde_json::json!({"directory":false,"bytes":bytes.len(),"revision":revision(&bytes)}),
                    )
                }
            }
            Action::Poll { path, expected } => {
                let target = parent(&root, &relative(&path)?)?;
                let bytes = read(&target.path)?;
                if revision(&bytes) == expected {
                    Ok(serde_json::Value::Null)
                } else {
                    serde_json::to_value(decode(&bytes)?)
                        .map_err(|_| error("encoding", "Cannot encode file response."))
                }
            }
            Action::Save {
                path,
                expected,
                text,
                encoding,
                bom,
            } => {
                let target = parent(&root, &relative(&path)?)?;
                let original = read(&target.path)?;
                if revision(&original) != expected {
                    return Err(error(
                        "conflict",
                        "Disk changed. Review the diff before saving.",
                    ));
                }
                let document = decode(&original)?;
                if document.encoding != encoding || document.bom != bom {
                    return Err(error("encoding", "Encoding and BOM changes are forbidden."));
                }
                let bytes = encode(&text, &encoding, bom)?;
                decode(&bytes)?;
                let permissions = checked_meta(&target.path)?.permissions();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if checked_meta(&target.path)?.nlink() > 1 {
                        return Err(error(
                            "hardlink",
                            "Hard-linked files require an explicit external save.",
                        ));
                    }
                }
                let temp = target.path.parent().unwrap().join(format!(
                    ".corpstex-save-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ));
                let result = (|| {
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&temp)
                        .map_err(io)?;
                    file.set_permissions(permissions).map_err(io)?;
                    file.write_all(&bytes).map_err(io)?;
                    file.sync_all().map_err(io)?;
                    drop(file);
                    if read(&target.path)? != original {
                        return Err(error(
                            "conflict",
                            "Disk changed during save. Buffer retained.",
                        ));
                    }
                    replace_checked(&temp, &target.path, &original, &path)?;
                    #[cfg(not(windows))]
                    if let Ok(dir) = File::open(target.path.parent().unwrap()) {
                        dir.sync_all().map_err(io)?;
                    }
                    serde_json::to_value(decode(&bytes)?)
                        .map_err(|_| error("encoding", "Cannot encode saved document."))
                })();
                if !matches!(&result,Err(e) if e.kind=="saveRace") {
                    let _ = fs::remove_file(temp);
                }
                result
            }
            Action::Create { path, directory } => {
                let target = parent(&root, &relative(&path)?)?;
                if directory {
                    fs::create_dir(&target.path).map_err(io)?;
                } else {
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target.path)
                        .map_err(io)?
                        .sync_all()
                        .map_err(io)?;
                }
                Ok(serde_json::Value::Null)
            }
            Action::Rename { path, destination } => {
                let source = parent(&root, &relative(&path)?)?;
                let dest = parent(&root, &relative(&destination)?)?;
                checked_meta(&source.path)?;
                move_no_replace(&source.path, &dest.path)?;
                Ok(serde_json::Value::Null)
            }
            Action::Duplicate { path, destination } => {
                let source = parent(&root, &relative(&path)?)?;
                let dest = parent(&root, &relative(&destination)?)?;
                let bytes = read(&source.path)?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&dest.path)
                    .map_err(io)?;
                file.write_all(&bytes).map_err(io)?;
                file.sync_all().map_err(io)?;
                Ok(serde_json::Value::Null)
            }
            Action::Delete { path, expected } => {
                let target = parent(&root, &relative(&path)?)?;
                let meta = checked_meta(&target.path)?;
                if meta.is_dir() {
                    fs::remove_dir(&target.path).map_err(io)?;
                } else {
                    if expected.as_deref() != Some(&revision(&read(&target.path)?)) {
                        return Err(error(
                            "conflict",
                            "File changed since deletion was requested.",
                        ));
                    }
                    fs::remove_file(&target.path).map_err(io)?;
                }
                Ok(serde_json::Value::Null)
            }
        }
    }
    pub fn request_exit(&self) {
        self.exit_pending.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Editor, String) {
        let root = std::env::temp_dir().join(format!(
            "corpstex-editor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let root = root.to_str().unwrap().to_owned();
        let editor = Editor::default();
        editor.request("main", &root, Action::Authorize).unwrap();
        (editor, root)
    }
    #[test]
    fn encodings_roundtrip_and_lossy_input_is_rejected() {
        for (encoding, bom) in [
            ("utf8", false),
            ("utf8", true),
            ("utf16le", true),
            ("utf16be", true),
        ] {
            let bytes = encode("héllo 🦀\r\n", encoding, bom).unwrap();
            let doc = decode(&bytes).unwrap();
            assert_eq!(encode(&doc.text, &doc.encoding, doc.bom).unwrap(), bytes);
        }
        for bytes in [
            vec![0xff],
            vec![0xff, 0xfe, 1],
            b"a\r\nb\n".to_vec(),
            b"binary\0".to_vec(),
        ] {
            assert!(decode(&bytes).is_err());
        }
    }
    #[test]
    fn scope_traversal_no_clobber_and_save_conflicts() {
        let (e, root) = fixture();
        assert!(e.authorized("other", &root).is_err());
        for path in [
            "../escape",
            "/absolute",
            "C:/windows",
            "a/../../b",
            "a\\b",
            ".git/config",
            "CON",
            "x.",
        ] {
            assert!(relative(path).is_err(), "{path}");
        }
        e.request(
            "main",
            &root,
            Action::Create {
                path: "a".into(),
                directory: false,
            },
        )
        .unwrap();
        assert!(e
            .request(
                "main",
                &root,
                Action::Create {
                    path: "a".into(),
                    directory: false
                }
            )
            .is_err());
        fs::write(Path::new(&root).join("a"), "external").unwrap();
        assert!(e
            .request(
                "main",
                &root,
                Action::Save {
                    path: "a".into(),
                    expected: revision(b""),
                    text: "mine".into(),
                    encoding: "utf8".into(),
                    bom: false
                }
            )
            .is_err());
        assert_eq!(
            fs::read_to_string(Path::new(&root).join("a")).unwrap(),
            "external"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn create_save_poll_duplicate_move_delete_and_dirty_exit() {
        let (e, root) = fixture();
        e.request(
            "main",
            &root,
            Action::Create {
                path: "folder".into(),
                directory: true,
            },
        )
        .unwrap();
        e.request(
            "main",
            &root,
            Action::Create {
                path: "a.ts".into(),
                directory: false,
            },
        )
        .unwrap();
        let saved = e
            .request(
                "main",
                &root,
                Action::Save {
                    path: "a.ts".into(),
                    expected: revision(b""),
                    text: "hello 🦀\r\n".into(),
                    encoding: "utf8".into(),
                    bom: false,
                },
            )
            .unwrap();
        assert_eq!(
            fs::read_to_string(Path::new(&root).join("a.ts")).unwrap(),
            "hello 🦀\r\n"
        );
        assert_eq!(
            e.request(
                "main",
                &root,
                Action::Poll {
                    path: "a.ts".into(),
                    expected: saved["revision"].as_str().unwrap().into()
                }
            )
            .unwrap(),
            serde_json::Value::Null
        );
        e.request(
            "main",
            &root,
            Action::Duplicate {
                path: "a.ts".into(),
                destination: "copy.ts".into(),
            },
        )
        .unwrap();
        assert!(e
            .request(
                "main",
                &root,
                Action::Rename {
                    path: "a.ts".into(),
                    destination: "copy.ts".into()
                }
            )
            .is_err());
        e.request(
            "main",
            &root,
            Action::Rename {
                path: "copy.ts".into(),
                destination: "folder/copy.ts".into(),
            },
        )
        .unwrap();
        e.request(
            "main",
            &root,
            Action::Rename {
                path: "folder".into(),
                destination: "moved".into(),
            },
        )
        .unwrap();
        assert!(
            e.request(
                "main",
                &root,
                Action::Delete {
                    path: "moved".into(),
                    expected: None
                }
            )
            .is_err(),
            "Nonempty directory must not be recursively deleted"
        );
        assert!(e
            .request(
                "main",
                &root,
                Action::Delete {
                    path: "a.ts".into(),
                    expected: Some("stale".into())
                }
            )
            .is_err());
        e.request(
            "main",
            &root,
            Action::Delete {
                path: "a.ts".into(),
                expected: Some(saved["revision"].as_str().unwrap().into()),
            },
        )
        .unwrap();
        e.dirty("main", true).unwrap();
        assert!(e.has_dirty());
        e.request_exit();
        assert!(e.exit_pending.load(Ordering::SeqCst));
        e.detach("main");
        assert!(!e.has_dirty());
        assert!(e.authorized("main", &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn atomic_replacement_preserves_a_racing_disk_version() {
        let (_e, root) = fixture();
        let path = Path::new(&root).join("race");
        let temp = Path::new(&root).join("temp");
        fs::write(&path, b"external").unwrap();
        fs::write(&temp, b"buffer").unwrap();
        let err = replace_checked(&temp, &path, b"baseline", "race").unwrap_err();
        assert_eq!(err.kind, "saveRace");
        assert_eq!(fs::read(&path).unwrap(), b"buffer");
        let retained = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(".corpstex-conflict-")
            })
            .unwrap();
        assert_eq!(fs::read(retained.path()).unwrap(), b"external");
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn symlink_escape_and_ancestor_replacement_are_rejected() {
        use std::os::unix::fs::symlink;
        let (e, root) = fixture();
        symlink("/tmp", Path::new(&root).join("escape")).unwrap();
        assert!(e
            .request(
                "main",
                &root,
                Action::Create {
                    path: "escape/outside".into(),
                    directory: false
                }
            )
            .is_err());
        fs::create_dir(Path::new(&root).join("dir")).unwrap();
        let held = parent(Path::new(&root), &relative("dir/file").unwrap()).unwrap();
        fs::rename(Path::new(&root).join("dir"), Path::new(&root).join("old")).unwrap();
        symlink("/tmp", Path::new(&root).join("dir")).unwrap();
        fs::write(&held.path, "safe").unwrap();
        assert_eq!(
            fs::read_to_string(Path::new(&root).join("old/file")).unwrap(),
            "safe"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
