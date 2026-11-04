//! Bounded stdio LSP transport, independent of Codex and OpenCode.
use crate::editor::{error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};
const MAX_FRAME: usize = 16 * 1024 * 1024;
type Pending = Arc<Mutex<HashMap<u64, mpsc::SyncSender<Value>>>>;
struct Server {
    child: Mutex<Child>,
    writer: mpsc::SyncSender<Value>,
    pending: Pending,
    diagnostics: Arc<Mutex<HashMap<String, (usize, Value)>>>,
    diagnostics_omitted: Arc<AtomicBool>,
    next: AtomicU64,
    alive: Arc<AtomicBool>,
    root: String,
}
fn kill_tree(child: &mut Child) {
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
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            kill_tree(child);
        }
    }
}
impl Server {
    fn send(&self, value: Value) -> Result<()> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(error(
                "lspExit",
                "Language server stopped. Restart it in Editor settings.",
            ));
        }
        self.writer.try_send(value).map_err(|_| {
            error(
                "lspBusy",
                "Language server transport is unavailable or busy.",
            )
        })
    }
    fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_timeout(method, params, Duration::from_secs(15))
    }
    fn request_timeout(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| error("lspBusy", "Language server is busy."))?;
            if pending.len() >= 128 {
                return Err(error(
                    "lspBusy",
                    "Language server has too many pending requests.",
                ));
            }
            pending.insert(id, tx);
        }
        let result = self
            .send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .and_then(|_| {
                rx.recv_timeout(timeout)
                    .map_err(|_| error("lspTimeout", "Language server request timed out."))
            });
        if let Ok(mut p) = self.pending.lock() {
            p.remove(&id);
        }
        let value = result?;
        if value.get("error").is_some() {
            return Err(error("lspRemote", "Language server rejected this request."));
        }
        Ok(value["result"].clone())
    }
}
#[derive(Default)]
pub struct Servers {
    servers: Mutex<HashMap<String, Arc<Server>>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub executable: String,
    pub args: Vec<String>,
}
pub fn discover(executable: &str) -> Option<PathBuf> {
    let path = Path::new(executable);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_owned());
    }
    if executable.contains(['/', '\\']) {
        return None;
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .filter(|p| p.is_absolute())
        .flat_map(|p| {
            let plain = p.join(executable);
            #[cfg(windows)]
            {
                vec![
                    plain.clone(),
                    plain.with_extension("exe"),
                    plain.with_extension("cmd"),
                ]
            }
            #[cfg(not(windows))]
            {
                vec![plain]
            }
        })
        .find(|p| p.is_file())
}
pub fn uri(path: &Path) -> String {
    let s = crate::editor::root_label(path).replace('\\', "/");
    let mut result = if s.starts_with("//") {
        "file:".to_owned()
    } else if s.starts_with('/') {
        "file://".to_owned()
    } else {
        "file:///".to_owned()
    };
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
            result.push(b as char);
        } else {
            result.push_str(&format!("%{b:02X}"));
        }
    }
    result
}
fn frame(reader: &mut impl BufRead) -> std::io::Result<Value> {
    let mut length = None;
    let mut total = 0;
    loop {
        let mut line = String::new();
        let count = reader.take(8193).read_line(&mut line)?;
        total += count;
        if count == 0 || total > 8192 {
            return Err(std::io::Error::other("invalid LSP header"));
        }
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.trim().split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                if length.is_some() {
                    return Err(std::io::Error::other("duplicate length"));
                }
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let length = length
        .filter(|n| *n <= MAX_FRAME)
        .ok_or_else(|| std::io::Error::other("invalid LSP length"))?;
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}
fn write_frame(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FRAME {
        return Err(std::io::Error::other("LSP frame too large"));
    }
    write!(writer, "Content-Length: {}\r\n\r\n", bytes.len())?;
    writer.write_all(&bytes)?;
    writer.flush()
}
impl Servers {
    fn key(window: &str, root: &str, id: &str) -> String {
        serde_json::to_string(&(window, root, id)).unwrap_or_default()
    }
    fn get(&self, window: &str, root: &str, id: &str) -> Result<Arc<Server>> {
        self.servers
            .lock()
            .map_err(|_| error("lspBusy", "Language server registry is busy."))?
            .get(&Self::key(window, root, id))
            .cloned()
            .ok_or_else(|| {
                error(
                    "lspMissing",
                    "Start the language server in Editor settings.",
                )
            })
    }
    pub fn start(
        &self,
        window: &str,
        root: &Path,
        id: &str,
        config: Configuration,
    ) -> Result<Value> {
        let root = root.to_string_lossy().to_string();
        let key = Self::key(window, &root, id);
        if config.args.len() > 64 || config.args.iter().any(|a| a.len() > 4096) || id.len() > 80 {
            return Err(error("lspConfig", "Invalid language server configuration."));
        }
        let executable = discover(&config.executable)
            .ok_or_else(|| error("lspMissing", "Language server executable was not found."))?;
        let mut registry = self
            .servers
            .lock()
            .map_err(|_| error("lspBusy", "Language server registry is busy."))?;
        if registry.contains_key(&key) {
            return Err(error(
                "lspBusy",
                "Stop the existing language server before starting another.",
            ));
        }
        if registry.len() >= 16 {
            return Err(error("lspBusy", "At most 16 language servers may run."));
        }
        let mut command = Command::new(executable);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
            .args(config.args)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|_| error("lspSpawn", "Language server could not start."))?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let pending: Pending = Arc::default();
        let diagnostics: Arc<Mutex<HashMap<String, (usize, Value)>>> = Arc::default();
        let diagnostics_omitted = Arc::new(AtomicBool::new(false));
        let omitted = diagnostics_omitted.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::sync_channel::<Value>(128);
        let alive_writer = alive.clone();
        std::thread::spawn(move || {
            for value in rx {
                if write_frame(&mut stdin, &value).is_err() {
                    break;
                }
            }
            alive_writer.store(false, Ordering::SeqCst);
        });
        let (p, d, a, w) = (
            pending.clone(),
            diagnostics.clone(),
            alive.clone(),
            tx.clone(),
        );
        let root_uri = uri(Path::new(&root));
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(value) = frame(&mut reader) {
                if let Some(method) = value["method"].as_str() {
                    if value.get("id").is_some() {
                        let reply = match method {
                            "workspace/configuration" => {
                                json!({"result":value["params"]["items"].as_array().map(|items|items.iter().map(|_|Value::Null).collect::<Vec<_>>()).unwrap_or_default()})
                            }
                            "workspace/workspaceFolders" => {
                                json!({"result":[{"uri":root_uri,"name":"Project"}]})
                            }
                            "window/workDoneProgress/create" => json!({"result":null}),
                            _ => {
                                json!({"error":{"code":-32601,"message":"Client method unsupported"}})
                            }
                        };
                        let mut reply = reply;
                        reply["jsonrpc"] = json!("2.0");
                        reply["id"] = value["id"].clone();
                        let _ = w.try_send(reply);
                    } else if method == "textDocument/publishDiagnostics" {
                        if let Some(uri) = value["params"]["uri"].as_str() {
                            if let Ok(mut d) = d.lock() {
                                let size = serde_json::to_vec(&value["params"])
                                    .map(|v| v.len())
                                    .unwrap_or(MAX_FRAME);
                                let total: usize = d.values().map(|(size, _)| size).sum();
                                let old = d.get(uri).map(|(size, _)| *size).unwrap_or(0);
                                if uri.len() <= 8192
                                    && (d.len() < 128 || d.contains_key(uri))
                                    && total - old + size <= 4 * 1024 * 1024
                                {
                                    d.insert(uri.into(), (size, value["params"].clone()));
                                } else {
                                    omitted.store(true, Ordering::SeqCst);
                                }
                            }
                        }
                    }
                } else if let Some(id) = value["id"].as_u64() {
                    if let Ok(mut p) = p.lock() {
                        if let Some(tx) = p.remove(&id) {
                            let _ = tx.try_send(value);
                        }
                    }
                }
            }
            a.store(false, Ordering::SeqCst);
            if let Ok(mut p) = p.lock() {
                p.clear();
            }
        });
        let server = Arc::new(Server {
            child: Mutex::new(child),
            writer: tx,
            pending,
            diagnostics,
            diagnostics_omitted,
            next: AtomicU64::new(1),
            alive,
            root: root.clone(),
        });
        registry.insert(key.clone(), server.clone());
        drop(registry);
        let root_uri = uri(Path::new(&root));
        let initialized=server.request("initialize",json!({"processId":std::process::id(),"clientInfo":{"name":"CorpsTex","version":"0.1.0"},"rootUri":root_uri,"workspaceFolders":[{"uri":root_uri,"name":"Project"}],"capabilities":{"general":{"positionEncodings":["utf-16"]},"workspace":{"configuration":true,"workspaceFolders":true},"textDocument":{"synchronization":{"dynamicRegistration":false,"didSave":true},"completion":{"completionItem":{"snippetSupport":true,"documentationFormat":["plaintext","markdown"]}},"hover":{"contentFormat":["plaintext","markdown"]},"publishDiagnostics":{"versionSupport":true}}}}));
        match initialized {
            Ok(value) => {
                if let Err(e) =
                    server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}))
                {
                    self.stop(window, &root, id);
                    return Err(e);
                }
                Ok(value)
            }
            Err(e) => {
                self.stop(window, &root, id);
                Err(e)
            }
        }
    }
    pub fn request(
        &self,
        window: &str,
        root: &str,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        if !matches!(
            method,
            "textDocument/completion"
                | "completionItem/resolve"
                | "textDocument/hover"
                | "textDocument/definition"
        ) {
            return Err(error("lspMethod", "Unsupported editor LSP request."));
        }
        self.get(window, root, id)?.request(method, params)
    }
    pub fn notify(
        &self,
        window: &str,
        root: &str,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<()> {
        if !matches!(
            method,
            "textDocument/didOpen"
                | "textDocument/didChange"
                | "textDocument/didSave"
                | "textDocument/didClose"
        ) {
            return Err(error("lspMethod", "Unsupported editor LSP notification."));
        }
        self.get(window, root, id)?
            .send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    pub fn poll(&self, window: &str, root: &str, id: &str) -> Result<Value> {
        let s = self.get(window, root, id)?;
        let values: Vec<Value> = s
            .diagnostics
            .lock()
            .map_err(|_| error("lspBusy", "Language server diagnostics are busy."))?
            .drain()
            .map(|(_, (_, v))| v)
            .collect();
        Ok(
            json!({"alive":s.alive.load(Ordering::SeqCst),"diagnostics":values,"diagnosticsOmitted":s.diagnostics_omitted.swap(false,Ordering::SeqCst),"root":s.root}),
        )
    }
    pub fn stop(&self, window: &str, root: &str, id: &str) {
        let server = self
            .servers
            .lock()
            .ok()
            .and_then(|mut r| r.remove(&Self::key(window, root, id)));
        if let Some(server) = server {
            let _ = server.request_timeout("shutdown", Value::Null, Duration::from_millis(750));
            let _ = server.send(json!({"jsonrpc":"2.0","method":"exit","params":null}));
            if let Ok(mut child) = server.child.lock() {
                let deadline = std::time::Instant::now() + Duration::from_millis(750);
                while matches!(child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                kill_tree(&mut child);
            }
            server.alive.store(false, Ordering::SeqCst);
        }
    }
    pub fn stop_all(&self) {
        if let Ok(mut s) = self.servers.lock() {
            s.clear();
        }
    }
    pub fn stop_window(&self, window: &str) {
        if let Ok(mut servers) = self.servers.lock() {
            servers.retain(|key, _| {
                serde_json::from_str::<(String, String, String)>(key)
                    .map(|(w, _, _)| w != window)
                    .unwrap_or(false)
            });
        }
    }
}
#[derive(Serialize)]
pub struct Discovery {
    pub executable: String,
    pub path: Option<String>,
}
pub fn discoveries() -> Vec<Discovery> {
    [
        "typescript-language-server",
        "pyright-langserver",
        "rust-analyzer",
        "gopls",
        "clangd",
        "vscode-json-language-server",
        "yaml-language-server",
    ]
    .into_iter()
    .map(|e| Discovery {
        executable: e.into(),
        path: discover(e).map(|p| p.to_string_lossy().into()),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_is_byte_counted_bounded_and_strict() {
        let value = json!({"jsonrpc":"2.0","id":1,"result":"🦀"});
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &value).unwrap();
        assert_eq!(frame(&mut std::io::Cursor::new(bytes)).unwrap(), value);
        for bytes in [
            b"Content-Length: 999999999\r\n\r\n".as_slice(),
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
            b"Content-Length: 2\r\n\r\n{",
        ] {
            assert!(frame(&mut std::io::Cursor::new(bytes)).is_err());
        }
    }
    #[test]
    fn uris_escape_special_and_unicode_characters() {
        assert_eq!(
            uri(Path::new("/tmp/a #🦀.rs")),
            "file:///tmp/a%20%23%F0%9F%A6%80.rs"
        );
        assert!(discover("../unsafe/server").is_none());
    }
    #[test]
    #[ignore = "explicit installed clangd smoke in a temporary project; no downloads, credentials or inference"]
    fn installed_editor_lsp_smoke() {
        let executable = discover("clangd").expect("clangd is required for this explicit smoke");
        let version = Command::new(&executable).arg("--version").output().unwrap();
        println!(
            "{}",
            String::from_utf8_lossy(&version.stdout)
                .lines()
                .next()
                .unwrap_or("clangd version unavailable")
        );
        let directory =
            std::env::temp_dir().join(format!("corpstex-real-lsp-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let text =
            "int square(int value) { return value * value; }\nint main() { return square(2); }\n";
        let path = directory.join("example.cpp");
        std::fs::write(&path, text).unwrap();
        let root = directory.to_str().unwrap();
        let document = uri(&path);
        let registry = Servers::default();
        let init = registry
            .start(
                "smoke",
                &directory,
                "clangd",
                Configuration {
                    executable: executable.to_string_lossy().into(),
                    args: vec![
                        "--background-index=false".into(),
                        "--clang-tidy=false".into(),
                    ],
                },
            )
            .unwrap();
        assert!(init["capabilities"]["hoverProvider"].as_bool().unwrap());
        registry
            .notify(
                "smoke",
                root,
                "clangd",
                "textDocument/didOpen",
                json!({"textDocument":{"uri":document,"languageId":"cpp","version":1,"text":text}}),
            )
            .unwrap();
        let params = json!({"textDocument":{"uri":document},"position":{"line":1,"character":22}});
        let hover = registry
            .request(
                "smoke",
                root,
                "clangd",
                "textDocument/hover",
                params.clone(),
            )
            .unwrap();
        assert!(hover.to_string().contains("square"));
        let definition = registry
            .request("smoke", root, "clangd", "textDocument/definition", params)
            .unwrap();
        assert!(definition.to_string().contains("example.cpp"));
        let completion=registry.request("smoke",root,"clangd","textDocument/completion",json!({"textDocument":{"uri":document},"position":{"line":1,"character":23},"context":{"triggerKind":1}})).unwrap();
        assert!(completion.to_string().contains("square"));
        registry.notify("smoke",root,"clangd","textDocument/didChange",json!({"textDocument":{"uri":document,"version":2},"contentChanges":[{"text":"int main() { return missing_name; }\n"}]})).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut diagnostic = false;
        while std::time::Instant::now() < deadline {
            let poll = registry.poll("smoke", root, "clangd").unwrap();
            if poll.to_string().contains("missing_name") {
                diagnostic = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(diagnostic, "real clangd diagnostics were not received");
        registry
            .notify(
                "smoke",
                root,
                "clangd",
                "textDocument/didClose",
                json!({"textDocument":{"uri":document}}),
            )
            .unwrap();
        registry.stop("smoke", root, "clangd");
        assert!(registry.get("smoke", root, "clangd").is_err());
        std::fs::remove_dir_all(directory).unwrap();
        println!("real initialize, open, completion, hover, definition, change, diagnostics, close and shutdown passed");
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn fixture_lifecycle_capabilities_diagnostics_correlation_and_isolation() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "corpstex-lsp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("fixture.py");
        std::fs::write(&script,r#"#!/usr/bin/python3
import sys,json,os
def send(value):
 b=json.dumps(value,ensure_ascii=False).encode();sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(b)).encode()+b);sys.stdout.buffer.flush()
while True:
 length=None
 while True:
  line=sys.stdin.buffer.readline()
  if not line: sys.exit(0)
  if line==b'\r\n': break
  if line.lower().startswith(b'content-length:'):length=int(line.split(b':')[1])
 value=json.loads(sys.stdin.buffer.read(length));method=value.get('method');ident=value.get('id')
 if method=='initialize':result={'capabilities':{'textDocumentSync':2,'hoverProvider':True,'completionProvider':{},'definitionProvider':True},'fixturePid':os.getpid()}
 elif method=='textDocument/hover':result={'contents':{'kind':'plaintext','value':'hello 🦀'}}
 elif method=='textDocument/completion':result=[{'label':'fixture','kind':3,'insertText':'fixture()'}]
 elif method=='textDocument/definition':result={'uri':value['params']['textDocument']['uri'],'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}}
 elif method=='textDocument/didOpen':
  send({'jsonrpc':'2.0','method':'textDocument/publishDiagnostics','params':{'uri':value['params']['textDocument']['uri'],'version':1,'diagnostics':[{'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}},'message':'fixture diagnostic','severity':2}]}});continue
 elif method=='shutdown':result=None
 elif method=='exit':sys.exit(0)
 else:continue
 if ident is not None:send({'jsonrpc':'2.0','id':ident,'result':result})
"#).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let registry = Servers::default();
        let root = directory.to_str().unwrap();
        let config = Configuration {
            executable: script.to_string_lossy().into(),
            args: vec![],
        };
        let init = registry
            .start("main", &directory, "fixture", config)
            .unwrap();
        let pid = init["fixturePid"].as_u64().unwrap();
        assert!(registry
            .request("other", root, "fixture", "textDocument/hover", json!({}))
            .is_err());
        assert!(registry
            .request("main", "/other", "fixture", "textDocument/hover", json!({}))
            .is_err());
        assert!(registry
            .request(
                "main",
                root,
                "fixture",
                "workspace/executeCommand",
                json!({})
            )
            .is_err());
        let document = uri(&directory.join("a.rs"));
        let params = json!({"textDocument":{"uri":document},"position":{"line":0,"character":0}});
        registry
            .notify(
                "main",
                root,
                "fixture",
                "textDocument/didOpen",
                json!({"textDocument":{"uri":document,"version":1,"text":"a","languageId":"rust"}}),
            )
            .unwrap();
        assert_eq!(
            registry
                .request(
                    "main",
                    root,
                    "fixture",
                    "textDocument/hover",
                    params.clone()
                )
                .unwrap()["contents"]["value"],
            "hello 🦀"
        );
        assert_eq!(
            registry
                .request(
                    "main",
                    root,
                    "fixture",
                    "textDocument/completion",
                    params.clone()
                )
                .unwrap()[0]["label"],
            "fixture"
        );
        assert_eq!(
            registry
                .request("main", root, "fixture", "textDocument/definition", params)
                .unwrap()["uri"],
            document
        );
        let polled = registry.poll("main", root, "fixture").unwrap();
        assert_eq!(
            polled["diagnostics"][0]["diagnostics"][0]["message"],
            "fixture diagnostic"
        );
        assert_eq!(
            registry.poll("main", root, "fixture").unwrap()["diagnostics"],
            json!([])
        );
        registry.stop("main", root, "fixture");
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        assert!(registry.get("main", root, "fixture").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
