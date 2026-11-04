//! Shared stdio transport and installed-runtime handshake. No shell or credential overrides.
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

// Image/tool history can legitimately exceed 1 MiB in a single JSONL frame.
// Keep a finite per-frame bound, independent of the initialization queue budget.
const LIMIT: usize = 16 * 1024 * 1024;
const INITIAL_NOTIFICATION_LIMIT: usize = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
type WriteRequest = (Vec<u8>, mpsc::SyncSender<Result<(), Error>>);

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum Error {
    InvalidRequest,
    OpenCodeUnavailable,
    OpenCodeUnsupported,
    OpenCodeAuthRequired,
    OpenCodeKeyBusy,
    OpenCodeKeySave,
    OpenCodeKeyActivation,
    InvalidProject,
    NotSubscribed,
    StaleGeneration,
    Busy,
    ExitPending,
    ConfirmationRequired,
    MissingExecutable,
    UnsupportedVersion,
    Spawn,
    Transport,
    MalformedMessage,
    MessageTooLarge,
    UnsupportedProtocol,
    ServerExit,
    Timeout,
    Cancelled,
    Remote { code: i64 },
}

#[cfg(test)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    executable: String,
    version: String,
    handshake: bool,
    notification_count: usize,
}

// Search only absolute PATH entries, never the project directory or a shell.
fn discover_in(path: &std::ffi::OsStr) -> Result<PathBuf, Error> {
    discover_named(path, "codex")
}

pub(crate) fn discover_named(path: &std::ffi::OsStr, name: &str) -> Result<PathBuf, Error> {
    for directory in env::split_paths(path).filter(|p| p.is_absolute()) {
        let executable = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        };
        {
            let candidate = directory.join(executable);
            if let Ok(metadata) = fs::metadata(&candidate) {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o111 == 0 {
                        continue;
                    }
                }
                if metadata.is_file() {
                    return fs::canonicalize(candidate).map_err(|_| Error::MissingExecutable);
                }
            }
        }
    }
    Err(Error::MissingExecutable)
}

pub(crate) struct Process {
    child: Child,
    notifications: VecDeque<String>,
    messages: mpsc::Receiver<Result<String, Error>>,
    writer: Option<mpsc::SyncSender<WriteRequest>>,
    workers: Vec<thread::JoinHandle<()>>,
}

fn read_lines(reader: impl Read, sender: mpsc::SyncSender<Result<String, Error>>) {
    let mut reader = BufReader::new(reader);
    // Bounded channel applies backpressure; a live project has no message-count limit.
    loop {
        let mut bytes = Vec::new();
        let result = reader
            .by_ref()
            .take((LIMIT + 1) as u64)
            .read_until(b'\n', &mut bytes);
        let message = match result {
            Ok(0) => return,
            Ok(_) if bytes.len() > LIMIT => Err(Error::MessageTooLarge),
            Ok(_) if !bytes.ends_with(b"\n") => Err(Error::MalformedMessage),
            Ok(_) => String::from_utf8(bytes).map_err(|_| Error::MalformedMessage),
            Err(_) => Err(Error::Transport),
        };
        let failed = message.is_err();
        if sender.send(message).is_err() || failed {
            return;
        }
    }
}

impl Process {
    fn spawn(executable: &Path, args: &[&str]) -> Result<Self, Error> {
        Self::spawn_at(executable, args, None)
    }

