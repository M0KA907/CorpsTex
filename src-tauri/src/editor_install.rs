//! Explicit, isolated npm presets. Never installs on editor startup.
use crate::editor::{error, Result};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
static BUSY: AtomicBool = AtomicBool::new(false);
pub fn busy() -> bool {
    BUSY.load(Ordering::SeqCst)
}
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub executable: String,
    pub args: Vec<String>,
    pub server_path: String,
    pub directory: String,
}
fn preset(id: &str) -> Result<(&'static str, Vec<&'static str>)> {
    match id{
    "typescript"=>Ok(("typescript-language-server",vec!["typescript-language-server","typescript"])),
    "python"=>Ok(("pyright-langserver",vec!["pyright"])),
    "json"=>Ok(("vscode-json-language-server",vec!["vscode-langservers-extracted"])),
    "yaml"=>Ok(("yaml-language-server",vec!["yaml-language-server"])),
    _=>Err(error("installer","No isolated installer for this preset. Install it with your OS package manager, then use Discover or configure its executable.")),
}
}
fn safe_directory(base: &Path, id: &str) -> Result<PathBuf> {
    preset(id)?;
    let directory = base.join("language-servers").join(id);
    // app data is trusted application-owned storage; reject redirection of managed children.
    for p in [
        base.to_owned(),
        base.join("language-servers"),
        directory.clone(),
    ] {
        if let Ok(meta) = std::fs::symlink_metadata(&p) {
            if meta.file_type().is_symlink() {
                return Err(error(
                    "installer",
                    "Managed installation path is a symlink.",
                ));
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(error(
                        "installer",
                        "Managed installation path is a reparse point.",
                    ));
                }
            }
        }
    }
    Ok(directory)
}
pub fn plan(base: &Path, id: &str) -> Result<Plan> {
    let (binary, packages) = preset(id)?;
    let directory = safe_directory(base, id)?;
    let npm=crate::editor_lsp::discover("npm").ok_or_else(||error("installer","npm is not installed. Install a language server externally or configure an existing executable."))?;
    let mut args = vec![
        "install".into(),
        "--prefix".into(),
        directory.to_string_lossy().to_string(),
        "--ignore-scripts".into(),
        "--no-audit".into(),
        "--no-fund".into(),
    ];
    args.extend(packages.into_iter().map(str::to_owned));
    let bin = directory
        .join("node_modules")
        .join(".bin")
        .join(if cfg!(windows) {
            format!("{binary}.cmd")
        } else {
            binary.into()
        });
    Ok(Plan {
        executable: npm.to_string_lossy().into(),
        args,
        server_path: bin.to_string_lossy().into(),
        directory: directory.to_string_lossy().into(),
    })
}
pub fn removal_plan(base: &Path, id: &str) -> Result<Plan> {
    let directory = safe_directory(base, id)?;
    Ok(Plan {
        executable: String::new(),
        args: Vec::new(),
        server_path: String::new(),
        directory: directory.to_string_lossy().into(),
    })
}
pub fn install(base: &Path, id: &str, approved: bool) -> Result<Plan> {
    if !approved {
        return Err(error(
            "approval",
            "Explicit installation approval is required.",
        ));
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err(error(
            "busy",
            "Another language server installation is running.",
        ));
    }
    let _guard = Guard;
    let plan = plan(base, id)?;
    run(&plan, Duration::from_secs(180))?;
    Ok(plan)
}
fn run(plan: &Plan, timeout: Duration) -> Result<()> {
    std::fs::create_dir_all(&plan.directory)
        .map_err(|_| error("installer", "Cannot create managed installation directory."))?;
    let mut command = Command::new(&plan.executable);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .args(&plan.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| error("installer", "npm could not start."))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    if !Path::new(&plan.server_path).is_file() {
                        return Err(error(
                            "installer",
                            "npm finished without the expected language server executable.",
                        ));
                    }
                    return Ok(());
                }
                return Err(error("installer","npm installation failed. Previous configuration was retained. Check network access and the managed installation folder."));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                #[cfg(unix)]
                {
                    unsafe extern "C" {
                        fn kill(pid: i32, signal: i32) -> i32;
                    }
                    unsafe {
                        kill(-(child.id() as i32), 9);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error(
                    "installer",
                    "Installation timed out or could not be monitored.",
                ));
            }
        }
    }
}
pub fn remove(base: &Path, id: &str, approved: bool) -> Result<()> {
    if !approved {
        return Err(error("approval", "Explicit removal approval is required."));
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err(error("busy", "Another installation operation is running."));
    }
    let _guard = Guard;
    let directory = safe_directory(base, id)?;
    if directory.exists() {
        std::fs::remove_dir_all(directory)
            .map_err(|_| error("installer", "Could not remove managed language server."))?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installs_and_removals_require_approval() {
        assert!(install(Path::new("/tmp/unused"), "python", false).is_err());
        assert!(remove(Path::new("/tmp/unused"), "python", false).is_err());
        assert!(preset("../../escape").is_err());
        assert!(preset("custom").is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn fixture_install_runner_timeout_and_managed_removal() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let base = std::env::temp_dir().join(format!(
            "corpstex-install-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let script = base.join("package manager.py");
        std::fs::write(&script,"#!/usr/bin/python3\nimport pathlib,sys,time\np=pathlib.Path(sys.argv[1]);p.write_text('fixture executable')\nif len(sys.argv)>2:time.sleep(10)\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let binary = base.join("server with spaces");
        let mut plan = Plan {
            executable: script.to_string_lossy().into(),
            args: vec![binary.to_string_lossy().into()],
            directory: base.to_string_lossy().into(),
            server_path: binary.to_string_lossy().into(),
        };
        run(&plan, Duration::from_secs(2)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&binary).unwrap(),
            "fixture executable"
        );
        plan.args.push("sleep".into());
        assert!(run(&plan, Duration::from_millis(100)).is_err());
        let installed = base.join("language-servers/python");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("fixture"), "installed").unwrap();
        remove(&base, "python", true).unwrap();
        assert!(!installed.exists());
        symlink("/tmp", &installed).unwrap();
        assert!(remove(&base, "python", true).is_err());
        assert!(removal_plan(&base, "python").is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}
