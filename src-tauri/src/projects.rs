//! One Rust-owned actor per canonical project. All state is transient and app-bound.
#[path = "approvals.rs"]
mod approvals;
#[path = "conversations.rs"]
mod conversations;
#[path = "opencode.rs"]
mod opencode;
pub use approvals::{Approval, Reply as ApprovalReply};
pub use conversations::Action;

use crate::runtime::{self, Error, Process};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

static NEXT_APPROVAL: AtomicU64 = AtomicU64::new(1);

const EVENT_COUNT: usize = 256;
const EVENT_BYTES: usize = 4 * 1024 * 1024;
type Spawner = dyn Fn(&Path, &AtomicBool) -> Result<(Process, String), Error> + Send + Sync;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    #[default]
    Codex,
    Opencode,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Starting,
    Ready,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub runtime: Runtime,
    pub root: String,
    pub generation: u64,
    pub status: Status,
    pub version: Option<String>,
    pub pid: Option<u32>,
    pub error: Option<Error>,
    pub active_turns: usize,
    pub pending_requests: usize,
    pub sequence: u64,
    pub uncertain: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub runtime: Runtime,
    pub project: String,
    pub generation: u64,
    pub sequence: u64,
    // Client receipt time, never a claimed runtime execution time.
    pub received_at_ms: Option<u64>,
    // ACP has no turn ID on the wire; this is explicitly client-correlated.
    pub client_turn_id: Option<String>,
    pub message: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Events {
    pub truncated: bool,
    pub events: Vec<Event>,
    // Outstanding requests survive event-ring eviction.
    pub pending: Vec<Value>,
}

struct ServerState {
    snapshot: Snapshot,
    active: HashSet<(String, String)>,
    pending: HashMap<String, Value>,
    tickets: HashMap<String, u64>,
    approvals_frozen: bool,
    events: VecDeque<(usize, Event)>,
    bytes: usize,
    threads: HashMap<String, Value>,
    conversation_revision: u64,
}
impl ServerState {
    fn new(root: &Path, generation: u64) -> Self {
        Self {
            snapshot: Snapshot {
                runtime: Runtime::Codex,
                root: root.to_string_lossy().into_owned(),
                generation,
                status: Status::Starting,
                version: None,
                pid: None,
                error: None,
                active_turns: 0,
                pending_requests: 0,
                sequence: 0,
                uncertain: false,
            },
            active: HashSet::new(),
            pending: HashMap::new(),
            tickets: HashMap::new(),
            approvals_frozen: false,
            events: VecDeque::new(),
            bytes: 0,
            threads: HashMap::new(),
            conversation_revision: 0,
        }
    }
    fn record(&mut self, line: &str) -> Result<(), Error> {
        let message: Value = serde_json::from_str(line).map_err(|_| Error::MalformedMessage)?;
        let method = message["method"]
            .as_str()
            .ok_or(Error::UnsupportedProtocol)?;
        if message.get("result").is_some() || message.get("error").is_some() {
            return Err(Error::MalformedMessage);
        }
        let params = &message["params"];
        if let Some(id) = message.get("id") {
            let key = request_key(id)?;
            // Duplicate IDs cannot overwrite an outstanding request unnoticed.
            if self.pending.contains_key(&key) {
                return Err(Error::UnsupportedProtocol);
            }
            self.tickets
                .insert(key.clone(), NEXT_APPROVAL.fetch_add(1, Ordering::Relaxed));
            self.pending.insert(key, message.clone());
        } else {
            match method {
                "turn/started" | "turn/completed" => {
                    if let (Some(thread), Some(turn)) =
                        (params["threadId"].as_str(), params["turn"]["id"].as_str())
                    {
                        let key = (thread.to_owned(), turn.to_owned());
                        if method == "turn/started" {
                            self.active.insert(key);
                        } else if matches!(
                            params["turn"]["status"].as_str(),
                            Some("completed" | "failed" | "interrupted")
                        ) {
                            self.active.remove(&key);
                        } else {
                            self.snapshot.uncertain = true;
                        }
                    } else {
                        self.snapshot.uncertain = true;
                    }
                }
                "serverRequest/resolved" => {
                    if let Ok(key) = request_key(&params["requestId"]) {
                        // Only resolve the request belonging to the reported thread.
                        if let Some(request) = self.pending.get(&key) {
                            let thread = request["params"]["threadId"]
                                .as_str()
                                .or_else(|| request["params"]["conversationId"].as_str());
                            if thread.is_some() && thread == params["threadId"].as_str() {
                                self.pending.remove(&key);
                                self.tickets.remove(&key);
                            } else {
                                self.snapshot.uncertain = true;
                            }
                        }
                    } else {
                        self.snapshot.uncertain = true;
                    }
                }
                _ => {} // Preserve unknown notifications intact, without guessing their meaning.
            }
        }
        conversations::record(&mut self.threads, &message);
        self.conversation_revision += 1;
        self.snapshot.active_turns = self.active.len();
        self.snapshot.pending_requests = self.pending.len();
        self.retain_event(message, line.len());
        Ok(())
    }

    fn retain_event(&mut self, message: Value, bytes: usize) {
        self.snapshot.sequence += 1;
        let client_turn_id = if self.snapshot.runtime == Runtime::Opencode {
            message["params"]["sessionId"].as_str().and_then(|session| {
                self.threads.get(session).and_then(|thread| {
                    thread["turns"].as_array()?.last()?["id"]
                        .as_str()
                        .map(str::to_owned)
                })
            })
        } else {
            None
        };
        let event = Event {
            runtime: self.snapshot.runtime,
            project: self.snapshot.root.clone(),
            generation: self.snapshot.generation,
            sequence: self.snapshot.sequence,
            received_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|duration| u64::try_from(duration.as_millis()).ok()),
            client_turn_id,
            message,
        };
        self.bytes += bytes;
        self.events.push_back((bytes, event));
        while self.events.len() > EVENT_COUNT || self.bytes > EVENT_BYTES {
            if let Some((bytes, _)) = self.events.pop_front() {
                self.bytes -= bytes;
            }
        }
    }
}
fn request_key(id: &Value) -> Result<String, Error> {
    if id.is_string() || id.as_u64().is_some() || id.as_i64().is_some() {
        Ok(id.to_string())
    } else {
        Err(Error::MalformedMessage)
    }
}

