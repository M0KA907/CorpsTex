//! Read-only directory metadata for the explicit in-app project picker.
use crate::runtime::Error;
use serde::Serialize;
use std::{fs, path::PathBuf, time::UNIX_EPOCH};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    name: String,
    path: String,
    is_dir: bool,
    size: Option<u64>,
    modified: Option<u64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    path: String,
    parent: Option<String>,
    home: Option<String>,
    roots: Vec<String>,
    entries: Vec<Entry>,
    truncated: bool,
    skipped: usize,
}
pub fn browse(path: Option<String>, show_hidden: bool) -> Result<Listing, Error> {
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let target = path
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| home.clone())
        .or_else(|| std::env::current_dir().ok())
        .ok_or(Error::InvalidProject)?;
    if !target.is_absolute() {
        return Err(Error::InvalidProject);
    }
    let target = fs::canonicalize(target).map_err(|_| Error::InvalidProject)?;
    let directory = fs::read_dir(&target).map_err(|_| Error::InvalidProject)?;
    let mut entries = Vec::new();
    let mut skipped = 0;
    let mut truncated = false;
    // Bound work and output even for enormous directories. No file content is opened.
    for (index, result) in directory.enumerate() {
        if index >= 5000 {
            truncated = true;
            break;
        }
        let Ok(entry) = result else {
            skipped += 1;
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            skipped += 1;
            continue;
        };
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            skipped += 1;
            continue;
        };
        let Some(path) = entry.path().to_str().map(str::to_owned) else {
            skipped += 1;
            continue;
        };
        entries.push(Entry {
            name,
            path,
            is_dir: meta.is_dir(),
            size: if meta.is_file() {
                Some(meta.len())
            } else {
                None
            },
            modified: meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs()),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    #[cfg(windows)]
    let roots = ('A'..='Z')
        .map(|letter| format!("{letter}:\\"))
        .filter(|path| std::path::Path::new(path).is_dir())
        .collect();
    #[cfg(not(windows))]
    let roots = vec!["/".to_owned()];
    Ok(Listing {
        path: target.to_str().ok_or(Error::InvalidProject)?.into(),
        parent: target
            .parent()
            .and_then(|path| path.to_str())
            .map(str::to_owned),
        home: home.and_then(|path| path.to_str().map(str::to_owned)),
        roots,
        entries,
        truncated,
        skipped,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_picker_lists_metadata_and_rejects_files_and_relative_paths() {
        let root = std::env::temp_dir().join(format!("corpstex-picker-{}", std::process::id()));
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::write(root.join("file.txt"), b"fixture").unwrap();
        fs::write(root.join(".hidden"), b"hidden").unwrap();
        let listing = browse(Some(root.to_str().unwrap().into()), false).unwrap();
        assert!(listing.entries[0].is_dir);
        assert_eq!(
            listing
                .entries
                .iter()
                .find(|entry| entry.name == "file.txt")
                .unwrap()
                .size,
            Some(7)
        );
        assert!(!listing.entries.iter().any(|entry| entry.name == ".hidden"));
        assert!(browse(Some(root.to_str().unwrap().into()), true)
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.name == ".hidden"));
        assert!(browse(Some(root.join("file.txt").to_str().unwrap().into()), false).is_err());
        assert!(browse(Some("relative".into()), false).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
