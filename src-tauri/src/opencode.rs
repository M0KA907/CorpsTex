//! Official ACP v1 adapter. OpenCode owns tools, configuration, auth and agent work.
//! No credential operations, shared daemon, or permission overrides.
use super::{
    approvals,
    conversations::{Action, Request},
    Command, ServerState, Status,
};
use crate::runtime::{self, Error, Process};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

type Reply = mpsc::SyncSender<Result<Value, Error>>;
#[derive(Debug)]
enum Kind {
    List,
    New { catalog: bool },
    Load(String),
    Config(String),
    SendConfig { action: Action, effort_stage: bool },
    Prompt { session: String, turn: String },
}
struct Pending {
    kind: Kind,
    reply: Option<Reply>,
    deadline: Option<Instant>,
}
#[derive(Default)]
struct Adapter {
    next: u64,
    capabilities: Value,
    configs: HashMap<String, Value>,
    loaded: HashSet<String>,
    reserved: HashSet<String>,
    pending: HashMap<u64, Pending>,
    catalog_session: Option<String>,
}
fn response(value: &Value) -> Result<Value, Error> {
    match (value.get("result"), value.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => match error["code"].as_i64().ok_or(Error::MalformedMessage)? {
            -32000 => Err(Error::OpenCodeAuthRequired),
            -32601 => Err(Error::OpenCodeUnsupported),
            code => Err(Error::Remote { code }),
        },
        _ => Err(Error::MalformedMessage),
    }
}
fn options(configs: &Value, id: &str) -> Vec<Value> {
    fn flatten(values: &[Value], out: &mut Vec<Value>) {
        for value in values {
            if let Some(group) = value["options"].as_array() {
                flatten(group, out);
            } else if value["value"].is_string() && value["name"].is_string() {
                out.push(value.clone());
            }
        }
    }
    let mut result = vec![];
    if let Some(config) = configs
        .as_array()
        .and_then(|v| v.iter().find(|c| c["id"] == id))
    {
        if let Some(values) = config["options"].as_array() {
            flatten(values, &mut result);
        }
    }
    result
}
fn current(configs: &Value, id: &str) -> Option<String> {
    configs.as_array()?.iter().find(|c| c["id"] == id)?["currentValue"]
        .as_str()
        .map(str::to_owned)
}
fn model_page(configs: &Value) -> Value {
    let default = current(configs, "model");
    let efforts: Vec<Value> = options(configs, "effort")
        .iter()
        .map(|o| json!({"reasoningEffort":o["value"]}))
        .collect();
    let data: Vec<Value> = options(configs, "model").iter().map(|o| {
        let id = o["value"].as_str().unwrap();
        let selected = Some(id) == default.as_deref();
        // ACP has labels, not prices. Never infer zero cost for unlabeled models.
        let free = id.ends_with(":free") || id.ends_with("-free") || o["name"].as_str().unwrap().to_lowercase().ends_with("(free)");
        json!({"model":id,"displayName":o["name"],"provider":id.split('/').next().unwrap_or("opencode"),"freeLabel":free,"isDefault":selected,"supportedReasoningEfforts":if selected {efforts.clone()} else {vec![]}})
    }).collect();
    json!({"data":data,"nextCursor":null,"configOptions":configs})
}
fn thread_value(id: &str, root: &str, title: Option<&str>) -> Value {
    json!({"id":id,"cwd":root,"name":title,"preview":"","turns":[]})
}
impl Adapter {
    fn rpc(
        &mut self,
        process: &mut Process,
        method: &str,
        params: Value,
        kind: Kind,
        reply: Option<Reply>,
    ) -> Result<(), Error> {
        self.next += 1;
        process.send(&json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params}))?;
        let deadline = if matches!(kind, Kind::Prompt { .. }) {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(55))
        };
        self.pending.insert(
            self.next,
            Pending {
                kind,
                reply,
                deadline,
            },
        );
        Ok(())
    }
    fn known(&self, state: &ServerState, id: &str) -> Result<(), Error> {
        if state.threads.contains_key(id) {
            Ok(())
        } else {
            Err(Error::InvalidRequest)
        }
    }
    fn config_valid(&self, id: &str, key: &str, value: &str) -> bool {
        self.configs
            .get(id)
            .is_some_and(|c| options(c, key).iter().any(|o| o["value"] == value))
    }
    fn action(
        &mut self,
        process: &mut Process,
        state: &mut ServerState,
        request: Request,
    ) -> Result<(), Error> {
        let Request { action, reply } = request;
        let checked = (|| -> Result<(), Error> {
            match &action {
                Action::Resume { thread_id }
                | Action::History { thread_id, .. }
                | Action::Send { thread_id, .. }
                | Action::Interrupt { thread_id, .. }
                | Action::OpenCodeConfig { thread_id, .. } => self.known(state, thread_id)?,
                _ => (),
            }
            match &action {
                Action::Resume { thread_id } if self.loaded.contains(thread_id) => {
                    // Re-opening a loaded thread is always allowed, even while its
                    // turn is waiting on an approval or otherwise active. Sending
                    // during an active turn is still rejected below.
                    return Ok(());
                }
                _ => (),
            }
            match &action {
                Action::OpenCodeConfig { thread_id, .. } | Action::Send { thread_id, .. }
                    if self.reserved.contains(thread_id)
                        || state.active.iter().any(|(id, _)| id == thread_id) =>
                {
                    return Err(Error::Busy);
                }
                Action::Resume { thread_id } if self.reserved.contains(thread_id) => {
                    return Err(Error::Busy);
                }
                _ => (),
            }
            if self.pending.len() >= 32 {
                return Err(Error::Busy);
            }
            Ok(())
        })();
        if let Err(e) = checked {
            let _ = reply.send(Err(e));
            return Ok(());
        }
        let root = state.snapshot.root.clone();
        match action {
            Action::ListThreads { cursor } => {
                if self.capabilities["sessionCapabilities"]["list"].is_object() {
                    self.rpc(
                        process,
                        "session/list",
                        json!({"cwd":root,"cursor":cursor}),
                        Kind::List,
                        Some(reply),
                    )?;
                } else {
                    let _ = reply.send(Err(Error::OpenCodeUnsupported));
                }
            }
            Action::ListModels { thread_id, .. } => {
                if thread_id
                    .as_ref()
                    .is_some_and(|id| !self.loaded.contains(id))
                {
                    let _ = reply.send(Err(Error::InvalidRequest));
                    return Ok(());
                }
                let id = thread_id.as_ref().or(self.catalog_session.as_ref());
                if let Some(config) = id.and_then(|id| self.configs.get(id)) {
                    let _ = reply.send(Ok(model_page(config)));
                } else {
                    self.rpc(
                        process,
                        "session/new",
                        json!({"cwd":root,"mcpServers":[]}),
                        Kind::New { catalog: true },
                        Some(reply),
                    )?;
                }
            }
            Action::NewThread => {
                if let Some(id) = self.catalog_session.take() {
                    let thread = state.threads.get(&id).ok_or(Error::InvalidRequest)?;
                    let _ = reply.send(Ok(json!({"thread":thread})));
                } else {
                    self.rpc(
                        process,
                        "session/new",
                        json!({"cwd":root,"mcpServers":[]}),
                        Kind::New { catalog: false },
                        Some(reply),
                    )?;
                }
            }
            Action::Resume { thread_id } => {
                if self.catalog_session.as_ref() == Some(&thread_id) {
                    self.catalog_session = None;
                }
                if self.loaded.contains(&thread_id) {
                    let _ = reply.send(Ok(json!({"thread":state.threads[&thread_id]})));
                } else if self.capabilities["loadSession"] == true {
                    self.reserved.insert(thread_id.clone());
                    state.threads.get_mut(&thread_id).unwrap()["turns"] = json!([]);
                    self.rpc(
                        process,
                        "session/load",
                        json!({"cwd":root,"sessionId":thread_id,"mcpServers":[]}),
                        Kind::Load(thread_id),
                        Some(reply),
                    )?;
                } else {
                    let _ = reply.send(Err(Error::OpenCodeUnsupported));
                }
            }
            Action::History { .. } => {
                let _ = reply.send(Ok(json!({"data":[],"nextCursor":null})));
            }
            Action::OpenCodeConfig {
                thread_id,
                config_id,
                value,
            } => {
                if !self.loaded.contains(&thread_id)
                    || !matches!(config_id.as_str(), "model" | "effort" | "mode")
                    || !self.config_valid(&thread_id, &config_id, &value)
                {
                    let _ = reply.send(Err(Error::InvalidRequest));
                } else {
                    self.reserved.insert(thread_id.clone());
                    self.rpc(
                        process,
                        "session/set_config_option",
                        json!({"sessionId":thread_id,"configId":config_id,"value":value}),
                        Kind::Config(thread_id),
                        Some(reply),
                    )?;
                }
            }
            action @ Action::Send { .. } => {
                let valid = self.validate_send(state, &action);
                if let Err(e) = valid {
                    let _ = reply.send(Err(e));
                } else if let Action::Send {
                    thread_id,
                    model: Some(model),
                    ..
                } = &action
                {
                    self.reserved.insert(thread_id.clone());
                    self.rpc(
                        process,
                        "session/set_config_option",
                        json!({"sessionId":thread_id,"configId":"model","value":model}),
                        Kind::SendConfig {
                            action,
                            effort_stage: false,
                        },
                        Some(reply),
                    )?;
                } else {
                    self.send_effort(process, state, action, reply)?;
                }
            }
            Action::Interrupt { thread_id, turn_id } => {
                if !state.active.contains(&(thread_id.clone(), turn_id)) {
                    let _ = reply.send(Err(Error::InvalidRequest));
                } else {
                    process.send(&json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":thread_id}}))?;
                    // Prompt response confirms completion. Never mark a turn stopped on write alone.
                    let _ = reply.send(Ok(json!({"sent":true})));
                }
            }
            _ => {
                let _ = reply.send(Err(Error::OpenCodeUnsupported));
            }
        }
        Ok(())
    }
    fn validate_send(&self, state: &ServerState, action: &Action) -> Result<(), Error> {
        let Action::Send {
            thread_id,
            text,
            images,
            skills,
            model,
            ..
        } = action
        else {
            return Err(Error::InvalidRequest);
        };
        if !self.loaded.contains(thread_id)
            || (text.trim().is_empty() && images.is_empty())
            || text.len() > 64 * 1024
            || !skills.is_empty()
            || images.len() > 8
            || images.iter().map(String::len).sum::<usize>() > crate::media::MAX_DATA
            || images.iter().any(|v| !crate::media::valid_data(v))
            || (!images.is_empty() && self.capabilities["promptCapabilities"]["image"] != true)
            || model
                .as_ref()
                .is_some_and(|m| !self.config_valid(thread_id, "model", m))
            || state.active.iter().any(|(id, _)| id == thread_id)
        {
            return Err(Error::InvalidRequest);
        }
        Ok(())
    }
    fn send_effort(
        &mut self,
        process: &mut Process,
        state: &mut ServerState,
        action: Action,
        reply: Reply,
    ) -> Result<(), Error> {
        let Action::Send {
            thread_id, effort, ..
        } = &action
        else {
            return Err(Error::InvalidRequest);
        };
        // Reset an earlier explicit effort when the owner chooses the runtime default.
        let desired = effort.clone().or_else(|| {
            self.config_valid(thread_id, "effort", "default")
                .then(|| "default".into())
        });
        if let Some(effort) = desired {
            if !self.config_valid(thread_id, "effort", &effort) {
                self.reserved.remove(thread_id);
                let _ = reply.send(Err(Error::InvalidRequest));
                return Ok(());
            }
            self.reserved.insert(thread_id.clone());
            self.rpc(
                process,
                "session/set_config_option",
                json!({"sessionId":thread_id,"configId":"effort","value":effort}),
                Kind::SendConfig {
                    action,
                    effort_stage: true,
                },
                Some(reply),
            )
        } else {
            self.prompt(process, state, action, reply)
        }
    }
    fn prompt(
        &mut self,
        process: &mut Process,
        state: &mut ServerState,
        action: Action,
        reply: Reply,
    ) -> Result<(), Error> {
        let Action::Send {
            thread_id,
            text,
            images,
            ..
        } = action
        else {
            return Err(Error::InvalidRequest);
        };
        if self.catalog_session.as_ref() == Some(&thread_id) {
            self.catalog_session = None;
        }
        let mut prompt = vec![json!({"type":"text","text":text})];
        for url in &images {
            let (mime, data) = url
                .strip_prefix("data:")
                .and_then(|v| v.split_once(";base64,"))
                .ok_or(Error::InvalidRequest)?;
            prompt.push(json!({"type":"image","mimeType":mime,"data":data}));
        }
        let turn = format!("acp-turn-{}", self.next + 1);
        self.rpc(
            process,
            "session/prompt",
            json!({"sessionId":thread_id,"prompt":prompt}),
            Kind::Prompt {
                session: thread_id.clone(),
                turn: turn.clone(),
            },
            None,
        )?;
        let mut content = vec![json!({"type":"text","text":text})];
        content.extend(images.iter().map(|url| json!({"type":"image","url":url})));
        let thread = state
            .threads
            .get_mut(&thread_id)
            .ok_or(Error::InvalidRequest)?;
        thread["turns"].as_array_mut().unwrap().push(json!({"id":turn,"status":"inProgress","items":[{"id":format!("{turn}-user"),"type":"userMessage","content":content,"clientGenerated":true}]}));
        if thread["preview"].as_str().unwrap_or("").is_empty() {
            thread["preview"] = json!(text.chars().take(80).collect::<String>());
        }
        state.active.insert((thread_id.clone(), turn.clone()));
        self.reserved.remove(&thread_id);
        state.snapshot.active_turns = state.active.len();
        state.conversation_revision += 1;
        let _ = reply.send(Ok(json!({"turn":{"id":turn,"status":"inProgress"}})));
        Ok(())
    }
    fn received(
        &mut self,
        process: &mut Process,
        state: &mut ServerState,
        value: Value,
    ) -> Result<(), Error> {
        if value["jsonrpc"] != "2.0" {
            return Err(Error::MalformedMessage);
        }
        if value["method"].is_string() {
            if value.get("result").is_some() || value.get("error").is_some() {
                return Err(Error::MalformedMessage);
            }
            if value.get("id").is_some() {
                // Keep requests verbatim and visibly pending. Unsupported requests have no decisions.
                state.record(&value.to_string())?;
            } else if value["method"] == "session/update" {
                let params = &value["params"];
                if params["update"]["sessionUpdate"] == "config_option_update" {
                    if let Some(id) = params["sessionId"]
                        .as_str()
                        .filter(|id| self.loaded.contains(*id))
                    {
                        if params["update"]["configOptions"].is_array() {
                            self.configs
                                .insert(id.into(), params["update"]["configOptions"].clone());
                        }
                    }
                } else {
                    update(state, params)?;
                }
                // Preserve the original ACP notification independently of transcript reduction.
                // Do not feed it through Codex's conversation/turn reducer.
                if params["sessionId"]
                    .as_str()
                    .is_some_and(|session| state.threads.contains_key(session))
                {
                    state.retain_event(value.clone(), value.to_string().len());
                }
            } else {
                // Unknown notifications remain inspectable in the transient event ring.
                state.record(&value.to_string())?;
            }
            return Ok(());
        }
        let id = value["id"].as_u64().ok_or(Error::MalformedMessage)?;
        let Pending { kind, reply, .. } =
            self.pending.remove(&id).ok_or(Error::UnsupportedProtocol)?;
        let result = response(&value);
        if let Err(error) = result {
            match &kind {
                Kind::Prompt { session, turn } => {
                    complete(state, session, turn, "failed", Some(&error))
                }
                Kind::Load(id) | Kind::Config(id) => {
                    self.reserved.remove(id);
                }
                Kind::SendConfig {
                    action: Action::Send { thread_id, .. },
                    ..
                } => {
                    self.reserved.remove(thread_id);
                }
                _ => (),
            }
            if let Some(reply) = reply {
                let _ = reply.send(Err(error));
            }
            return Ok(());
        }
        let result = result.unwrap();
        let output = match kind {
            Kind::List => {
                let mut data = vec![];
                for session in result["sessions"]
                    .as_array()
                    .ok_or(Error::MalformedMessage)?
                {
                    let cwd = session["cwd"].as_str().ok_or(Error::MalformedMessage)?;
                    if std::fs::canonicalize(cwd).ok().as_deref()
                        != Some(Path::new(&state.snapshot.root))
                    {
                        continue;
                    }
                    let id = session["sessionId"]
                        .as_str()
                        .ok_or(Error::MalformedMessage)?;
                    let mut thread =
                        thread_value(id, &state.snapshot.root, session["title"].as_str());
                    thread["updatedAt"] = session["updatedAt"].clone();
                    state
                        .threads
                        .entry(id.into())
                        .and_modify(|t| {
                            t["name"] = thread["name"].clone();
                        })
                        .or_insert_with(|| thread.clone());
                    data.push(thread);
                }
                json!({"data":data,"nextCursor":result["nextCursor"]})
            }
            Kind::New { catalog } => {
                let id = result["sessionId"]
                    .as_str()
                    .ok_or(Error::MalformedMessage)?
                    .to_owned();
                let config = result["configOptions"].clone();
                self.configs.insert(id.clone(), config.clone());
                self.loaded.insert(id.clone());
                let thread = thread_value(&id, &state.snapshot.root, None);
                state.threads.insert(id.clone(), thread.clone());
                if catalog {
                    self.catalog_session = Some(id);
                    model_page(&config)
                } else {
                    json!({"thread":thread})
                }
            }
            Kind::Load(id) => {
                self.reserved.remove(&id);
                self.loaded.insert(id.clone());
                self.configs
                    .insert(id.clone(), result["configOptions"].clone());
                json!({"thread":state.threads[&id]})
            }
            Kind::Config(id) => {
                self.reserved.remove(&id);
                self.configs.insert(id, result["configOptions"].clone());
                result
            }
            Kind::SendConfig {
                action,
                effort_stage,
            } => {
                let Action::Send { thread_id, .. } = &action else {
                    return Err(Error::InvalidRequest);
                };
                self.configs
                    .insert(thread_id.clone(), result["configOptions"].clone());
                let reply = reply.ok_or(Error::Transport)?;
                if effort_stage {
                    self.prompt(process, state, action, reply)?;
                } else {
                    self.send_effort(process, state, action, reply)?;
                }
                return Ok(());
            }
            Kind::Prompt { session, turn } => {
                let status = match result["stopReason"].as_str() {
                    Some("cancelled") => "interrupted",
                    Some("end_turn" | "max_tokens" | "max_turn_requests" | "refusal") => {
                        "completed"
                    }
                    _ => {
                        state.snapshot.uncertain = true;
                        complete(
                            state,
                            &session,
                            &turn,
                            "failed",
                            Some(&Error::OpenCodeUnsupported),
                        );
                        return Ok(());
                    }
                };
                complete(state, &session, &turn, status, None);
                return Ok(());
            }
        };
        state.conversation_revision += 1;
        if let Some(reply) = reply {
            let _ = reply.send(Ok(output));
        }
        Ok(())
    }
}
fn complete(state: &mut ServerState, session: &str, id: &str, status: &str, error: Option<&Error>) {
    if let Some(turn) = state
        .threads
        .get_mut(session)
        .and_then(|t| t["turns"].as_array_mut())
        .and_then(|v| v.iter_mut().find(|t| t["id"] == id))
    {
        turn["status"] = json!(status);
        if let Some(error) = error {
            turn["error"] = json!({"message":format!("OpenCode request failed: {error:?}")});
        }
    }
    // An ACP prompt response ends its permission waiters too; do not send decisions for them.
    let keys: Vec<String> = state
        .pending
        .iter()
        .filter(|(_, r)| r["params"]["sessionId"] == session)
        .map(|(k, _)| k.clone())
        .collect();
    for key in keys {
        state.pending.remove(&key);
        state.tickets.remove(&key);
    }
    state.active.remove(&(session.to_owned(), id.to_owned()));
    state.snapshot.active_turns = state.active.len();
    state.snapshot.pending_requests = state.pending.len();
    state.conversation_revision += 1;
}
fn update(state: &mut ServerState, params: &Value) -> Result<(), Error> {
    let session = params["sessionId"]
        .as_str()
        .ok_or(Error::MalformedMessage)?;
    // No foreign-session content is allowed into a subscribed project's transcript.
    let Some(thread) = state.threads.get_mut(session) else {
        return Ok(());
    };
    let u = &params["update"];
    let kind = u["sessionUpdate"].as_str().ok_or(Error::MalformedMessage)?;
    if kind == "session_info_update" {
        if u["title"].is_string() {
            thread["name"] = u["title"].clone();
        }
        state.conversation_revision += 1;
        return Ok(());
    }
    if matches!(
        kind,
        "available_commands_update" | "current_mode_update" | "usage_update"
    ) {
        thread[kind] = u.clone();
        state.conversation_revision += 1;
        return Ok(());
    }
    let turns = thread["turns"]
        .as_array_mut()
        .ok_or(Error::MalformedMessage)?;
    let live = turns.last().is_some_and(|t| t["status"] == "inProgress");
    let user = kind == "user_message_chunk";
    let message = u["messageId"].as_str();
    if !live
        && (turns.is_empty()
            || (user
                && !turns.last().unwrap()["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|i| message.is_some() && i["id"].as_str() == message)))
    {
        turns.push(
            json!({"id":format!("acp-history-{}",turns.len()),"status":"completed","items":[]}),
        );
    }
    let items = turns.last_mut().unwrap()["items"]
        .as_array_mut()
        .ok_or(Error::MalformedMessage)?;
    match kind {
        "user_message_chunk" | "agent_message_chunk" | "agent_thought_chunk" => {
            let ty = if user {
                "userMessage"
            } else if kind == "agent_thought_chunk" {
                "reasoning"
            } else {
                "agentMessage"
            };
            let id = message
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{ty}-{}", items.len()));
            // ACP v1 without message IDs groups consecutive chunks of the same type.
            if user && live {
                if let Some(at) = items.iter().position(|i| i["clientGenerated"] == true) {
                    items[at] = json!({"id":id,"type":"userMessage","content":[]});
                }
            }
            let at = items.iter().position(|i| i["id"] == id).or_else(|| {
                if message.is_none() {
                    items
                        .last()
                        .filter(|i| i["type"] == ty)
                        .map(|_| items.len() - 1)
                } else {
                    None
                }
            });
            let at = at.unwrap_or_else(|| {
                items.push(json!({"id":id,"type":ty,"text":"","content":[],"summary":[]}));
                items.len() - 1
            });
            let item = &mut items[at];
            if u["content"]["type"] == "text" {
                let text = u["content"]["text"]
                    .as_str()
                    .ok_or(Error::MalformedMessage)?;
                if user {
                    item["content"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"type":"text","text":text}));
                } else if ty == "reasoning" {
                    let old = item["summary"][0].as_str().unwrap_or("");
                    item["summary"] = json!([format!("{old}{text}")]);
                } else {
                    let old = item["text"].as_str().unwrap_or("");
                    item["text"] = json!(format!("{old}{text}"));
                }
            } else if u["content"]["type"] == "image" {
                let c = &u["content"];
                let url = format!(
                    "data:{};base64,{}",
                    c["mimeType"].as_str().unwrap_or(""),
                    c["data"].as_str().unwrap_or("")
                );
                if crate::media::valid_data(&url) {
                    if user {
                        item["content"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!({"type":"image","url":url}));
                    } else {
                        item["type"] = json!("dynamicToolCall");
                        item["contentItems"] = json!([{"type":"inputImage","imageUrl":url}]);
                    }
                }
            } else {
                item["reportedContent"] = u["content"].clone();
            }
        }
        "tool_call" | "tool_call_update" => {
            let id = u["toolCallId"].as_str().ok_or(Error::MalformedMessage)?;
            let at = items.iter().position(|i| i["id"] == id).unwrap_or_else(|| {
                items.push(json!({"id":id,"type":"opencodeTool","reported":{}}));
                items.len() - 1
            });
            for (key, value) in u.as_object().ok_or(Error::MalformedMessage)? {
                if !value.is_null() {
                    items[at]["reported"][key] = value.clone();
                }
            }
            items[at]["status"] = u["status"]
                .as_str()
                .map(|s| json!(s))
                .unwrap_or_else(|| items[at]["status"].clone());
        }
        _ => {
            items.push(
                json!({"id":format!("update-{}",items.len()),"type":"opencodeUpdate","reported":u}),
            );
        }
    }
    state.conversation_revision += 1;
    Ok(())
}
fn handshake(process: &mut Process, stop: &AtomicBool) -> Result<Value, Error> {
    process.send(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"corpstex","version":"0.1.0"}}}))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        if Instant::now() > deadline {
            return Err(Error::Timeout);
        }
        if let Some(line) = process.poll()? {
            let v: Value = serde_json::from_str(&line).map_err(|_| Error::MalformedMessage)?;
            if v.get("id").is_none() && v["method"].is_string() && v["jsonrpc"] == "2.0" {
                continue;
            }
            if v["id"] != 0 || v["jsonrpc"] != "2.0" {
                return Err(Error::OpenCodeUnsupported);
            }
            let init = response(&v)?;
            if init["protocolVersion"] != 1
                || init["agentInfo"]["name"]
                    .as_str()
                    .map(str::to_lowercase)
                    .as_deref()
                    != Some("opencode")
            {
                return Err(Error::OpenCodeUnsupported);
            }
            return Ok(init);
        }
    }
}
fn serve(
    process: &mut Process,
    state: &Arc<Mutex<ServerState>>,
    stop: &AtomicBool,
    commands: mpsc::Receiver<Command>,
    init: Value,
) -> Result<(), Error> {
    let mut adapter = Adapter {
        capabilities: init["agentCapabilities"].clone(),
        ..Adapter::default()
    };
    while !stop.load(Ordering::Relaxed) {
        for _ in 0..8 {
            match commands.try_recv() {
                Ok(Command::Conversation(request)) => adapter.action(
                    process,
                    &mut *state.lock().map_err(|_| Error::Transport)?,
                    request,
                )?,
                Ok(Command::Approval { reply, response }) => {
                    let result = approvals::respond(
                        process,
                        &mut *state.lock().map_err(|_| Error::Transport)?,
                        &reply,
                    );
                    let failed = result == Err(Error::Transport);
                    let _ = response.send(result);
                    if failed {
                        return Err(Error::Transport);
                    }
                }
                Err(_) => break,
            }
        }
        if adapter
            .pending
            .values()
            .any(|p| p.deadline.is_some_and(|d| d < Instant::now()))
        {
            return Err(Error::Timeout);
        }
        if let Some(line) = process.poll()? {
            let value: Value = serde_json::from_str(&line).map_err(|_| Error::MalformedMessage)?;
            adapter.received(
                process,
                &mut *state.lock().map_err(|_| Error::Transport)?,
                value,
            )?;
        }
    }
    Ok(())
}
pub(super) fn run(
    root: &Path,
    state: Arc<Mutex<ServerState>>,
    stop: Arc<AtomicBool>,
    spawn: Arc<super::Spawner>,
    commands: mpsc::Receiver<Command>,
) {
    let outcome = (|| {
        let (mut process, _) = spawn(root, &stop)?;
        let init = handshake(&mut process, &stop)?;
        {
            let mut s = state.lock().map_err(|_| Error::Transport)?;
            s.snapshot.status = Status::Ready;
            s.snapshot.pid = Some(process.pid());
            s.snapshot.version = init["agentInfo"]["version"].as_str().map(str::to_owned);
        }
        serve(&mut process, &state, &stop, commands, init)
    })();
    if let Ok(mut s) = state.lock() {
        s.snapshot.pid = None;
        s.snapshot.status = if stop.load(Ordering::Relaxed) {
            Status::Stopped
        } else {
            Status::Failed
        };
        s.snapshot.error = outcome.err();
    }
}

pub(super) fn spawn(root: &Path, stop: &AtomicBool) -> Result<(Process, String), Error> {
    if stop.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let path = std::env::var_os("PATH").ok_or(Error::OpenCodeUnavailable)?;
    let executable =
        runtime::discover_named(&path, "opencode").map_err(|_| Error::OpenCodeUnavailable)?;
    Ok((
        Process::spawn_at(&executable, &["acp"], Some(root))?,
        String::new(),
    ))
}

#[cfg(test)]
#[path = "opencode_tests.rs"]
mod tests;
