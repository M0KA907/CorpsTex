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
    pub packages: Vec<String>,
    pub installed_packages: Vec<String>,
    pub previous_packages: Vec<String>,
}
fn preset(id: &str) -> Result<(&'static str, &'static str)> {
    match id {
        "typescript" => Ok(("typescript-language-server", include_str!("../lsp-locks/typescript.json"))),
        "python" => Ok(("pyright-langserver", include_str!("../lsp-locks/python.json"))),
        "json" => Ok(("vscode-json-language-server", include_str!("../lsp-locks/json.json"))),
        "yaml" => Ok(("yaml-language-server", include_str!("../lsp-locks/yaml.json"))),
        _ => Err(error("installer", "No isolated installer for this preset. Install it with your OS package manager, then use Discover or configure its executable.")),
    }
}
fn versions(manifest: &serde_json::Value) -> Vec<String> {
    manifest["dependencies"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, version)| {
            let version = version.as_str()?;
            let parts: Vec<_> = version.split('.').collect();
            (parts.len() == 3
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
            .then(|| format!("{name}@{version}"))
        })
        .collect()
}
fn installed_versions(directory: &Path) -> Vec<String> {
    let manifest_path = directory.join("package.json");
    let manifest = std::fs::symlink_metadata(&manifest_path)
        .ok()
        .filter(|meta| meta.is_file() && meta.len() < 64 * 1024)
        .and_then(|_| std::fs::read(manifest_path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    [
        "typescript-language-server",
        "typescript",
        "pyright",
        "vscode-langservers-extracted",
        "yaml-language-server",
    ]
    .into_iter()
    .filter_map(|name| {
        manifest.as_ref()?["dependencies"].get(name)?;
        let path = directory
            .join("node_modules")
            .join(name)
            .join("package.json");
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > 64 * 1024 {
            return None;
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let version = manifest["version"].as_str()?;
        let declared = serde_json::json!({"dependencies":{name:version}});
        versions(&declared).into_iter().next()
    })
    .collect()
}
fn previous_directory(base: &Path, id: &str) -> Result<PathBuf> {
    safe_directory(base, id)?;
    let path = base.join("language-servers").join(format!("{id}.previous"));
    check_directory(&path)?;
    Ok(path)
}
fn check_directory(path: &Path) -> Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
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
    Ok(())
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
        check_directory(&p)?;
    }
    Ok(directory)
}
pub fn plan(base: &Path, id: &str) -> Result<Plan> {
    let (binary, lock) = preset(id)?;
    let lock: serde_json::Value =
        serde_json::from_str(lock).map_err(|_| error("installer", "Invalid preset lock."))?;
    let packages = versions(&lock["packages"][""]);
    let directory = safe_directory(base, id)?;
    let npm=crate::editor_lsp::discover("npm").ok_or_else(||error("installer","npm is not installed. Install a language server externally or configure an existing executable."))?;
    let args = vec![
        "ci".into(),
        "--prefix".into(),
        directory.to_string_lossy().to_string(),
        "--ignore-scripts".into(),
        "--engine-strict".into(),
        "--no-audit".into(),
        "--no-fund".into(),
    ];
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
        packages,
        installed_packages: installed_versions(&directory),
        previous_packages: installed_versions(&previous_directory(base, id)?),
    })
}
pub fn removal_plan(base: &Path, id: &str) -> Result<Plan> {
    let directory = safe_directory(base, id)?;
    Ok(Plan {
        executable: String::new(),
        args: Vec::new(),
        server_path: String::new(),
        directory: directory.to_string_lossy().into(),
        packages: Vec::new(),
        installed_packages: installed_versions(&directory),
        previous_packages: installed_versions(&previous_directory(base, id)?),
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
    install_with_plan(base, id, plan)
}
fn install_with_plan(base: &Path, id: &str, mut plan: Plan) -> Result<Plan> {
    let directory = Path::new(&plan.directory);
    let stage = directory.with_file_name(format!(
        ".{id}-stage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(directory.parent().unwrap())
        .map_err(|_| error("installer", "Cannot create managed directory."))?;
    std::fs::create_dir(&stage)
        .map_err(|_| error("installer", "Cannot create staging directory."))?;
    let outcome = (|| {
        let (_, lock) = preset(id)?;
        let parsed: serde_json::Value =
            serde_json::from_str(lock).map_err(|_| error("installer", "Invalid preset lock."))?;
        std::fs::write(
            stage.join("package.json"),
            serde_json::to_vec(&parsed["packages"][""]).unwrap(),
        )
        .map_err(|_| error("installer", "Cannot write staged manifest."))?;
        std::fs::write(stage.join("package-lock.json"), lock)
            .map_err(|_| error("installer", "Cannot write staged lock."))?;
        let staged = Plan {
            args: plan
                .args
                .iter()
                .map(|arg| {
                    if arg == &plan.directory {
                        stage.to_string_lossy().into_owned()
                    } else {
                        arg.clone()
                    }
                })
                .collect(),
            directory: stage.to_string_lossy().into(),
            server_path: stage
                .join(
                    Path::new(&plan.server_path)
                        .strip_prefix(directory)
                        .unwrap(),
                )
                .to_string_lossy()
                .into(),
            executable: plan.executable.clone(),
            packages: plan.packages.clone(),
            installed_packages: vec![],
            previous_packages: vec![],
        };
        run(&staged, Duration::from_secs(180))?;
        let mut installed = installed_versions(&stage);
        let mut expected = plan.packages.clone();
        installed.sort();
        expected.sort();
        if installed != expected
            || std::fs::read_to_string(stage.join("package-lock.json"))
                .ok()
                .as_deref()
                != Some(lock)
        {
            return Err(error("installer", "Staged package versions or lock do not match the approved preset. Previous installation retained."));
        }
        activate(base, id, &stage)?;
        Ok(())
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    outcome?;
    plan.installed_packages = installed_versions(directory);
    plan.previous_packages = installed_versions(&previous_directory(base, id)?);
    Ok(plan)
}
fn activate(base: &Path, id: &str, stage: &Path) -> Result<()> {
    check_directory(stage)?;
    if !stage.is_dir() {
        return Err(error(
            "installer",
            "Staging directory is missing. Current installation retained.",
        ));
    }
    let directory = safe_directory(base, id)?;
    let previous = previous_directory(base, id)?;
    let had_current = directory.exists();
    if had_current {
        if previous.exists() {
            std::fs::remove_dir_all(&previous).map_err(|_| {
                error(
                    "installer",
                    "Could not retire older rollback installation; current server retained.",
                )
            })?;
        }
        std::fs::rename(&directory, &previous).map_err(|_| {
            error(
                "installer",
                "Could not retain previous installation; current server retained.",
            )
        })?;
    }
    if std::fs::rename(stage, &directory).is_err() {
        if had_current && std::fs::rename(&previous, &directory).is_err() {
            return Err(error("installer", "Activation failed; previous installation remains in the .previous directory. Restore it before starting."));
        }
        return Err(error(
            "installer",
            "Activation failed; previous installation restored.",
        ));
    }
    Ok(())
}
pub fn rollback_plan(base: &Path, id: &str) -> Result<Plan> {
    let mut plan = removal_plan(base, id)?;
    let (binary, _) = preset(id)?;
    let previous = previous_directory(base, id)?;
    let relative = Path::new("node_modules")
        .join(".bin")
        .join(if cfg!(windows) {
            format!("{binary}.cmd")
        } else {
            binary.into()
        });
    if !previous.join(&relative).is_file() {
        return Err(error(
            "installer",
            "No previous managed installation is available.",
        ));
    }
    plan.server_path = Path::new(&plan.directory)
        .join(relative)
        .to_string_lossy()
        .into();
    plan.packages = plan.previous_packages.clone();
    Ok(plan)
}
pub fn rollback(base: &Path, id: &str, approved: bool) -> Result<Plan> {
    if !approved {
        return Err(error("approval", "Explicit rollback approval is required."));
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err(error("busy", "Another installation operation is running."));
    }
    let _guard = Guard;
    let mut plan = rollback_plan(base, id)?;
    let directory = safe_directory(base, id)?;
    let previous = previous_directory(base, id)?;
    let swap = directory.with_file_name(format!(".{id}-rollback"));
    check_directory(&swap)?;
    if swap.exists() {
        return Err(error(
            "installer",
            "A prior rollback needs recovery. Review the managed folder.",
        ));
    }
    let had_current = directory.exists();
    if had_current {
        std::fs::rename(&directory, &swap).map_err(|_| {
            error(
                "installer",
                "Cannot stage rollback; current server retained.",
            )
        })?;
    }
    if std::fs::rename(&previous, &directory).is_err() {
        if had_current {
            let _ = std::fs::rename(&swap, &directory);
        }
        return Err(error(
            "installer",
            "Cannot restore previous installation. Review the managed folder.",
        ));
    }
    if had_current && std::fs::rename(&swap, &previous).is_err() {
        return Err(error("installer", "Rollback restored; newer installation remains in the .*-rollback folder. Review it before another operation."));
    }
    plan.installed_packages = installed_versions(&directory);
    plan.previous_packages = installed_versions(&previous);
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
    let previous = previous_directory(base, id)?;
    if previous.exists() {
        std::fs::remove_dir_all(&previous)
            .map_err(|_| error("installer", "Could not remove previous managed server."))?;
    }
    if directory.exists() {
        std::fs::remove_dir_all(directory)
            .map_err(|_| error("installer", "Could not remove managed language server."))?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    static TEST_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    #[test]
    #[ignore = "explicit smoke against temporary preinstalled locked presets; no downloads or inference"]
    fn managed_presets_initialize_with_the_editor_transport() {
        use crate::editor_lsp::{Configuration, Servers};
        let installed = PathBuf::from(
            std::env::var("CORPSTEX_MANAGED_SMOKE_BASE")
                .expect("temporary preset installation root"),
        );
        let root =
            std::env::temp_dir().join(format!("corpstex-managed-lsp-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let servers = Servers::default();
        for id in ["typescript", "python", "json", "yaml"] {
            let (binary, lock) = preset(id).unwrap();
            let lock: serde_json::Value = serde_json::from_str(lock).unwrap();
            let mut expected = versions(&lock["packages"][""]);
            let mut installed_packages = installed_versions(&installed.join(id));
            expected.sort();
            installed_packages.sort();
            assert_eq!(installed_packages, expected);
            let executable = installed.join(id).join("node_modules/.bin").join(binary);
            let result = servers
                .start(
                    "managed-smoke",
                    &root,
                    id,
                    Configuration {
                        executable: executable.to_string_lossy().into(),
                        args: vec!["--stdio".into()],
                    },
                )
                .unwrap();
            assert!(result["capabilities"].is_object());
            eprintln!(
                "{id}: initialize/initialized passed, packages={:?}",
                installed_versions(&installed.join(id))
            );
            servers.stop("managed-smoke", root.to_str().unwrap(), id);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn every_preset_has_an_exact_integrity_locked_graph() {
        for id in ["typescript", "python", "json", "yaml"] {
            let (_, lock) = preset(id).unwrap();
            let lock: serde_json::Value = serde_json::from_str(lock).unwrap();
            let packages = lock["packages"].as_object().unwrap();
            assert!(!versions(&packages[""]).is_empty());
            for (path, package) in packages.iter().filter(|(path, _)| !path.is_empty()) {
                assert!(package["version"].is_string(), "{id}: {path}");
                assert!(package["integrity"]
                    .as_str()
                    .unwrap()
                    .starts_with("sha512-"));
                assert!(package["resolved"]
                    .as_str()
                    .unwrap()
                    .starts_with("https://registry.npmjs.org/"));
            }
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn staged_updates_keep_previous_server_and_allow_explicit_rollback() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = TEST_GATE.lock().unwrap();
        let base =
            std::env::temp_dir().join(format!("corpstex-staged-install-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let script = base.join("fixture-npm");
        std::fs::write(&script, r#"#!/usr/bin/python3
import json,pathlib,sys
p=pathlib.Path(sys.argv[sys.argv.index('--prefix')+1])
assert sys.argv[1]=='ci' and '--ignore-scripts' in sys.argv and '--engine-strict' in sys.argv
manifest=json.loads((p/'package.json').read_text());lock=json.loads((p/'package-lock.json').read_text())
assert manifest['dependencies']==lock['packages']['']['dependencies']
(p/'partial').write_text('staging only')
if '--fixture-fail' in sys.argv:sys.exit(1)
b=p/'node_modules/.bin/pyright-langserver';b.parent.mkdir(parents=True);b.write_text('new executable')
package=p/'node_modules/pyright/package.json';package.parent.mkdir(parents=True);package.write_text(json.dumps(dict(name='pyright',version=manifest['dependencies']['pyright'])))
"#).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let current = base.join("language-servers/python");
        std::fs::create_dir_all(current.join("node_modules/.bin")).unwrap();
        let binary = current.join("node_modules/.bin/pyright-langserver");
        std::fs::write(&binary, "old executable").unwrap();
        std::fs::create_dir_all(current.join("node_modules/pyright")).unwrap();
        std::fs::write(
            current.join("node_modules/pyright/package.json"),
            r#"{"name":"pyright","version":"1.1.400"}"#,
        )
        .unwrap();
        std::fs::write(
            current.join("package.json"),
            r#"{"dependencies":{"pyright":"1.1.400"}}"#,
        )
        .unwrap();
        let make_plan = |fail| {
            let mut plan = plan(&base, "python").unwrap();
            plan.executable = script.to_string_lossy().into();
            if fail {
                plan.args.push("--fixture-fail".into());
            }
            plan
        };
        assert!(install_with_plan(&base, "python", make_plan(true)).is_err());
        assert_eq!(std::fs::read_to_string(&binary).unwrap(), "old executable");
        assert!(!current.join("partial").exists());
        assert_eq!(
            std::fs::read_dir(current.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        let installed = install_with_plan(&base, "python", make_plan(false)).unwrap();
        assert_eq!(installed.installed_packages, ["pyright@1.1.414"]);
        assert_eq!(installed.previous_packages, ["pyright@1.1.400"]);
        assert_eq!(std::fs::read_to_string(&binary).unwrap(), "new executable");
        assert!(rollback(&base, "python", false).is_err());
        rollback(&base, "python", true).unwrap();
        assert_eq!(std::fs::read_to_string(&binary).unwrap(), "old executable");
        rollback(&base, "python", true).unwrap();
        assert_eq!(std::fs::read_to_string(&binary).unwrap(), "new executable");
        // A failed activation restores the current directory from its retained copy.
        assert!(activate(&base, "python", &base.join("missing-stage")).is_err());
        assert_eq!(std::fs::read_to_string(&binary).unwrap(), "new executable");
        remove(&base, "python", true).unwrap();
        assert!(!current.exists());
        assert!(!base.join("language-servers/python.previous").exists());
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn installs_and_removals_require_approval() {
        let _serial = TEST_GATE.lock().unwrap();
        assert!(rollback(Path::new("/tmp/unused"), "python", false).is_err());
        assert!(install(Path::new("/tmp/unused"), "python", false).is_err());
        assert!(remove(Path::new("/tmp/unused"), "python", false).is_err());
        assert!(preset("../../escape").is_err());
        assert!(preset("custom").is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn fixture_install_runner_timeout_and_managed_removal() {
        let _serial = TEST_GATE.lock().unwrap();
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
            packages: vec![],
            installed_packages: vec![],
            previous_packages: vec![],
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