pub enum Command {
    Conversation(conversations::Request),
    Approval {
        reply: ApprovalReply,
        response: mpsc::SyncSender<Result<Value, Error>>,
    },
}

struct Entry {
    state: Arc<Mutex<ServerState>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    commands: mpsc::SyncSender<Command>,
    requests: Arc<AtomicU64>,
}
struct RequestGuard(Arc<AtomicU64>);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Inner {
    projects: HashMap<PathBuf, Entry>,
    windows: HashMap<String, HashSet<PathBuf>>,
    generation: u64,
    exit_requested: bool,
    closing: bool,
}
pub struct Registry {
    runtime: Runtime,
    inner: Mutex<Inner>,
    spawn: Arc<Spawner>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            runtime: Runtime::Codex,
            inner: Mutex::new(Inner::default()),
            spawn: Arc::new(runtime::connect_project),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub projects: Vec<Snapshot>,
    pub approvals: Vec<Approval>,
    pub exit_requested: bool,
    // Global summaries are returned only during app-exit confirmation.
    pub exit_projects: Vec<Snapshot>,
}

fn canonical(root: &str) -> Result<PathBuf, Error> {
    if !Path::new(root).is_absolute() {
        return Err(Error::InvalidProject);
    }
    let root = std::fs::canonicalize(root).map_err(|_| Error::InvalidProject)?;
    if !root.is_dir() || root.to_str().is_none() {
        return Err(Error::InvalidProject);
    }
    Ok(root)
}

impl Registry {
    pub fn opencode() -> Self {
        let mut registry = Self::default();
        registry.runtime = Runtime::Opencode;
        registry.spawn = Arc::new(opencode::spawn);
        registry
    }
    pub fn connect(
        &self,
        window: &str,
        root: &str,
        restart: Option<u64>,
        acknowledge_loss: bool,
    ) -> Result<Snapshot, Error> {
        let root = canonical(root)?;
        let mut inner = self.inner.lock().map_err(|_| Error::Transport)?;
        if inner.closing || inner.exit_requested {
            return Err(Error::ExitPending);
        }
        if let Some(entry) = inner.projects.get(&root) {
            let state = entry.state.lock().map_err(|_| Error::Transport)?;
            if let Some(generation) = restart {
                if generation != state.snapshot.generation {
                    return Err(Error::StaleGeneration);
                }
                if matches!(state.snapshot.status, Status::Starting | Status::Ready) {
                    return Err(Error::Busy);
                }
                if (state.snapshot.active_turns > 0
                    || state.snapshot.pending_requests > 0
                    || state.snapshot.uncertain)
                    && !acknowledge_loss
                {
                    return Err(Error::ConfirmationRequired);
                }
            } else {
                let snapshot = state.snapshot.clone();
                drop(state);
                inner
                    .windows
                    .entry(window.to_owned())
                    .or_default()
                    .insert(root);
                return Ok(snapshot);
            }
        } else if restart.is_some() {
            return Err(Error::StaleGeneration);
        }
        // A terminal state is published only AFTER the old child has been dropped/reaped.
        // Joining before replacement prevents overlap between process generations.
        if let Some(mut previous) = inner.projects.remove(&root) {
            if let Some(worker) = previous.worker.take() {
                worker.join().map_err(|_| Error::Transport)?;
            }
        }
        inner.generation += 1;
        let mut initial = ServerState::new(&root, inner.generation);
        initial.snapshot.runtime = self.runtime;
        let state = Arc::new(Mutex::new(initial));
        let stop = Arc::new(AtomicBool::new(false));
        let (actor_state, actor_stop, spawn) = (state.clone(), stop.clone(), self.spawn.clone());
        let project = root.clone();
        let (commands, receiver) = mpsc::sync_channel(32);
        let engine = self.runtime;
        let worker = thread::spawn(move || match engine {
            Runtime::Codex => run(&project, actor_state, actor_stop, spawn, receiver),
            Runtime::Opencode => opencode::run(&project, actor_state, actor_stop, spawn, receiver),
        });
        let snapshot = state.lock().map_err(|_| Error::Transport)?.snapshot.clone();
        inner.projects.insert(
            root.clone(),
            Entry {
                state,
                stop,
                worker: Some(worker),
                commands,
                requests: Arc::new(AtomicU64::new(0)),
            },
        );
        inner
            .windows
            .entry(window.to_owned())
            .or_default()
            .insert(root);
        Ok(snapshot)
    }

    // Explicit refresh after a key save: never interrupt active work or approvals.
    pub fn reconnect_idle(
        &self,
        window: &str,
        root: &str,
        generation: u64,
    ) -> Result<Snapshot, Error> {
        {
            let mut inner = self.inner.lock().map_err(|_| Error::Transport)?;
            if inner.exit_requested || inner.closing {
                return Err(Error::ExitPending);
            }
            let path = PathBuf::from(root);
            if !inner
                .windows
                .get(window)
                .is_some_and(|roots| roots.contains(&path))
            {
                return Err(Error::NotSubscribed);
            }
            let entry = inner.projects.get_mut(&path).ok_or(Error::NotSubscribed)?;
            {
                let mut state = entry.state.lock().map_err(|_| Error::Transport)?;
                if state.snapshot.generation != generation {
                    return Err(Error::StaleGeneration);
                }
                if state.snapshot.status != Status::Ready
                    || state.snapshot.uncertain
                    || !state.active.is_empty()
                    || !state.pending.is_empty()
                    || entry.requests.load(Ordering::SeqCst) != 0
                {
                    return Err(Error::Busy);
                }
                state.approvals_frozen = true;
                entry.stop.store(true, Ordering::Relaxed);
            }
            // The worker only locks its own state, so joining under the registry lock
            // excludes new requests while the old child is dropped and reaped.
            if let Some(worker) = entry.worker.take() {
                worker.join().map_err(|_| Error::Transport)?;
            }
        }
        self.connect(window, root, Some(generation), false)
    }

