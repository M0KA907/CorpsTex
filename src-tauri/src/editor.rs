//! Editor-only filesystem authority. No runtime requests, inference or durable buffers.
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    hash::BuildHasher,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock, RwLock,
    },
};

const LIMIT: u64 = 32 * 1024 * 1024;
#[derive(Default)]
pub struct Editor {
    roots: Mutex<HashMap<String, Scope>>,
    next_scope: AtomicU64,
    dirty: Mutex<HashSet<String>>,
    gate: Mutex<()>,
    scope_gate: RwLock<()>,
    pub exit_pending: AtomicBool,
}
struct Scope {
    id: u64,
    project: String,
    runtime: String,
    grants: HashMap<PathBuf, Arc<File>>,
}
// Constructed only by the native bridge; never deserialized from renderer input.
pub struct GrantRequest {
    window: String,
    scope: u64,
    path: PathBuf,
    handle: Arc<File>,
    pub description: String,
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
// Canonical aliases are displayed before consent; anchor every canonical ancestor
// before showing the native dialog. Replacement cannot redirect the granted handle.
fn open_grant_directory(path: &Path) -> Result<File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let mut directory = open_directory(Path::new("/"))?;
        for component in path.components().skip(1) {
            let Component::Normal(name) = component else {
                return Err(error("path", "Invalid canonical directory."));
            };
            directory = open_directory(
                &PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name),
            )?;
        }
        Ok(directory)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(error(
            "grant",
            "Native editor grants are unavailable on this platform.",
        ))
    }
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
// Stage into a private sibling directory before checking the bytes to delete.
// Recreating the original name can no longer redirect the final unlink.
struct PendingDelete {
    target: PathBuf,
    directory: PathBuf,
    staged: PathBuf,
    retained_label: String,
    lease: File,
}
impl PendingDelete {
    fn stage(target: &Path, relative: &str) -> Result<Self> {
        let name = format!(
            ".corpstex-save-delete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let directory = target
            .parent()
            .ok_or_else(|| error("path", "Cannot delete the project root."))?
            .join(&name);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .map_err(io)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&directory).map_err(io)?;
        let lease = match open_directory(&directory) {
            Ok(lease) => lease,
            Err(e) => {
                let _ = fs::remove_dir(&directory);
                return Err(e);
            }
        };
        #[cfg(target_os = "linux")]
        let staged = {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!("/proc/self/fd/{}/file", lease.as_raw_fd()))
        };
        #[cfg(not(target_os = "linux"))]
        let staged = directory.join("file");
        if let Err(e) = move_no_replace(target, &staged) {
            drop(lease);
            let _ = fs::remove_dir(&directory);
            return Err(e);
        }
        let prefix = relative.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        Ok(Self {
            target: target.to_path_buf(),
            directory,
            staged,
            retained_label: if prefix.is_empty() {
                format!("{name}/file")
            } else {
                format!("{prefix}/{name}/file")
            },
            lease,
        })
    }

    fn finish(self, expected: &str) -> Result<()> {
        let result = (|| {
            if revision(&read(&self.staged)?) != expected {
                return Err(error(
                    "conflict",
                    "File changed since deletion was requested.",
                ));
            }
            fs::remove_file(&self.staged).map_err(io)
        })();
        if let Err(e) = result {
            // Never overwrite a file another process created at the original name.
            if move_no_replace(&self.staged, &self.target).is_err() {
                return Err(error("deleteRace", &format!(
                    "Deletion stopped. The original path could not be restored without overwriting another file. Retained content is at {}. Review it manually.",
                    self.retained_label
                )));
            }
            drop(self.lease);
            let _ = fs::remove_dir(&self.directory);
            return Err(e);
        }
        drop(self.lease);
        fs::remove_dir(&self.directory).map_err(io)
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
    pub fn context(&self, window: &str, project: &str, runtime: &str) -> Result<u64> {
        if !["codex", "opencode"].contains(&runtime)
            || (!project.is_empty() && !Path::new(project).is_absolute())
            || project.chars().any(char::is_control)
        {
            return Err(error("project", "Invalid editor context."));
        }
        let _guard = self
            .scope_gate
            .write()
            .map_err(|_| error("busy", "Editor is busy."))?;
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| error("busy", "Editor state unavailable."))?;
        if let Some(scope) = roots.get(window) {
            if scope.project == project && scope.runtime == runtime {
                return Ok(scope.id);
            }
        }
        let id = self.next_scope.fetch_add(1, Ordering::SeqCst) + 1;
        roots.insert(
            window.into(),
            Scope {
                id,
                project: project.into(),
                runtime: runtime.into(),
                grants: HashMap::new(),
            },
        );
        Ok(id)
    }
    pub fn prepare_grant(&self, window: &str, scope: u64, root: &str) -> Result<GrantRequest> {
        let roots = self
            .roots
            .lock()
            .map_err(|_| error("busy", "Editor state unavailable."))?;
        let context = roots
            .get(window)
            .filter(|s| s.id == scope)
            .ok_or_else(|| error("grant", "Editor context changed. Select the folder again."))?;
        if !Path::new(root).is_absolute() || root.len() > 4096 || root.chars().any(char::is_control)
        {
            return Err(error("project", "Select an absolute project folder."));
        }
        let path = fs::canonicalize(root).map_err(io)?;
        if root_label(&path).chars().any(char::is_control) {
            return Err(error(
                "path",
                "Control characters are not allowed in granted paths.",
            ));
        }
        let handle = Arc::new(open_grant_directory(&path)?);
        let description = format!("Allow editor read/write access to this directory?\n\n{}\n\nWindow: {}\nRuntime: {}\nProject: {}\n\nThis grant expires when the project or runtime changes, the window closes, or CorpsTex exits. Open buffers are retained; return to their project and grant access again to save them. Runtime agent permissions are unchanged.", root_label(&path), window, context.runtime, context.project);
        Ok(GrantRequest {
            window: window.into(),
            scope,
            path,
            handle,
            description,
        })
    }
    pub fn revoke(&self, window: &str, scope: u64) -> Result<u64> {
        let _guard = self
            .scope_gate
            .write()
            .map_err(|_| error("busy", "Editor is busy."))?;
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| error("busy", "Editor state unavailable."))?;
        let context = roots
            .get_mut(window)
            .filter(|s| s.id == scope)
            .ok_or_else(|| error("grant", "Editor context changed."))?;
        context.id = self.next_scope.fetch_add(1, Ordering::SeqCst) + 1;
        context.grants.clear();
        Ok(context.id)
    }
    pub fn finish_grant(&self, request: GrantRequest, confirmed: bool) -> Result<String> {
        let _guard = self
            .scope_gate
            .write()
            .map_err(|_| error("busy", "Editor is busy."))?;
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| error("busy", "Editor state unavailable."))?;
        let context = roots
            .get_mut(&request.window)
            .filter(|s| s.id == request.scope)
            .ok_or_else(|| error("grant", "Editor context changed. No access was granted."))?;
        if !confirmed {
            return Err(error(
                "grant",
                "Folder access cancelled. No access was granted.",
            ));
        }
        let label = root_label(&request.path);
        context.grants.insert(request.path, request.handle);
        Ok(label)
    }
    fn lease(&self, window: &str, scope: u64, root: &str) -> Result<(PathBuf, Arc<File>)> {
        let roots = self
            .roots
            .lock()
            .map_err(|_| error("busy", "Editor state unavailable."))?;
        roots.get(window).filter(|s| s.id == scope)
            .and_then(|s| s.grants.iter().find(|(r, _)| root_label(r) == root))
            .map(|(r, f)| (r.clone(), f.clone()))
            .ok_or_else(|| error("grant", "Folder access is not granted in this editor context. Use Grant editor access in Files. Your buffer is retained."))
    }
    pub fn with_authorized<T>(
        &self,
        window: &str,
        scope: u64,
        root: &str,
        operation: impl FnOnce(PathBuf) -> Result<T>,
    ) -> Result<T> {
        let _guard = self
            .scope_gate
            .read()
            .map_err(|_| error("busy", "Editor is busy."))?;
        let (path, _lease) = self.lease(window, scope, root)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let current = open_grant_directory(&path)?.metadata().map_err(io)?;
            let granted = _lease.metadata().map_err(io)?;
            if current.dev() != granted.dev() || current.ino() != granted.ino() {
                return Err(error(
                    "grant",
                    "Granted directory was replaced. Grant access again.",
                ));
            }
        }
        operation(path)
    }
    #[cfg(test)]
    fn authorized(&self, window: &str, root: &str) -> Result<PathBuf> {
        self.lease(window, self.test_scope(window), root)
            .map(|v| v.0)
    }
    #[cfg(test)]
    fn test_scope(&self, window: &str) -> u64 {
        self.roots
            .lock()
            .unwrap()
            .get(window)
            .map(|s| s.id)
            .unwrap_or(0)
    }
    #[cfg(test)]
    fn request(&self, window: &str, root: &str, action: Action) -> Result<serde_json::Value> {
        self.request_scoped(window, self.test_scope(window), root, action)
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
        let Ok(_guard) = self.scope_gate.write() else {
            return;
        };
        if let Ok(mut r) = self.roots.lock() {
            r.remove(window);
        }
        if let Ok(mut d) = self.dirty.lock() {
            d.remove(window);
        }
    }
    pub fn request_scoped(
        &self,
        window: &str,
        scope: u64,
        root: &str,
        action: Action,
    ) -> Result<serde_json::Value> {
        let _scope = self
            .scope_gate
            .read()
            .map_err(|_| error("busy", "Editor context unavailable."))?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| error("busy", "Editor is busy."))?;
        let (root, _lease) = self.lease(window, scope, root)?;
        if matches!(action, Action::Authorize) {
            return Ok(serde_json::json!({"root":root_label(&root)}));
        }
        #[cfg(target_os = "linux")]
        let root = {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!("/proc/self/fd/{}/.", _lease.as_raw_fd()))
        };
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
                    PendingDelete::stage(&target.path, &path)?.finish(
                        expected.as_deref().ok_or_else(|| {
                            error("conflict", "Inspect the file before deleting it.")
                        })?,
                    )?;
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
    #[test]
    #[cfg(target_os = "linux")]
    fn renderer_paths_and_confirmation_flags_do_not_create_grants() {
        let (e, root) = fixture();
        let scope = e.context("ungranted", &root, "codex").unwrap();
        for action in [
            Action::Authorize,
            Action::List {
                path: String::new(),
            },
            Action::Read {
                path: "a".into(),
                large: false,
            },
            Action::Create {
                path: "a".into(),
                directory: false,
            },
        ] {
            assert!(e.request_scoped("ungranted", scope, &root, action).is_err());
        }
        // Even preparing native consent and a renderer-supplied "approved" field
        // cannot authorize. Only the native response completion can add a grant.
        let request = e.prepare_grant("ungranted", scope, &root).unwrap();
        let action: Action =
            serde_json::from_value(serde_json::json!({"type":"authorize","approved":true}))
                .unwrap();
        assert!(e.request_scoped("ungranted", scope, &root, action).is_err());
        assert!(e.finish_grant(request, false).is_err());
        assert!(e
            .request_scoped("ungranted", scope, &root, Action::Authorize)
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn grants_are_scoped_and_revocation_rejects_stale_native_confirmations() {
        let (e, root) = fixture();
        let first = e.test_scope("main");
        assert!(e
            .request_scoped("other", first, &root, Action::Authorize)
            .is_err());
        let pending = e.prepare_grant("main", first, &root).unwrap();
        let second = e.context("main", &root, "opencode").unwrap();
        assert_ne!(first, second);
        assert!(e.finish_grant(pending, true).is_err());
        assert!(e
            .request_scoped("main", first, &root, Action::Authorize)
            .is_err());
        assert!(e
            .request_scoped("main", second, &root, Action::Authorize)
            .is_err());
        let grant = e.prepare_grant("main", second, &root).unwrap();
        e.finish_grant(grant, true).unwrap();
        assert!(e.with_authorized("main", second, &root, |_| Ok(())).is_ok());
        assert_eq!(e.context("main", &root, "opencode").unwrap(), second);
        let pending = e.prepare_grant("main", second, &root).unwrap();
        let revoked = e.revoke("main", second).unwrap();
        assert!(e.finish_grant(pending, true).is_err());
        assert!(e
            .with_authorized("main", revoked, &root, |_| Ok(()))
            .is_err());
        let pending = e.prepare_grant("main", revoked, &root).unwrap();
        let other = format!("{root}/other-project");
        let changed = e.context("main", &other, "opencode").unwrap();
        assert!(e.finish_grant(pending, true).is_err());
        let pending = e.prepare_grant("main", changed, &root).unwrap();
        e.detach("main");
        let reopened = e.context("main", &other, "opencode").unwrap();
        assert_ne!(changed, reopened);
        assert!(e.finish_grant(pending, true).is_err());
        assert!(e
            .with_authorized("main", reopened, &root, |_| Ok(()))
            .is_err());
        let fresh = Editor::default();
        let scope = fresh.context("main", &root, "codex").unwrap();
        assert!(fresh
            .request_scoped("main", scope, &root, Action::Authorize)
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn additional_grants_aliases_and_replacements_keep_held_directory_identity() {
        use std::os::unix::fs::symlink;
        let (e, root) = fixture();
        let scope = e.test_scope("main");
        let extra = Path::new(&root).join("extra");
        fs::create_dir(&extra).unwrap();
        fs::write(extra.join("a"), b"original").unwrap();
        let alias = Path::new(&root).join("alias");
        symlink(&extra, &alias).unwrap();
        let request = e
            .prepare_grant("main", scope, alias.to_str().unwrap())
            .unwrap();
        assert_eq!(request.path, extra);
        assert!(request.description.contains(extra.to_str().unwrap()));
        let saved = Path::new(&root).join("saved");
        fs::rename(&extra, &saved).unwrap();
        fs::create_dir(&extra).unwrap();
        fs::write(extra.join("a"), b"replacement").unwrap();
        let canonical = e.finish_grant(request, true).unwrap();
        let doc = e
            .request_scoped(
                "main",
                scope,
                &canonical,
                Action::Read {
                    path: "a".into(),
                    large: false,
                },
            )
            .unwrap();
        assert_eq!(doc["text"], "original");
        assert!(
            e.with_authorized("main", scope, &canonical, |_| Ok(()))
                .is_err(),
            "LSP must not start in a substituted path"
        );
        assert!(
            e.request_scoped("main", scope, &root, Action::Authorize)
                .is_ok(),
            "extra grant preserves project grant"
        );
        fs::remove_dir_all(root).unwrap();
    }
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
        let scope = editor.context("main", &root, "codex").unwrap();
        let grant = editor.prepare_grant("main", scope, &root).unwrap();
        editor.finish_grant(grant, true).unwrap();
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
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn deletion_validates_staged_bytes_and_never_unlinks_a_recreated_path() {
        let (_e, root) = fixture();
        let path = Path::new(&root).join("race");
        fs::write(&path, b"reviewed").unwrap();
        let pending = PendingDelete::stage(&path, "race").unwrap();
        // A replacement after staging must survive even a successful deletion.
        fs::write(&path, b"replacement").unwrap();
        pending.finish(&revision(b"reviewed")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");

        // A replacement between the initial check and staging is restored.
        let pending = PendingDelete::stage(&path, "race").unwrap();
        assert_eq!(
            pending.finish(&revision(b"reviewed")).unwrap_err().kind,
            "conflict"
        );
        assert_eq!(fs::read(&path).unwrap(), b"replacement");

        let pending = PendingDelete::stage(&path, "race").unwrap();
        let retained = Path::new(&root).join(&pending.retained_label);
        let label = pending.retained_label.clone();
        // An open writer can change the staged inode before validation.
        fs::write(&retained, b"changed").unwrap();
        fs::write(&path, b"new occupant").unwrap();
        let error = pending.finish(&revision(b"replacement")).unwrap_err();
        assert_eq!(error.kind, "deleteRace");
        assert!(error.message.contains(&label));
        assert_eq!(fs::read(&retained).unwrap(), b"changed");
        assert_eq!(fs::read(&path).unwrap(), b"new occupant");
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
