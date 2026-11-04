//! User-submitted keys go to the installed runtime's official stdin import.
//! Never read/export credentials, write our own credential file, or log payloads.
use crate::runtime::{self, Error, Process};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
static SAVING: AtomicBool = AtomicBool::new(false);
pub struct Saving;
impl Drop for Saving {
    fn drop(&mut self) {
        SAVING.store(false, Ordering::SeqCst);
    }
}
pub fn begin() -> Result<Saving, Error> {
    SAVING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| Error::OpenCodeKeyBusy)?;
    Ok(Saving)
}
pub fn saving() -> bool {
    SAVING.load(Ordering::SeqCst)
}

fn valid_provider(provider: &str) -> bool {
    !provider.is_empty()
        && provider.len() <= 80
        && provider
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
        && provider.as_bytes()[0].is_ascii_alphanumeric()
}

fn save_with(
    provider: &str,
    key: &str,
    id: &str,
    mut run: impl FnMut(&[&str], Option<&Value>) -> Result<bool, Error>,
) -> Result<(), Error> {
    let key = key.trim();
    if !valid_provider(provider)
        || key.is_empty()
        || key.len() > 16 * 1024
        || key.chars().any(char::is_whitespace)
        || key.chars().any(char::is_control)
    {
        return Err(Error::InvalidRequest);
    }
    // A new account preserves existing accounts. OpenCode owns storage and activation.
    let payload = json!([{"id":id,"integrationID":provider,"label":"CorpsTex","active":true,"value":{"type":"key","key":key}}]);
    if !run(&["auth", "import", "--standalone"], Some(&payload))
        .map_err(|_| Error::OpenCodeKeySave)?
    {
        return Err(Error::OpenCodeKeySave);
    }
    if !run(&["auth", "switch", provider, id, "--standalone"], None)
        .map_err(|_| Error::OpenCodeKeyActivation)?
    {
        return Err(Error::OpenCodeKeyActivation);
    }
    Ok(())
}

pub fn save(provider: String, key: String, _guard: Saving) -> Result<(), Error> {
    let path = std::env::var_os("PATH").ok_or(Error::OpenCodeUnavailable)?;
    let executable =
        runtime::discover_named(&path, "opencode").map_err(|_| Error::OpenCodeUnavailable)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Transport)?
        .as_nanos();
    let id = format!(
        "cred_corpstex_{}_{}_{}",
        std::process::id(),
        nanos,
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    save_with(&provider, &key, &id, |args, payload| {
        run(&executable, args, payload)
    })
}

fn run(executable: &Path, args: &[&str], payload: Option<&Value>) -> Result<bool, Error> {
    let mut process = Process::spawn_at(executable, args, None)?;
    if let Some(payload) = payload {
        process.send(payload)?;
    }
    process.finish_input()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_only_enter_stdin_and_activation_preserves_existing_accounts() {
        let mut stages = 0;
        save_with(
            "openrouter",
            "fixture-key-do-not-log",
            "cred_fixture",
            |args, payload| {
                assert!(!args.join(" ").contains("fixture-key-do-not-log"));
                if stages == 0 {
                    assert_eq!(args, ["auth", "import", "--standalone"]);
                    let p = payload.unwrap();
                    assert_eq!(p[0]["value"]["type"], "key");
                    assert_eq!(p[0]["value"]["key"], "fixture-key-do-not-log");
                    assert_eq!(p[0]["integrationID"], "openrouter");
                } else {
                    assert_eq!(
                        args,
                        [
                            "auth",
                            "switch",
                            "openrouter",
                            "cred_fixture",
                            "--standalone"
                        ]
                    );
                    assert!(payload.is_none());
                }
                stages += 1;
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(stages, 2);
    }
    #[test]
    fn rejects_invalid_input_and_reports_partial_save_without_retry() {
        for (provider, key) in [
            ("--server", "key"),
            ("../provider", "key"),
            ("openai", ""),
            ("openai", "bad\nkey"),
        ] {
            assert_eq!(
                save_with(provider, key, "id", |_, _| panic!("must not spawn")),
                Err(Error::InvalidRequest)
            );
        }
        let mut stages = 0;
        assert_eq!(
            save_with("openai", "fixture", "id", |_, _| {
                stages += 1;
                Ok(stages == 1)
            }),
            Err(Error::OpenCodeKeyActivation)
        );
        assert_eq!(stages, 2);
        let mut stages = 0;
        assert_eq!(
            save_with("openai", "fixture", "id", |_, _| {
                stages += 1;
                Ok(false)
            }),
            Err(Error::OpenCodeKeySave)
        );
        assert_eq!(stages, 1);
    }
}

#[cfg(all(test, unix))]
#[test]
fn official_command_input_is_piped_and_all_diagnostics_are_discarded() {
    let payload = json!([{"id":"fixture","value":{"type":"key","key":"fixture-only"}}]);
    let script="import sys,json; value=json.load(sys.stdin); sys.stdout.write('fixture diagnostic\\n'); sys.stderr.write('fixture diagnostic\\n'); sys.exit(0 if value[0]['value']['key']=='fixture-only' else 1)";
    assert!(run(
        Path::new("/usr/bin/python3"),
        &["-u", "-c", script],
        Some(&payload)
    )
    .unwrap());
}

#[cfg(test)]
#[test]
fn key_save_guard_serializes_submissions_and_blocks_exit_until_done() {
    let guard = begin().unwrap();
    assert!(saving());
    assert!(matches!(begin(), Err(Error::OpenCodeKeyBusy)));
    drop(guard);
    assert!(!saving());
}