    pub fn view(&self, window: &str) -> Result<View, Error> {
        let inner = self.inner.lock().map_err(|_| Error::Transport)?;
        let mut projects = Vec::new();
        let mut exit_projects = Vec::new();
        let mut approvals = Vec::new();
        for (root, entry) in &inner.projects {
            let state = entry.state.lock().map_err(|_| Error::Transport)?;
            let snapshot = state.snapshot.clone();
            approvals.extend(approvals::queue(&state));
            if inner
                .windows
                .get(window)
                .is_some_and(|roots| roots.contains(root))
            {
                projects.push(snapshot.clone());
            }
            if inner.exit_requested {
                exit_projects.push(snapshot);
            }
        }
        projects.sort_by(|a, b| a.root.cmp(&b.root));
        exit_projects.sort_by(|a, b| a.root.cmp(&b.root));
        approvals.sort_by_key(|a| a.ticket);
        Ok(View {
            projects,
            approvals,
            exit_requested: inner.exit_requested,
            exit_projects,
        })
    }

    pub fn events(
        &self,
        window: &str,
        root: &str,
        generation: u64,
        after: u64,
    ) -> Result<Events, Error> {
        // Match the recorded identity even if the directory was removed after startup.
        let root = PathBuf::from(root);
        let inner = self.inner.lock().map_err(|_| Error::Transport)?;
        if !inner
            .windows
            .get(window)
            .is_some_and(|roots| roots.contains(&root))
        {
            return Err(Error::NotSubscribed);
        }
        let state = inner
            .projects
            .get(&root)
            .ok_or(Error::NotSubscribed)?
            .state
            .lock()
            .map_err(|_| Error::Transport)?;
        if generation != state.snapshot.generation {
            return Err(Error::StaleGeneration);
        }
        let first = state
            .events
            .front()
            .map(|(_, event)| event.sequence)
            .unwrap_or(state.snapshot.sequence + 1);
        Ok(Events {
            truncated: after.saturating_add(1) < first,
            events: state
                .events
                .iter()
                .filter(|(_, event)| event.sequence > after)
                .map(|(_, event)| event.clone())
                .collect(),
            pending: state.pending.values().cloned().collect(),
        })
    }

    pub fn request(
        &self,
        window: &str,
        root: &str,
        generation: u64,
        action: Action,
    ) -> Result<Value, Error> {
        let (sender, _guard) = {
            let inner = self.inner.lock().map_err(|_| Error::Transport)?;
            if inner.exit_requested || inner.closing {
                return Err(Error::ExitPending);
            }
            let root = PathBuf::from(root);
            if !inner
                .windows
                .get(window)
                .is_some_and(|roots| roots.contains(&root))
            {
                return Err(Error::NotSubscribed);
            }
            let entry = inner.projects.get(&root).ok_or(Error::NotSubscribed)?;
            let state = entry.state.lock().map_err(|_| Error::Transport)?;
            if state.snapshot.generation != generation {
                return Err(Error::StaleGeneration);
            }
            if state.snapshot.status != Status::Ready {
                return Err(Error::ServerExit);
            }
            entry.requests.fetch_add(1, Ordering::SeqCst);
            (entry.commands.clone(), RequestGuard(entry.requests.clone()))
        };
        let (reply, response) = mpsc::sync_channel(1);
        sender
            .try_send(Command::Conversation(conversations::Request {
                action,
                reply,
            }))
            .map_err(|_| Error::Busy)?;
        response
            .recv_timeout(std::time::Duration::from_secs(65))
            .map_err(|_| Error::ServerExit)?
    }

    // The trusted approval center is global, independent of the window's selected project.
    // Only an existing native window may call this path; root/generation/ticket are rechecked.
    pub fn approval_reply(
        &self,
        root: &str,
        generation: u64,
        reply: ApprovalReply,
    ) -> Result<Value, Error> {
        let sender = {
            let inner = self.inner.lock().map_err(|_| Error::Transport)?;
            if inner.exit_requested || inner.closing {
                return Err(Error::ExitPending);
            }
            let entry = inner
                .projects
                .get(&PathBuf::from(root))
                .ok_or(Error::InvalidProject)?;
            let state = entry.state.lock().map_err(|_| Error::Transport)?;
            if state.snapshot.generation != generation {
                return Err(Error::StaleGeneration);
            }
            if state.snapshot.status != Status::Ready {
                return Err(Error::ServerExit);
            }
            entry.commands.clone()
        };
        let (response, receiver) = mpsc::sync_channel(1);
        sender
            .try_send(Command::Approval { reply, response })
            .map_err(|_| Error::Busy)?;
        receiver
            .recv_timeout(std::time::Duration::from_secs(65))
            .map_err(|_| Error::ServerExit)?
    }

    pub fn conversation(
        &self,
        window: &str,
        root: &str,
        generation: u64,
        thread: &str,
    ) -> Result<Value, Error> {
        self.conversation_since(window, root, generation, thread, None)
    }

    pub fn conversation_since(
        &self,
        window: &str,
        root: &str,
        generation: u64,
        thread: &str,
        after: Option<u64>,
    ) -> Result<Value, Error> {
        self.with_conversation(window, root, generation, thread, |thread, revision| {
            if after == Some(revision) {
                return Ok(Value::Null);
            }
            let mut thread = thread.clone();
            thread["clientRevision"] = Value::from(revision);
            Ok(thread)
        })
    }

    pub fn image_path(
        &self,
        window: &str,
        root: &str,
        generation: u64,
        thread: &str,
        path: &str,
    ) -> Result<PathBuf, Error> {
        self.with_conversation(window, root, generation, thread, |thread, _| {
            crate::media::image_path(thread, root, path)
        })
    }