    pub(crate) fn spawn_at(
        executable: &Path,
        args: &[&str],
        cwd: Option<&Path>,
    ) -> Result<Self, Error> {
        let mut command = Command::new(executable);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|_| Error::Spawn)?;
        let stdout = child.stdout.take().ok_or(Error::Transport)?;
        let stderr = child.stderr.take().ok_or(Error::Transport)?;
        let mut stdin = child.stdin.take().ok_or(Error::Transport)?;
        let (sender, messages) = mpsc::sync_channel(32);
        let (writer, writes) = mpsc::sync_channel::<WriteRequest>(1);
        let workers = vec![
            thread::spawn(move || {
                while let Ok((bytes, response)) = writes.recv() {
                    let result = stdin
                        .write_all(&bytes)
                        .and_then(|_| stdin.flush())
                        .map_err(|_| Error::Transport);
                    let failed = result.is_err();
                    let _ = response.send(result);
                    if failed {
                        break;
                    }
                }
            }),
            thread::spawn(move || read_lines(stdout, sender)),
            // stderr is never parsed as protocol or logged (it can contain secrets).
            thread::spawn(move || {
                let _ = std::io::copy(&mut BufReader::new(stderr), &mut std::io::sink());
            }),
        ];
        Ok(Self {
            child,
            notifications: VecDeque::new(),
            messages,
            writer: Some(writer),
            workers,
        })
    }

    pub(crate) fn send(&mut self, value: &Value) -> Result<(), Error> {
        self.send_with_timeout(value, WRITE_TIMEOUT)
    }

    fn send_with_timeout(&mut self, value: &Value, timeout: Duration) -> Result<(), Error> {
        let mut bytes = serde_json::to_vec(value).map_err(|_| Error::Transport)?;
        bytes.push(b'\n');
        let (response, result) = mpsc::sync_channel(1);
        self.writer
            .as_ref()
            .ok_or(Error::Transport)?
            .try_send((bytes, response))
            .map_err(|_| Error::Transport)?;
        match result.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(()),
            _ => {
                // A partial write has an unknown outcome. Stop this connection and
                // never retry it; killing the child also unblocks its stdin writer.
                self.writer.take();
                let _ = self.child.kill();
                Err(Error::Transport)
            }
        }
    }

    fn receive(&mut self, deadline: Instant, cancelled: &AtomicBool) -> Result<String, Error> {
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            match self.messages.recv_timeout(Duration::from_millis(20)) {
                Ok(message) => return message,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Error::ServerExit),
                // EOF comes from the reader after all final stdout has been queued.
                // Polling child exit here could race that reader and lose a response.
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }
    }

    pub(crate) fn pid(&self) -> u32 {
        self.child.id()
    }

    // Credential commands read only stdin. Discard every byte of their output,
    // including failures, because a provider may include secrets in diagnostics.
    pub(crate) fn finish_input(&mut self) -> Result<bool, Error> {
        self.writer.take();
        let deadline = Instant::now() + Duration::from_secs(55);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|_| Error::Transport)? {
                return Ok(status.success());
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            match self.messages.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(_)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
            }
        }
    }

    pub(crate) fn poll(&mut self) -> Result<Option<String>, Error> {
        if let Some(line) = self.notifications.pop_front() {
            return Ok(Some(line));
        }
        match self.messages.recv_timeout(Duration::from_millis(20)) {
            Ok(line) => line.map(Some),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::ServerExit),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if self
                    .child
                    .try_wait()
                    .map_err(|_| Error::Transport)?
                    .is_some()
                {
                    // Allow the stdout reader to deliver final messages before detecting EOF.
                    match self.messages.recv_timeout(Duration::from_millis(20)) {
                        Ok(line) => line.map(Some),
                        Err(_) => Err(Error::ServerExit),
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        cancelled: &AtomicBool,
    ) -> Result<(Value, usize), Error> {
        self.send(&json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + TIMEOUT;
        let mut notifications = 0;
        loop {
            let line = self.receive(deadline, cancelled)?;
            let value: Value = serde_json::from_str(&line).map_err(|_| Error::MalformedMessage)?;
            match classify(&value, id)? {
                Frame::Notification => {
                    notifications += 1;
                    if self.notifications.iter().map(String::len).sum::<usize>() + line.len()
                        > INITIAL_NOTIFICATION_LIMIT
                    {
                        return Err(Error::Transport);
                    }
                    self.notifications.push_back(line);
                }
                Frame::Response(result) => return Ok((result, notifications)),
            }
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // EOF gives Codex an opportunity to perform its own shutdown cleanup.
        self.writer.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = self.child.kill();
                    break;
                }
            }
        }
        let _ = self.child.wait();
        // A descendant may inherit the pipes. Never block application exit joining it.
        for worker in self.workers.drain(..) {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

enum Frame {
    Notification,
    Response(Value),
}
fn classify(value: &Value, expected: u64) -> Result<Frame, Error> {
    if !value.is_object() {
        return Err(Error::MalformedMessage);
    }
    if value.get("method").is_some() {
        if value.get("id").is_some() {
            // No server requests (including approvals) are expected during a probe.
            // Stop visibly; never synthesize an approval or silently discard it.
            return Err(Error::UnsupportedProtocol);
        }
        if value["method"].as_str().is_none()
            || value.get("result").is_some()
            || value.get("error").is_some()
        {
            return Err(Error::MalformedMessage);
        }
        return Ok(Frame::Notification);
    }
    if value.get("id") != Some(&json!(expected)) {
        return Err(Error::UnsupportedProtocol);
    }
    match (value.get("result"), value.get("error")) {
        (Some(result), None) => Ok(Frame::Response(result.clone())),
        (None, Some(error)) => {
            let code = error["code"].as_i64().ok_or(Error::MalformedMessage)?;
            if error["message"].as_str().is_none() {
                return Err(Error::MalformedMessage);
            }
            // Do not forward arbitrary server error messages or data to logs/UI.
            Err(Error::Remote { code })
        }
        _ => Err(Error::MalformedMessage),
    }
}

pub(crate) fn response(value: &Value, id: u64) -> Result<Value, Error> {
    match classify(value, id)? {
        Frame::Response(result) => Ok(result),
        _ => Err(Error::UnsupportedProtocol),
    }
}

fn validate_version(line: &str) -> Result<String, Error> {
    let version = line
        .trim()
        .strip_prefix("codex-cli ")
        .ok_or(Error::UnsupportedVersion)?;
    let core = version.split('-').next().unwrap_or("");
    let numbers: Vec<_> = core.split('.').collect();
    if numbers.len() != 3
        || numbers
            .iter()
            .any(|n| n.is_empty() || !n.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err(Error::UnsupportedVersion);
    }
    // No invented minimum version: compatibility is probed with initialize below.
    Ok(line.trim().to_owned())
}

fn installed(cancelled: &AtomicBool) -> Result<(PathBuf, String), Error> {
    let executable = discover_in(&env::var_os("PATH").unwrap_or_default())?;
    let version = {
        let mut process = Process::spawn(&executable, &["--version"])?;
        let deadline = Instant::now() + TIMEOUT;
        let version = validate_version(&process.receive(deadline, cancelled)?)?;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            if let Some(status) = process.child.try_wait().map_err(|_| Error::Transport)? {
                if !status.success() {
                    return Err(Error::UnsupportedVersion);
                }
                break;
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            thread::sleep(Duration::from_millis(20));
        }
        version
    };
    Ok((executable, version))
}

pub(crate) fn connect_project(
    root: &Path,
    cancelled: &AtomicBool,
) -> Result<(Process, String), Error> {
    let (executable, version) = installed(cancelled)?;
    let mut process = Process::spawn_at(&executable, &["app-server"], Some(root))?;
    initialize(&mut process, cancelled)?;
    Ok((process, version))
}

#[cfg(test)]
pub fn probe(cancelled: &AtomicBool) -> Result<Report, Error> {
    let (executable, version) = installed(cancelled)?;
    let mut process = Process::spawn(&executable, &["app-server"])?;
    let notification_count = initialize(&mut process, cancelled)?;
    Ok(Report {
        executable: executable.display().to_string(),
        version,
        handshake: true,
        notification_count,
    })
}

pub(crate) fn initialize(process: &mut Process, cancelled: &AtomicBool) -> Result<usize, Error> {
    let (result, notification_count) = process.request(1, "initialize", json!({
        "clientInfo": {"name":"corpstex", "title":"CorpsTex", "version":env!("CARGO_PKG_VERSION")}
    }), cancelled)?;
    // userAgent is the common stable response field; tolerate additive fields.
    if result["userAgent"].as_str().is_none() {
        return Err(Error::UnsupportedProtocol);
    }
    process.send(&json!({"method":"initialized"}))?;
    Ok(notification_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn discovery_rejects_relative_paths_and_missing_executables() {
        assert_eq!(
            discover_in(std::ffi::OsStr::new(".")),
            Err(Error::MissingExecutable)
        );
        assert_eq!(
            discover_in(std::ffi::OsStr::new("/corpstex-nonexistent-fixture")),
            Err(Error::MissingExecutable)
        );
    }

    #[test]
    fn versions_are_identified_without_inventing_a_minimum() {
        assert!(validate_version("codex-cli 0.166.0\n").is_ok());
        assert!(validate_version("codex-cli 0.1.0").is_ok());
        for invalid in [
            "other 0.166.0",
            "codex-cli unknown",
            "codex-cli 1.2",
            "codex-cli 1..3",
        ] {
            assert_eq!(validate_version(invalid), Err(Error::UnsupportedVersion));
        }
    }

    #[test]
    fn correlates_ids_and_preserves_remote_error_codes() {
        assert!(matches!(
            classify(&json!({"id":7,"result":{"ok":true}}), 7),
            Ok(Frame::Response(_))
        ));
        for id in [json!(8), json!("7"), Value::Null] {
            assert!(matches!(
                classify(&json!({"id":id,"result":{}}), 7),
                Err(Error::UnsupportedProtocol)
            ));
        }
        assert!(matches!(
            classify(
                &json!({"id":7,"error":{"code":-32601,"message":"private detail"}}),
                7
            ),
            Err(Error::Remote { code: -32601 })
        ));
        assert!(matches!(
            classify(&json!({"method":"future/event","params":{"new":true}}), 7),
            Ok(Frame::Notification)
        ));
        assert!(matches!(
            classify(
                &json!({"id":"server-1","method":"item/commandExecution/requestApproval"}),
                7
            ),
            Err(Error::UnsupportedProtocol)
        ));
    }

    #[test]
    fn malformed_envelopes_are_rejected() {
        for frame in [
            json!([]),
            json!({"id":1}),
            json!({"id":1,"result":{},"error":{}}),
            json!({"method":3}),
            json!({"id":1,"error":{"code":"bad"}}),
        ] {
            assert!(matches!(classify(&frame, 1), Err(Error::MalformedMessage)));
        }
    }

    #[test]
    fn transport_rejects_partial_and_non_utf8_frames() {
        for bytes in [b"{}".to_vec(), vec![255, b'\n']] {
            let (tx, rx) = mpsc::sync_channel(32);
            read_lines(Cursor::new(bytes), tx);
            assert_eq!(rx.recv().unwrap(), Err(Error::MalformedMessage));
        }
    }

    #[test]
    fn transport_accepts_image_sized_history_and_preserves_following_frame() {
        let frame = json!({"id":2,"result":{"data":[{"id":"turn","items":[{
            "type":"userMessage","content":[{"type":"image","url":
                format!("data:image/png;base64,{}", "A".repeat(6 * 1024 * 1024))}]
        }]}]}});
        let mut bytes = serde_json::to_vec(&frame).unwrap();
        bytes.extend_from_slice(b"\n{\"method\":\"future/event\"}\n");
        let (tx, rx) = mpsc::sync_channel(32);
        read_lines(Cursor::new(bytes), tx);
        let line = rx.recv().unwrap().unwrap();
        assert_eq!(
            response(&serde_json::from_str(&line).unwrap(), 2).unwrap(),
            frame["result"]
        );
        assert_eq!(
            rx.recv().unwrap().unwrap(),
            "{\"method\":\"future/event\"}\n"
        );
        assert!(rx.recv().is_err());
    }

    #[test]
    fn transport_bounds_frames_and_distinguishes_oversize_from_malformed() {
        let mut exact = vec![b' '; LIMIT];
        exact[LIMIT - 1] = b'\n';
        let (tx, rx) = mpsc::sync_channel(32);
        read_lines(Cursor::new(exact), tx);
        assert_eq!(rx.recv().unwrap().unwrap().len(), LIMIT);
        let (tx, rx) = mpsc::sync_channel(32);
        read_lines(Cursor::new(vec![b'x'; LIMIT + 1]), tx);
        assert_eq!(rx.recv().unwrap(), Err(Error::MessageTooLarge));
        assert!(rx.recv().is_err());
    }

    // Fixed shell scripts are fixture servers only. Production never invokes a shell.
    #[cfg(unix)]
    fn fixture(script: &str) -> Process {
        Process::spawn(Path::new("/bin/sh"), &["-c", script]).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn stalled_stdin_write_stops_connection_without_retry_or_unbounded_wait() {
        let mut process = fixture("exec sleep 30");
        let started = Instant::now();
        assert_eq!(
            process.send_with_timeout(
                &json!({"id":1,"result":{"answer":"x".repeat(1024 * 1024)}}),
                Duration::from_millis(100),
            ),
            Err(Error::Transport)
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            process.send(&json!({"id":1,"result":{}})),
            Err(Error::Transport)
        );
        assert!(!process.child.wait().unwrap().success());
    }

    #[cfg(unix)]
    #[test]
    fn fixture_handshake_unknown_notification_and_initialized_order() {
        let mut process = fixture("read -r request; case \"$request\" in *initialize*) ;; *) exit 1;; esac; printf '%s\n' '{\"method\":\"future/event\",\"params\":{}}' '{\"id\":1,\"result\":{\"userAgent\":\"fixture\",\"future\":true}}'; read -r ready; case \"$ready\" in *initialized*) printf '%s\n' '{\"method\":\"fixture/ready\"}';; *) exit 2;; esac");
        let count = initialize(&mut process, &AtomicBool::new(false)).unwrap();
        assert_eq!(count, 1);
        let ready = process
            .receive(Instant::now() + TIMEOUT, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&ready).unwrap()["method"],
            "fixture/ready"
        );
        let queued = process.poll().unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&queued).unwrap()["method"],
            "future/event"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fixture_exit_malformed_and_unsupported_protocol() {
        let mut exited = fixture("exit 0");
        assert_eq!(
            exited.receive(Instant::now() + TIMEOUT, &AtomicBool::new(false)),
            Err(Error::ServerExit)
        );
        let mut malformed = fixture("read -r request; printf '%s\n' 'not json'");
        assert_eq!(
            malformed.request(1, "initialize", json!({}), &AtomicBool::new(false)),
            Err(Error::MalformedMessage)
        );
        let mut unsupported_shape =
            fixture("read -r request; printf '%s\\n' '{\"id\":1,\"result\":{}}'");
        assert_eq!(
            initialize(&mut unsupported_shape, &AtomicBool::new(false)),
            Err(Error::UnsupportedProtocol)
        );
        let mut incompatible = fixture("read -r request; printf '%s\n' '{\"id\":2,\"result\":{}}'");
        assert_eq!(
            incompatible.request(1, "initialize", json!({}), &AtomicBool::new(false)),
            Err(Error::UnsupportedProtocol)
        );
    }

    #[cfg(unix)]
    #[test]
    fn fixture_timeout_and_cancellation_clean_up_the_child() {
        // read blocks without launching any descendants.
        let mut process = fixture("read -r request; read -r next");
        process
            .send(&json!({"id":1,"method":"initialize"}))
            .unwrap();
        assert_eq!(
            process.receive(
                Instant::now() + Duration::from_millis(50),
                &AtomicBool::new(false)
            ),
            Err(Error::Timeout)
        );
        assert_eq!(
            process.receive(Instant::now() + TIMEOUT, &AtomicBool::new(true)),
            Err(Error::Cancelled)
        );
        process.child.kill().unwrap();
        assert!(!process.child.wait().unwrap().success());
    }

    #[cfg(unix)]
    #[test]
    fn process_drop_allows_eof_cleanup_before_forced_termination() {
        let directory =
            std::env::temp_dir().join(format!("corpstex-m02-eof-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let marker = directory.join("cleaned");
        let process = Process::spawn(
            Path::new("/bin/sh"),
            &[
                "-c",
                "read -r request; printf done > \"$1\"",
                "fixture",
                marker.to_str().unwrap(),
            ],
        )
        .unwrap();
        drop(process);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "done");
        std::fs::remove_file(marker).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dropping_probe_reaps_its_child() {
        let process = fixture("read -r request");
        let proc_path = PathBuf::from(format!("/proc/{}", process.child.id()));
        assert!(proc_path.exists());
        drop(process);
        assert!(!proc_path.exists());
    }

    #[test]
    #[ignore = "explicit saved-history smoke; requires CORPSTEX_SMOKE_THREAD_ID; no inference"]
    fn installed_saved_history_transport() {
        let thread_id =
            env::var("CORPSTEX_SMOKE_THREAD_ID").expect("select a saved thread explicitly");
        let cancelled = AtomicBool::new(false);
        let (executable, version) = installed(&cancelled).unwrap();
        let mut process = Process::spawn(&executable, &["app-server"]).unwrap();
        initialize(&mut process, &cancelled).unwrap();
        process
            .request(
                2,
                "thread/resume",
                json!({"threadId":thread_id,"excludeTurns":true}),
                &cancelled,
            )
            .unwrap();
        let (history, _) = process
            .request(
                3,
                "thread/turns/list",
                json!({"threadId":thread_id,"limit":30,"sortDirection":"desc","itemsView":"full"}),
                &cancelled,
            )
            .unwrap();
        assert!(!history["data"].as_array().unwrap().is_empty());
        println!(
            "{version}: saved history parsed ({} result bytes); no turn started",
            serde_json::to_vec(&history).unwrap().len()
        );
    }

    #[test]
    #[ignore = "explicit installed-Codex smoke test; no credentials or inference required"]
    fn installed_codex_handshake() {
        let report = probe(&AtomicBool::new(false)).expect("installed Codex handshake");
        assert!(report.handshake);
        println!(
            "{}: initialize/initialized passed; temporary child cleaned up",
            report.version
        );
    }
}