    fn with_conversation<T>(
        &self,
        window: &str,
        root: &str,
        generation: u64,
        thread: &str,
        read: impl FnOnce(&Value, u64) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let inner = self.inner.lock().map_err(|_| Error::Transport)?;
        let root = PathBuf::from(root);
        if !inner
            .windows
            .get(window)
            .is_some_and(|roots| roots.contains(&root))
        {
            return Err(Error::NotSubscribed);
        }
        let state = inner
            .projects
            .get(&root)
            .ok_or(Error::NotSubscribed)?
            .state
            .lock()
            .map_err(|_| Error::Transport)?;
        if state.snapshot.generation != generation {
            return Err(Error::StaleGeneration);
        }
        let thread = state.threads.get(thread).ok_or(Error::InvalidRequest)?;
        read(thread, state.conversation_revision)
    }

    pub fn detach_window(&self, window: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.windows.remove(window);
        }
        // Subscription removal never stops a project process.
    }

    pub fn has_projects(&self) -> Result<bool, Error> {
        Ok(!self
            .inner
            .lock()
            .map_err(|_| Error::Transport)?
            .projects
            .is_empty())
    }

    pub fn request_exit(&self) -> Result<bool, Error> {
        let mut inner = self.inner.lock().map_err(|_| Error::Transport)?;
        inner.exit_requested = true;
        for entry in inner.projects.values() {
            entry
                .state
                .lock()
                .map_err(|_| Error::Transport)?
                .approvals_frozen = true;
        }
        // Always confirm if any server has existed: in-flight events cannot race an idle check.
        Ok(!inner.projects.is_empty())
    }

    pub fn cancel_exit(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            if !inner.closing {
                inner.exit_requested = false;
                for entry in inner.projects.values() {
                    if let Ok(mut state) = entry.state.lock() {
                        state.approvals_frozen = false;
                    }
                }
            }
        }
    }

    pub fn shutdown(&self, confirmed: bool) -> Result<(), Error> {
        let mut workers = Vec::new();
        {
            let mut inner = self.inner.lock().map_err(|_| Error::Transport)?;
            if !confirmed && !inner.projects.is_empty() {
                return Err(Error::ConfirmationRequired);
            }
            inner.closing = true;
            for entry in inner.projects.values() {
                entry
                    .state
                    .lock()
                    .map_err(|_| Error::Transport)?
                    .approvals_frozen = true;
            }
            for entry in inner.projects.values_mut() {
                entry.stop.store(true, Ordering::Relaxed);
                if let Some(worker) = entry.worker.take() {
                    workers.push(worker);
                }
            }
        }
        for worker in workers {
            worker.join().map_err(|_| Error::Transport)?;
        }
        Ok(())
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        let _ = self.shutdown(true);
    }
}

fn run(
    root: &Path,
    state: Arc<Mutex<ServerState>>,
    stop: Arc<AtomicBool>,
    spawn: Arc<Spawner>,
    commands: mpsc::Receiver<Command>,
) {
    let outcome = (|| {
        let (mut process, version) = spawn(root, &stop)?;
        {
            let mut state = state.lock().map_err(|_| Error::Transport)?;
            state.snapshot.status = Status::Ready;
            state.snapshot.pid = Some(process.pid());
            state.snapshot.version = Some(version);
        }
        conversations::serve(&mut process, &state, &stop, commands)
        // Process drops and reaps here, before publishing a terminal status.
    })();
    if let Ok(mut state) = state.lock() {
        state.snapshot.pid = None;
        if stop.load(Ordering::Relaxed) {
            state.snapshot.status = Status::Stopped;
        } else {
            state.snapshot.status = Status::Failed;
            state.snapshot.error = outcome.err();
        }
        // Keep unresolved turns/requests visible after a crash until explicit reconnection.
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        sync::{atomic::AtomicUsize, Barrier},
        time::{Duration, Instant},
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "corpstex-m02-{}-{}",
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

    const HOLD: &str = "read -r ignored";
    const ACTIVE: &str = "printf '%s\n' '{\"method\":\"turn/started\",\"params\":{\"threadId\":\"thread\",\"turn\":{\"id\":\"turn\",\"status\":\"inProgress\"}}}' '{\"id\":1,\"method\":\"item/commandExecution/requestApproval\",\"params\":{\"threadId\":\"thread\",\"turnId\":\"turn\"}}'; read -r ignored";
    fn spawn_fixture(
        root: &Path,
        cancelled: &AtomicBool,
        tail: &str,
    ) -> Result<(Process, String), Error> {
        // Fixture code only: production never invokes /bin/sh.
        let script = format!("read -r initialize; printf '%s\\n' '{{\"id\":1,\"result\":{{\"userAgent\":\"fixture\"}}}}'; read -r initialized; {tail}");
        let mut process = Process::spawn_at(Path::new("/bin/sh"), &["-c", &script], Some(root))?;
        runtime::initialize(&mut process, cancelled)?;
        Ok((process, "fixture 0.166.0".into()))
    }
    fn registry(tail: &'static str) -> (Arc<Registry>, Arc<AtomicUsize>) {
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = starts.clone();
        (
            Arc::new(Registry {
                runtime: Runtime::Codex,
                inner: Mutex::new(Inner::default()),
                spawn: Arc::new(move |root, cancelled| {
                    counter.fetch_add(1, Ordering::Relaxed);
                    spawn_fixture(root, cancelled, tail)
                }),
            }),
            starts,
        )
    }
    fn wait(
        registry: &Registry,
        window: &str,
        condition: impl Fn(&[Snapshot]) -> bool,
    ) -> Vec<Snapshot> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let snapshots = registry.view(window).unwrap().projects;
            if condition(&snapshots) {
                return snapshots;
            }
            assert!(
                Instant::now() < deadline,
                "fixture state timed out: {snapshots:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    const APPROVALS: &str = r#"printf '%s\n' '{"id":9007199254740993,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread","turnId":"turn","itemId":"item","command":"fixture command","availableDecisions":["accept","decline","cancel"]}}' '{"id":"9007199254740993","method":"item/fileChange/requestApproval","params":{"threadId":"thread","turnId":"turn","itemId":"file"}}'; while IFS= read -r response; do printf '%s\n' "$response" >> responses; done"#;

    #[test]
    fn legacy_approval_resolution_uses_conversation_identity() {
        let mut state = ServerState::new(Path::new("/fixture"), 1);
        state.record(&json!({"id":1,"method":"execCommandApproval","params":{"conversationId":"legacy-thread","callId":"call"}}).to_string()).unwrap();
        state.record(&json!({"method":"serverRequest/resolved","params":{"threadId":"other","requestId":1}}).to_string()).unwrap();
        assert_eq!(state.pending.len(), 1);
        state.record(&json!({"method":"serverRequest/resolved","params":{"threadId":"legacy-thread","requestId":1}}).to_string()).unwrap();
        assert!(state.pending.is_empty());
        assert!(state.tickets.is_empty());
        assert_eq!(state.snapshot.pending_requests, 0);
    }

    #[test]
    fn global_approval_queue_routes_colliding_ids_and_serializes_concurrent_replies() {
        let left = Temp::new();
        let right = Temp::new();
        let (registry, _) = registry(APPROVALS);
        registry.connect("left", left.root(), None, false).unwrap();
        registry
            .connect("right", right.root(), None, false)
            .unwrap();
        wait(&registry, "left", |s| s[0].pending_requests == 2);
        wait(&registry, "right", |s| s[0].pending_requests == 2);
        // The center sees all projects even from a window with no project subscriptions.
        let queue = registry.view("approval-center").unwrap().approvals;
        assert_eq!(queue.len(), 4);
        assert!(queue.windows(2).all(|p| p[0].ticket < p[1].ticket));
        let a = queue
            .iter()
            .find(|a| a.root == left.root() && a.request["id"].is_number())
            .unwrap();
        let reply = ApprovalReply {
            key: a.key.clone(),
            ticket: a.ticket,
            decision: approvals::Decision::Decline {},
        };
        assert_eq!(
            registry.approval_reply(right.root(), a.generation, reply.clone()),
            Err(Error::StaleGeneration)
        );
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (registry, barrier, root, reply, generation) = (
                    registry.clone(),
                    barrier.clone(),
                    a.root.clone(),
                    reply.clone(),
                    a.generation,
                );
                thread::spawn(move || {
                    barrier.wait();
                    registry.approval_reply(&root, generation, reply)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(results.contains(&Err(Error::InvalidRequest)));
        assert_eq!(
            registry.view("right").unwrap().projects[0].pending_requests,
            2
        );
        // Numeric IDs above JS's safe integer limit never round-trip through JS numbers.
        let deadline = Instant::now() + Duration::from_secs(2);
        let response = loop {
            if let Ok(contents) = std::fs::read_to_string(left.0.join("responses")) {
                if let Ok(value) = serde_json::from_str::<Value>(contents.trim()) {
                    break value;
                }
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(
            response,
            json!({"id":9007199254740993_u64,"result":{"decision":"decline"}})
        );
        assert!(!right.0.join("responses").exists());
        assert_eq!(registry.view("left").unwrap().approvals.len(), 3);
        registry.request_exit().unwrap();
        let a = &registry.view("left").unwrap().approvals[0];
        let reply = ApprovalReply {
            key: a.key.clone(),
            ticket: a.ticket,
            decision: approvals::Decision::Accept {},
        };
        assert_eq!(
            registry.approval_reply(&a.root, a.generation, reply.clone()),
            Err(Error::ExitPending)
        );
        registry.cancel_exit();
        registry
            .approval_reply(&a.root, a.generation, reply)
            .unwrap();
        registry.shutdown(true).unwrap();
        let a = &registry.view("left").unwrap().approvals[0];
        assert_eq!(a.status, Status::Stopped);
        assert_eq!(
            registry.approval_reply(
                &a.root,
                a.generation,
                ApprovalReply {
                    key: a.key.clone(),
                    ticket: a.ticket,
                    decision: approvals::Decision::Accept {}
                }
            ),
            Err(Error::ExitPending)
        );
    }

    #[test]
    fn crashed_approval_cannot_be_answered_and_reconnect_does_not_replay_it() {
        let project = Temp::new();
        let (registry, _) = registry(
            r#"printf '%s\n' '{"id":1,"method":"item/fileChange/requestApproval","params":{"threadId":"t","turnId":"v","itemId":"i"}}'; exit 7"#,
        );
        registry
            .connect("main", project.root(), None, false)
            .unwrap();
        let s = wait(&registry, "main", |s| s[0].status == Status::Failed)[0].clone();
        let a = registry.view("main").unwrap().approvals.remove(0);
        assert_eq!(a.status, Status::Failed);
        let reply = ApprovalReply {
            key: a.key,
            ticket: a.ticket,
            decision: approvals::Decision::Accept {},
        };
        assert_eq!(
            registry.approval_reply(&a.root, a.generation, reply.clone()),
            Err(Error::ServerExit)
        );
        registry
            .connect("main", project.root(), Some(s.generation), true)
            .unwrap();
        assert_eq!(
            registry.approval_reply(&a.root, a.generation, reply),
            Err(Error::StaleGeneration)
        );
        registry.shutdown(true).unwrap();
    }

    #[test]
    fn concurrent_canonical_aliases_and_windows_share_one_process() {
        let project = Temp::new();
        let alias = Temp::new();
        let link = alias.0.join("alias");
        std::os::unix::fs::symlink(&project.0, &link).unwrap();
        let (registry, starts) = registry(HOLD);
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let (registry, barrier) = (registry.clone(), barrier.clone());
                let root = if index % 2 == 0 {
                    project.0.join(".")
                } else {
                    link.clone()
                };
                thread::spawn(move || {
                    barrier.wait();
                    registry
                        .connect(
                            &format!("window-{index}"),
                            root.to_str().unwrap(),
                            None,
                            false,
                        )
                        .unwrap()
                        .generation
                })
            })
            .collect();
        let generations: HashSet<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(generations.len(), 1);
        wait(&registry, "window-0", |s| s[0].status == Status::Ready);
        assert_eq!(starts.load(Ordering::Relaxed), 1);
        registry.detach_window("window-0");
        assert!(registry.view("window-0").unwrap().projects.is_empty());
        assert_eq!(
            registry.view("window-1").unwrap().projects[0].status,
            Status::Ready
        );
        registry.shutdown(true).unwrap();
    }

    #[test]
    fn concurrent_projects_isolate_events_and_colliding_request_ids() {
        let (left, right) = (Temp::new(), Temp::new());
        let (registry, starts) = registry(ACTIVE);
        thread::scope(|scope| {
            scope.spawn(|| registry.connect("left", left.root(), None, false).unwrap());
            scope.spawn(|| {
                registry
                    .connect("right", right.root(), None, false)
                    .unwrap()
            });
        });
        let l = wait(&registry, "left", |s| s[0].pending_requests == 1)[0].clone();
        let r = wait(&registry, "right", |s| s[0].pending_requests == 1)[0].clone();
        assert_eq!(starts.load(Ordering::Relaxed), 2);
        assert_ne!(l.pid, r.pid);
        assert_ne!(l.generation, r.generation);
        assert_eq!(l.active_turns, 1);
        for (window, snapshot) in [("left", &l), ("right", &r)] {
            let batch = registry
                .events(window, &snapshot.root, snapshot.generation, 0)
                .unwrap();
            assert_eq!(batch.pending.len(), 1);
            assert_eq!(batch.events.len(), 2);
            assert!(batch
                .events
                .iter()
                .all(|event| event.project == snapshot.root
                    && event.generation == snapshot.generation));
            assert_eq!(batch.events[0].sequence, 1);
            assert_eq!(batch.events[1].sequence, 2);
        }
        assert!(matches!(
            registry.events("left", &r.root, r.generation, 0),
            Err(Error::NotSubscribed)
        ));
        assert!(matches!(
            registry.events("right", &l.root, l.generation, 0),
            Err(Error::NotSubscribed)
        ));
        registry.shutdown(true).unwrap();
    }

    #[test]
    fn exit_warns_freezes_startup_can_cancel_and_reaps_children() {
        let project = Temp::new();
        let other = Temp::new();
        let (registry, _) = registry(ACTIVE);
        registry
            .connect("main", project.root(), None, false)
            .unwrap();
        let snapshot = wait(&registry, "main", |s| s[0].pending_requests == 1)[0].clone();
        assert!(registry.request_exit().unwrap());
        let view = registry.view("main").unwrap();
        assert!(view.exit_requested);
        assert_eq!(view.exit_projects[0].active_turns, 1);
        assert_eq!(view.exit_projects[0].pending_requests, 1);
        assert!(matches!(
            registry.shutdown(false),
            Err(Error::ConfirmationRequired)
        ));
        assert!(matches!(
            registry.connect("main", other.root(), None, false),
            Err(Error::ExitPending)
        ));
        registry.cancel_exit();
        assert!(!registry.view("main").unwrap().exit_requested);
        assert!(registry
            .connect("main", project.root(), None, false)
            .is_ok());
        registry.shutdown(true).unwrap();
        let stopped = registry.view("main").unwrap().projects[0].clone();
        assert_eq!(stopped.status, Status::Stopped);
        assert!(stopped.pid.is_none());
        assert_eq!(stopped.pending_requests, 1); // Not silently answered or forgotten.
        #[cfg(target_os = "linux")]
        assert!(!PathBuf::from(format!("/proc/{}", snapshot.pid.unwrap())).exists());
    }

    #[test]
    fn crash_requires_explicit_reconnect_and_never_replays_work() {
        let project = Temp::new();
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = starts.clone();
        let registry = Registry {
            runtime: Runtime::Codex,
            inner: Mutex::new(Inner::default()),
            spawn: Arc::new(move |root, cancelled| {
                let count = counter.fetch_add(1, Ordering::Relaxed);
                let tail = if count == 0 {
                    "printf '%s\\n' '{\"id\":1,\"method\":\"approval\",\"params\":{\"threadId\":\"thread\"}}'; exit 7"
                } else {
                    HOLD
                };
                spawn_fixture(root, cancelled, tail)
            }),
        };
        registry
            .connect("main", project.root(), None, false)
            .unwrap();
        let failed = wait(&registry, "main", |s| s[0].status == Status::Failed)[0].clone();
        assert_eq!(failed.error, Some(Error::ServerExit));
        assert_eq!(failed.pending_requests, 1);
        assert!(failed.pid.is_none());
        registry
            .connect("second", project.root(), None, false)
            .unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 1); // Never restart automatically.
        assert!(matches!(
            registry.connect("main", project.root(), Some(failed.generation), false),
            Err(Error::ConfirmationRequired)
        ));
        let new = registry
            .connect("main", project.root(), Some(failed.generation), true)
            .unwrap();
        assert_ne!(new.generation, failed.generation);
        let ready = wait(&registry, "main", |s| s[0].status == Status::Ready)[0].clone();
        assert_eq!(ready.pending_requests, 0);
        assert_eq!(ready.active_turns, 0);
        assert_eq!(starts.load(Ordering::Relaxed), 2);
        assert!(matches!(
            registry.events("main", &ready.root, failed.generation, 0),
            Err(Error::StaleGeneration)
        ));
        assert!(matches!(
            registry.connect("main", project.root(), Some(ready.generation), true),
            Err(Error::Busy)
        ));
        registry.shutdown(true).unwrap();
    }

    #[test]
    fn ring_gap_is_explicit_and_pending_requests_survive_eviction() {
        let mut state = ServerState::new(Path::new("/fixture"), 42);
        state
            .record(
                &json!({"id":"1","method":"approval","params":{"threadId":"thread"}}).to_string(),
            )
            .unwrap();
        for _ in 0..300 {
            state
                .record("{\"method\":\"future/event\",\"params\":{\"opaque\":true}}")
                .unwrap();
        }
        assert_eq!(state.events.len(), EVENT_COUNT);
        assert_eq!(state.pending.len(), 1);
        assert!(state.events.front().unwrap().1.sequence > 1);
        let registry = Registry::default();
        let root = PathBuf::from("/fixture");
        {
            let mut inner = registry.inner.lock().unwrap();
            inner.projects.insert(
                root.clone(),
                Entry {
                    state: Arc::new(Mutex::new(state)),
                    stop: Arc::new(AtomicBool::new(false)),
                    worker: None,
                    commands: mpsc::sync_channel(1).0,
                    requests: Arc::new(AtomicU64::new(0)),
                },
            );
            inner.windows.entry("main".into()).or_default().insert(root);
        }
        let events = registry.events("main", "/fixture", 42, 0).unwrap();
        assert!(events.truncated);
        assert_eq!(events.pending.len(), 1);
        assert_eq!(events.events.last().unwrap().sequence, 301);
        assert!(events
            .events
            .iter()
            .all(|event| event.received_at_ms.is_some()));
        assert!(events
            .events
            .iter()
            .all(|event| event.client_turn_id.is_none()));
        let serialized = serde_json::to_value(&events.events[0]).unwrap();
        assert!(serialized["receivedAtMs"].is_u64());
        assert_eq!(serialized["message"]["method"], "future/event");
    }

    #[test]
    fn turn_and_resolution_tracking_are_thread_scoped_and_conservative() {
        let mut state = ServerState::new(Path::new("/fixture"), 1);
        for thread in ["one", "two"] {
            state.record(&json!({"method":"turn/started","params":{"threadId":thread,"turn":{"id":"same"}}}).to_string()).unwrap();
        }
        assert_eq!(state.snapshot.active_turns, 2);
        state.record(&json!({"method":"turn/completed","params":{"threadId":"one","turn":{"id":"same","status":"interrupted"}}}).to_string()).unwrap();
        assert_eq!(state.snapshot.active_turns, 1);
        state
            .record(&json!({"id":1,"method":"approval","params":{"threadId":"one"}}).to_string())
            .unwrap();
        state.record(&json!({"method":"serverRequest/resolved","params":{"threadId":"two","requestId":1}}).to_string()).unwrap();
        assert_eq!(state.snapshot.pending_requests, 1);
        assert!(state.snapshot.uncertain);
        state.record(&json!({"method":"serverRequest/resolved","params":{"threadId":"one","requestId":1}}).to_string()).unwrap();
        assert_eq!(state.snapshot.pending_requests, 0);
        state
            .record("{\"method\":\"turn/started\",\"params\":{}}")
            .unwrap();
        assert!(state.snapshot.uncertain);
    }

    #[test]
    fn persistent_transport_does_not_stop_after_128_notifications() {
        let project = Temp::new();
        let (registry, _) = registry("i=0; while [ \"$i\" -lt 300 ]; do printf '%s\n' '{\"method\":\"future/event\"}'; i=$((i+1)); done; read -r ignored");
        registry
            .connect("main", project.root(), None, false)
            .unwrap();
        let snapshot = wait(&registry, "main", |s| s[0].sequence == 300)[0].clone();
        assert_eq!(snapshot.status, Status::Ready);
        assert!(
            registry
                .events("main", &snapshot.root, snapshot.generation, 0)
                .unwrap()
                .truncated
        );
        registry.shutdown(true).unwrap();
    }

    #[test]
    fn shutdown_cancels_an_in_progress_startup() {
        let project = Temp::new();
        let started = Arc::new(AtomicBool::new(false));
        let signal = started.clone();
        let registry = Registry {
            runtime: Runtime::Codex,
            inner: Mutex::new(Inner::default()),
            spawn: Arc::new(move |_, cancelled| {
                signal.store(true, Ordering::Relaxed);
                while !cancelled.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(Error::Cancelled)
            }),
        };
        registry
            .connect("main", project.root(), None, false)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !started.load(Ordering::Relaxed) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        registry.shutdown(true).unwrap();
        let view = registry.view("main").unwrap();
        assert_eq!(view.projects[0].status, Status::Stopped);
        assert!(view.projects[0].pid.is_none());
        assert!(matches!(
            registry.connect("main", project.root(), None, false),
            Err(Error::ExitPending)
        ));
    }

    #[test]
    fn unchanged_conversation_polls_skip_payload_and_updates_remain_scoped() {
        let project = Temp::new();
        let (registry, _) = registry("read -r request; printf '%s\\n' '{\"id\":2,\"result\":{\"data\":[{\"id\":\"older\",\"status\":\"completed\",\"items\":[]}],\"nextCursor\":null}}'; read -r ignored");
        registry
            .connect("main", project.root(), None, false)
            .unwrap();
        let ready = wait(&registry, "main", |s| s[0].status == Status::Ready)[0].clone();
        let state = registry.inner.lock().unwrap().projects[&PathBuf::from(&ready.root)]
            .state
            .clone();
        state.lock().unwrap().threads.insert(
            "thread".into(),
            json!({"id":"thread","turns":[],"opaqueImage":"A".repeat(6 * 1024 * 1024)}),
        );
        state.lock().unwrap().threads.get_mut("thread").unwrap()["turns"] =
            json!([{"id":"media","items":[{"type":"imageView","path":"output.png"}]}]);
        assert_eq!(
            registry
                .image_path(
                    "main",
                    &ready.root,
                    ready.generation,
                    "thread",
                    "output.png"
                )
                .unwrap(),
            PathBuf::from(&ready.root).join("output.png")
        );
        assert_eq!(
            registry.image_path(
                "other",
                &ready.root,
                ready.generation,
                "thread",
                "output.png"
            ),
            Err(Error::NotSubscribed)
        );
        assert_eq!(
            registry.image_path(
                "main",
                &ready.root,
                ready.generation + 1,
                "thread",
                "output.png"
            ),
            Err(Error::StaleGeneration)
        );
        assert_eq!(
            registry.image_path(
                "main",
                &ready.root,
                ready.generation,
                "missing",
                "output.png"
            ),
            Err(Error::InvalidRequest)
        );
        assert_eq!(
            registry.image_path(
                "main",
                &ready.root,
                ready.generation,
                "thread",
                "secret.png"
            ),
            Err(Error::InvalidRequest)
        );
        state.lock().unwrap().threads.get_mut("thread").unwrap()["turns"] = json!([]);
        let first = registry
            .conversation("main", &ready.root, ready.generation, "thread")
            .unwrap();
        let revision = first["clientRevision"].as_u64().unwrap();
        let poll = |window, generation, thread, after| {
            registry.conversation_since(window, &ready.root, generation, thread, Some(after))
        };
        for _ in 0..100 {
            assert!(poll("main", ready.generation, "thread", revision)
                .unwrap()
                .is_null());
        }
        assert_eq!(
            poll("other", ready.generation, "thread", revision),
            Err(Error::NotSubscribed)
        );
        assert_eq!(
            poll("main", ready.generation + 1, "thread", revision),
            Err(Error::StaleGeneration)
        );
        assert_eq!(
            poll("main", ready.generation, "missing", revision),
            Err(Error::InvalidRequest)
        );
        registry
            .request(
                "main",
                &ready.root,
                ready.generation,
                Action::History {
                    thread_id: "thread".into(),
                    cursor: None,
                },
            )
            .unwrap();
        let history = poll("main", ready.generation, "thread", revision).unwrap();
        assert_eq!(history["turns"][0]["id"], "older");
        let revision = history["clientRevision"].as_u64().unwrap();
        assert!(poll("main", ready.generation, "thread", revision)
            .unwrap()
            .is_null());
        state.lock().unwrap().record(&json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread","tokenUsage":{"total":{"totalTokens":123}}}}).to_string()).unwrap();
        let updated = poll("main", ready.generation, "thread", revision).unwrap();
        assert_eq!(updated["tokenUsage"]["total"]["totalTokens"], 123);
        assert_eq!(
            updated["opaqueImage"].as_str().unwrap().len(),
            6 * 1024 * 1024
        );
        registry.shutdown(true).unwrap();
    }

    #[test]
    fn invalid_roots_and_empty_shutdown_never_spawn() {
        let (registry, starts) = registry(HOLD);
        assert!(matches!(
            registry.connect("main", ".", None, false),
            Err(Error::InvalidProject)
        ));
        assert!(matches!(
            registry.connect("main", "/corpstex-missing-fixture", None, false),
            Err(Error::InvalidProject)
        ));
        assert_eq!(starts.load(Ordering::Relaxed), 0);
        assert!(!registry.request_exit().unwrap());
        registry.shutdown(false).unwrap();
    }

    #[test]
    #[ignore = "explicit real conversation smoke; uses existing Codex login and a short inference"]
    fn installed_chat_and_history() {
        let project = Temp::new();
        let registry = Registry::default();
        registry
            .connect("chat", project.root(), None, false)
            .unwrap();
        let ready = wait(&registry, "chat", |s| s[0].status != Status::Starting)[0].clone();
        assert_eq!(ready.status, Status::Ready);
        let call = |action| registry.request("chat", &ready.root, ready.generation, action);
        let models = call(Action::ListModels {
            cursor: None,
            thread_id: None,
        })
        .expect("model/list");
        assert!(models["data"].is_array());
        let created = call(Action::NewThread).expect("thread/start");
        let id = created["thread"]["id"].as_str().unwrap().to_owned();
        call(Action::Send {
            thread_id: id.clone(),
            text: "Reply exactly: CorpsTex chat OK. Do not use tools or edit any files.".into(),
            skills: vec![],
            images: vec![],
            model: None,
            effort: None,
        })
        .expect("turn/start");
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let thread = registry
                .conversation("chat", &ready.root, ready.generation, &id)
                .unwrap();
            let turns = thread["turns"].as_array().unwrap();
            if let Some(turn) = turns.last() {
                if turn["status"] != "inProgress" {
                    assert_eq!(
                        turn["status"], "completed",
                        "real smoke turn did not complete"
                    );
                    assert!(turn["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["type"] == "agentMessage"
                            && item["text"]
                                .as_str()
                                .is_some_and(|text| text.contains("CorpsTex chat OK"))));
                    break;
                }
            }
            assert!(Instant::now() < deadline, "real smoke turn timed out");
            thread::sleep(Duration::from_millis(100));
        }
        let listed = call(Action::ListThreads { cursor: None }).expect("thread/list");
        assert!(listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == id));
        call(Action::Resume {
            thread_id: id.clone(),
        })
        .expect("thread/resume");
        let history = call(Action::History {
            thread_id: id,
            cursor: None,
        })
        .expect("thread/turns/list");
        assert!(!history["data"].as_array().unwrap().is_empty());
        registry.shutdown(true).unwrap();
        println!("Real installed-Codex text turn completed; listing, resume and native history passed. No tool or file action requested.");
    }

    #[test]
    #[ignore = "explicit installed Codex project-lifecycle smoke; no credentials or inference"]
    fn installed_project_lifecycle() {
        let project = Temp::new();
        let registry = Registry::default();
        registry
            .connect("first", project.root(), None, false)
            .unwrap();
        let first = wait(&registry, "first", |s| s[0].status != Status::Starting)[0].clone();
        assert_eq!(first.status, Status::Ready, "{first:?}");
        let second = registry
            .connect("second", project.root(), None, false)
            .unwrap();
        assert_eq!(first.pid, second.pid);
        assert_eq!(first.generation, second.generation);
        assert!(registry.request_exit().unwrap());
        registry.shutdown(true).unwrap();
        assert_eq!(
            registry.view("first").unwrap().projects[0].status,
            Status::Stopped
        );
        println!(
            "{}: canonical project shared by two window bindings; shutdown reaped child",
            first.version.unwrap()
        );
    }
}
