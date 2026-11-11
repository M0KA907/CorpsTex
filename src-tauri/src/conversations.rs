//! Narrow official-protocol actions; no shell, approval, or configuration escape hatch.
use super::{Error, Process, ServerState};
use crate::runtime;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    ListThreads {
        cursor: Option<String>,
    },
    ListArchivedThreads {
        cursor: Option<String>,
    },
    ListAllThreads {
        cursor: Option<String>,
    },
    ListMcpStatus {
        thread_id: String,
        cursor: Option<String>,
    },
    Fork {
        thread_id: String,
    },
    Rename {
        thread_id: String,
        name: String,
    },
    Archive {
        thread_id: String,
    },
    Unarchive {
        thread_id: String,
    },
    Compact {
        thread_id: String,
    },
    Review {
        thread_id: String,
        target: ReviewTarget,
    },
    ListModels {
        cursor: Option<String>,
        thread_id: Option<String>,
    },
    ListSkills {
        force_reload: bool,
    },
    ReadUsage {
        thread_id: String,
    },
    ReadLimits,
    ReadSettings,
    ReadRequirements,
    WriteSettings {
        expected_version: String,
        changes: Vec<SettingChange>,
    },
    OpenCodeConfig {
        thread_id: String,
        config_id: String,
        value: String,
    },
    NewThread,
    Resume {
        thread_id: String,
    },
    History {
        thread_id: String,
        cursor: Option<String>,
    },
    Send {
        thread_id: String,
        text: String,
        #[serde(default)]
        images: Vec<String>,
        #[serde(default)]
        skills: Vec<SkillInput>,
        model: Option<String>,
        effort: Option<String>,
    },
    Steer {
        thread_id: String,
        expected_turn_id: String,
        text: String,
        #[serde(default)]
        images: Vec<String>,
        #[serde(default)]
        skills: Vec<SkillInput>,
    },
    Interrupt {
        thread_id: String,
        turn_id: String,
    },
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ReviewTarget {
    UncommittedChanges,
    BaseBranch { branch: String },
    Commit { sha: String },
    Custom { instructions: String },
}

impl Action {
    // Mutations sharing a thread must not race before their runtime replies arrive.
    fn mutation_thread(&self) -> Option<&str> {
        match self {
            Self::Fork { thread_id }
            | Self::Rename { thread_id, .. }
            | Self::Archive { thread_id }
            | Self::Unarchive { thread_id }
            | Self::Compact { thread_id }
            | Self::Review { thread_id, .. }
            | Self::Send { thread_id, .. }
            | Self::Steer { thread_id, .. } => Some(thread_id),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingChange {
    key: String,
    value: String,
}

pub struct Request {
    pub action: Action,
    pub reply: mpsc::SyncSender<Result<Value, Error>>,
}
struct Pending {
    action: Action,
    reply: Option<mpsc::SyncSender<Result<Value, Error>>>,
    deadline: Instant,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillInput {
    name: String,
    path: String,
}
#[derive(Default)]
struct Catalog {
    config_version: Option<String>,
    models: HashMap<String, Vec<String>>,
    skills: HashMap<String, String>,
}
impl Catalog {
    fn new() -> Self {
        Self::default()
    }
}
fn skill_input(text: &str, skills: &[SkillInput], catalog: &Catalog) -> Result<Vec<Value>, Error> {
    if skills.len() > 32 {
        return Err(Error::InvalidRequest);
    }
    let mut input = vec![json!({"type":"text","text":text})];
    for skill in skills {
        if catalog.skills.get(&skill.path) != Some(&skill.name) {
            return Err(Error::InvalidRequest);
        }
        input.push(json!({"type":"skill","name":skill.name,"path":skill.path}));
    }
    Ok(input)
}

fn message_input(
    text: &str,
    skills: &[SkillInput],
    images: &[String],
    catalog: &Catalog,
) -> Result<Vec<Value>, Error> {
    if images.len() > 8
        || images.iter().map(String::len).sum::<usize>() > crate::media::MAX_DATA
        || images.iter().any(|url| !crate::media::valid_data(url))
    {
        return Err(Error::InvalidRequest);
    }
    let mut input = skill_input(text, skills, catalog)?;
    input.extend(images.iter().map(|url| json!({"type":"image","url":url})));
    Ok(input)
}

fn params(
    action: &Action,
    state: &ServerState,
    catalog: &Catalog,
) -> Result<(&'static str, Value), Error> {
    let root = &state.snapshot.root;
    let require_thread = |id: &String| {
        if state.threads.contains_key(id) {
            Ok(())
        } else {
            Err(Error::InvalidRequest)
        }
    };
    if let Some(id) = action.mutation_thread() {
        require_thread(&id.to_owned())?;
        if state.threads[id]["clientArchived"] == true
            && !matches!(action, Action::Unarchive { .. })
        {
            return Err(Error::InvalidRequest);
        }
        if !matches!(action, Action::Steer { .. })
            && state.active.iter().any(|(thread, _)| thread == id)
        {
            return Err(Error::Busy);
        }
        if matches!(action, Action::Archive { .. })
            && (!state.active.is_empty() || !state.pending.is_empty() || state.snapshot.uncertain)
        {
            // Archive may include descendants. Do not hide outstanding work or approvals.
            return Err(Error::Busy);
        }
    }
    Ok(match action {
        Action::ListThreads { cursor }
        | Action::ListArchivedThreads { cursor }
        | Action::ListAllThreads { cursor } => (
            "thread/list",
            json!({"cwd":root,"limit":30,"cursor":cursor,"archived":matches!(action, Action::ListArchivedThreads { .. }),"modelProviders":[],"sourceKinds":if matches!(action, Action::ListThreads {..}) {json!(["cli","vscode","appServer"])} else {json!(["cli","vscode","exec","appServer","subAgent","subAgentReview","subAgentCompact","subAgentThreadSpawn","subAgentOther","unknown"])},"sortKey":"updated_at"}),
        ),
        Action::ListMcpStatus { thread_id, cursor } => {
            require_thread(thread_id)?;
            (
                "mcpServerStatus/list",
                json!({"threadId":thread_id,"cursor":cursor,"limit":30,"detail":"toolsAndAuthOnly"}),
            )
        }
        Action::Fork { thread_id } => (
            "thread/fork",
            json!({"threadId":thread_id,"excludeTurns":true}),
        ),
        Action::Rename { thread_id, name } => {
            if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err(Error::InvalidRequest);
            }
            ("thread/name/set", json!({"threadId":thread_id,"name":name}))
        }
        Action::Archive { thread_id } => ("thread/archive", json!({"threadId":thread_id})),
        Action::Unarchive { thread_id } => ("thread/unarchive", json!({"threadId":thread_id})),
        Action::Compact { thread_id } => ("thread/compact/start", json!({"threadId":thread_id})),
        Action::Review { thread_id, target } => {
            let bounded = |text: &str, limit| {
                !text.trim().is_empty() && text.len() <= limit && !text.contains('\0')
            };
            let target = match target {
                ReviewTarget::UncommittedChanges => json!({"type":"uncommittedChanges"}),
                ReviewTarget::BaseBranch { branch } if bounded(branch, 512) => {
                    json!({"type":"baseBranch","branch":branch})
                }
                ReviewTarget::Commit { sha }
                    if (7..=64).contains(&sha.len())
                        && sha.bytes().all(|c| c.is_ascii_hexdigit()) =>
                {
                    json!({"type":"commit","sha":sha,"title":null})
                }
                ReviewTarget::Custom { instructions } if bounded(instructions, 64 * 1024) => {
                    json!({"type":"custom","instructions":instructions})
                }
                _ => return Err(Error::InvalidRequest),
            };
            (
                "review/start",
                json!({"threadId":thread_id,"target":target,"delivery":"inline"}),
            )
        }
        Action::ListModels { cursor, .. } => ("model/list", json!({"limit":100,"cursor":cursor})),
        Action::ListSkills { force_reload } => (
            "skills/list",
            json!({"cwds":[root],"forceReload":force_reload}),
        ),
        Action::ReadUsage { thread_id } => {
            require_thread(thread_id)?;
            ("account/usage/read", json!({"threadId":thread_id}))
        }
        Action::ReadLimits => ("account/rateLimits/read", json!({})),
        Action::ReadSettings => ("config/read", json!({"cwd":root,"includeLayers":true})),
        Action::ReadRequirements => ("configRequirements/read", json!({})),
        Action::WriteSettings {
            expected_version,
            changes,
        } => {
            if expected_version.len() > 256
                || catalog.config_version.as_ref() != Some(expected_version)
                || changes.is_empty()
                || changes.len() > 3
            {
                return Err(Error::InvalidRequest);
            }
            let mut keys = std::collections::HashSet::new();
            let mut edits = Vec::new();
            for change in changes {
                let valid = match change.key.as_str() {
                    "approval_policy" => {
                        matches!(change.value.as_str(), "untrusted" | "on-request" | "never")
                    }
                    "sandbox_mode" => matches!(
                        change.value.as_str(),
                        "read-only" | "workspace-write" | "danger-full-access"
                    ),
                    "web_search" => matches!(change.value.as_str(), "disabled" | "cached" | "live"),
                    _ => false,
                };
                if !valid || !keys.insert(&change.key) {
                    return Err(Error::InvalidRequest);
                }
                edits.push(
                    json!({"keyPath":change.key,"value":change.value,"mergeStrategy":"replace"}),
                );
            }
            (
                "config/batchWrite",
                json!({"expectedVersion":expected_version,"reloadUserConfig":false,"edits":edits}),
            )
        }
        Action::OpenCodeConfig { .. } => return Err(Error::OpenCodeUnsupported),
        Action::NewThread => ("thread/start", json!({"cwd":root})),
        Action::Resume { thread_id } => {
            require_thread(thread_id)?;
            // Resuming can unarchive native history. Restoration must be explicit.
            if state.threads[thread_id]["clientArchived"] == true {
                return Err(Error::InvalidRequest);
            }
            (
                "thread/resume",
                json!({"threadId":thread_id,"excludeTurns":true}),
            )
        }
        Action::History { thread_id, cursor } => {
            require_thread(thread_id)?;
            (
                "thread/turns/list",
                json!({"threadId":thread_id,"cursor":cursor,"limit":30,"sortDirection":"desc","itemsView":"full"}),
            )
        }
        Action::Send {
            thread_id,
            text,
            images,
            skills,
            model,
            effort,
        } => {
            require_thread(thread_id)?;
            if (text.trim().is_empty() && images.is_empty())
                || text.len() > 64 * 1024
                || state.active.iter().any(|(id, _)| id == thread_id)
            {
                return Err(Error::InvalidRequest);
            }
            if let Some(model) = model {
                if !catalog.models.contains_key(model) {
                    return Err(Error::InvalidRequest);
                }
            }
            if let Some(effort) = effort {
                if !model
                    .as_ref()
                    .and_then(|model| catalog.models.get(model))
                    .is_some_and(|list| list.contains(effort))
                {
                    return Err(Error::InvalidRequest);
                }
            }
            let mut params =
                json!({"threadId":thread_id,"input":message_input(text, skills, images, catalog)?});
            if let Some(model) = model {
                params["model"] = json!(model);
            }
            if let Some(effort) = effort {
                params["effort"] = json!(effort);
            }
            ("turn/start", params)
        }
        Action::Steer {
            thread_id,
            expected_turn_id,
            text,
            images,
            skills,
        } => {
            require_thread(thread_id)?;
            if (text.trim().is_empty() && images.is_empty())
                || text.len() > 64 * 1024
                || !state
                    .active
                    .contains(&(thread_id.clone(), expected_turn_id.clone()))
            {
                return Err(Error::InvalidRequest);
            }
            (
                "turn/steer",
                json!({"threadId":thread_id,"expectedTurnId":expected_turn_id,"input":message_input(text, skills, images, catalog)?}),
            )
        }
        Action::Interrupt { thread_id, turn_id } => {
            require_thread(thread_id)?;
            if !state.active.contains(&(thread_id.clone(), turn_id.clone())) {
                return Err(Error::InvalidRequest);
            }
            (
                "turn/interrupt",
                json!({"threadId":thread_id,"turnId":turn_id}),
            )
        }
    })
}

fn same_root(thread: &Value, root: &str) -> bool {
    thread["cwd"]
        .as_str()
        .and_then(|cwd| std::fs::canonicalize(cwd).ok())
        .is_some_and(|path| path == std::path::Path::new(root))
}
fn hydrate(state: &mut ServerState, thread: &Value) -> Result<(), Error> {
    let id = thread["id"].as_str().ok_or(Error::MalformedMessage)?;
    if !same_root(thread, &state.snapshot.root) {
        return Err(Error::InvalidProject);
    }
    let mut thread = thread.clone();
    if let Some(previous) = state.threads.get(id) {
        thread["turns"] = previous["turns"].clone();
        if thread.get("clientArchived").is_none() && previous.get("clientArchived").is_some() {
            thread["clientArchived"] = previous["clientArchived"].clone();
        }
        if previous.get("tokenUsage").is_some() {
            thread["tokenUsage"] = previous["tokenUsage"].clone();
        }
    }
    if !thread["turns"].is_array() {
        thread["turns"] = json!([]);
    }
    state.threads.insert(id.to_owned(), thread);
    state.invalidate_conversation(id);
    Ok(())
}
fn apply(
    action: &Action,
    result: &mut Value,
    state: &mut ServerState,
    catalog: &mut Catalog,
) -> Result<(), Error> {
    // These replies can mutate an existing transcript, including late replies to
    // already completed turns. Hydration invalidates each returned thread below.
    match action {
        Action::Rename { thread_id, .. }
        | Action::Archive { thread_id }
        | Action::Unarchive { thread_id }
        | Action::Review { thread_id, .. }
        | Action::History { thread_id, .. }
        | Action::Send { thread_id, .. } => state.invalidate_conversation(thread_id),
        _ => {}
    }
    if matches!(
        action,
        Action::Rename { .. } | Action::Archive { .. } | Action::Compact { .. }
    ) && !result.is_object()
    {
        return Err(Error::MalformedMessage);
    }
    match action {
        Action::ListThreads { .. }
        | Action::ListArchivedThreads { .. }
        | Action::ListAllThreads { .. } => {
            let data = result["data"].as_array().ok_or(Error::MalformedMessage)?;
            let mut visible = Vec::new();
            for thread in data {
                if same_root(thread, &state.snapshot.root) {
                    let mut thread = thread.clone();
                    thread["clientArchived"] =
                        json!(matches!(action, Action::ListArchivedThreads { .. }));
                    hydrate(state, &thread)?;
                    visible.push(thread);
                }
            }
            result["data"] = json!(visible);
        }
        Action::ListMcpStatus { .. } => {
            let data = result["data"].as_array().ok_or(Error::MalformedMessage)?;
            let mut visible = Vec::new();
            for server in data {
                let name = server["name"].as_str().ok_or(Error::MalformedMessage)?;
                let tools = server["tools"].as_object().ok_or(Error::MalformedMessage)?;
                let names: Vec<&str> = tools.keys().map(String::as_str).collect();
                // Do not forward endpoint, tool schemas, metadata, error strings or credentials.
                visible.push(json!({"name":name,"authStatus":server["authStatus"].as_str(),"runtimeStatus":server["runtimeStatus"].as_str(),"toolNames":names,"toolsUnavailable":!server["toolsError"].is_null()}));
            }
            let cursor = &result["nextCursor"];
            if !cursor.is_null() && !cursor.is_string() {
                return Err(Error::MalformedMessage);
            }
            *result = json!({"data":visible,"nextCursor":cursor});
        }
        Action::ListModels { cursor, .. } => {
            let data = result["data"].as_array().ok_or(Error::MalformedMessage)?;
            if cursor.is_none() {
                catalog.models.clear();
            }
            for model in data {
                if let (Some(name), Some(options)) = (
                    model["model"].as_str(),
                    model["supportedReasoningEfforts"].as_array(),
                ) {
                    catalog.models.insert(
                        name.to_owned(),
                        options
                            .iter()
                            .filter_map(|v| v["reasoningEffort"].as_str().map(str::to_owned))
                            .collect(),
                    );
                }
            }
        }
        Action::ListSkills { .. } => {
            let entries = result["data"].as_array().ok_or(Error::MalformedMessage)?;
            let mut skills = HashMap::new();
            let mut visible = Vec::new();
            for entry in entries {
                if entry["cwd"].as_str() != Some(state.snapshot.root.as_str()) {
                    continue;
                }
                for skill in entry["skills"].as_array().ok_or(Error::MalformedMessage)? {
                    let name = skill["name"].as_str().ok_or(Error::MalformedMessage)?;
                    let path = skill["path"].as_str().ok_or(Error::MalformedMessage)?;
                    if skill["enabled"] == true {
                        skills.insert(path.to_owned(), name.to_owned());
                    }
                }
                visible.push(entry.clone());
            }
            catalog.skills = skills;
            result["data"] = json!(visible);
        }
        Action::ReadUsage { thread_id } => {
            let usage = &result["threadUsage"];
            if !usage.is_null() && usage["threadId"].as_str() != Some(thread_id.as_str()) {
                return Err(Error::InvalidRequest);
            }
            *result = json!({"threadUsage": if usage.is_null() { Value::Null } else { json!({"threadId":thread_id,"nativeUsageUsdMicros":usage["nativeUsageUsdMicros"],"estimatedUsageUsdMicros":usage["estimatedUsageUsdMicros"]}) }});
        }
        Action::ReadLimits => {
            let buckets: Vec<&Value> = result["rateLimitsByLimitId"]
                .as_object()
                .filter(|map| !map.is_empty())
                .map(|map| map.values().collect())
                .unwrap_or_else(|| vec![&result["rateLimits"]]);
            let limits: Vec<Value> = buckets.into_iter().filter(|bucket|bucket.is_object()).map(|bucket|json!({"limitId":bucket["limitId"],"limitName":bucket["limitName"],"primary":bucket["primary"],"secondary":bucket["secondary"]})).collect();
            *result = json!({"limits":limits});
        }
        Action::ReadSettings => {
            if !result["config"].is_object() {
                return Err(Error::MalformedMessage);
            }
            let mut values = serde_json::Map::new();
            let mut origins = serde_json::Map::new();
            // Config can contain credentials and arbitrary provider fields. Never expose whole layers.
            for key in [
                "model",
                "model_provider",
                "model_reasoning_effort",
                "approval_policy",
                "sandbox_mode",
                "web_search",
            ] {
                let value = &result["config"][key];
                if value.is_null()
                    || value.is_string()
                    || (key == "approval_policy" && value.is_object())
                {
                    values.insert(
                        key.to_owned(),
                        if value.is_object() {
                            json!({"type":"custom"})
                        } else {
                            value.clone()
                        },
                    );
                }
                if let Some(kind) = result["origins"][key]["name"]["type"].as_str() {
                    origins.insert(key.to_owned(), json!(kind));
                }
            }
            let user = result["layers"].as_array().and_then(|layers| {
                layers.iter().find(|layer| {
                    layer["name"]["type"] == "user" && layer["name"]["profile"].is_null()
                })
            });
            catalog.config_version = user
                .and_then(|layer| layer["version"].as_str())
                .map(str::to_owned);
            *result = json!({"values":values,"origins":origins,"version":catalog.config_version});
        }
        Action::ReadRequirements => {
            let requirements = &result["requirements"];
            let filtered = if requirements.is_null() {
                Value::Null
            } else if requirements.is_object() {
                let mut filtered = serde_json::Map::new();
                for key in [
                    "allowedApprovalPolicies",
                    "allowedSandboxModes",
                    "allowedWebSearchModes",
                ] {
                    if requirements[key].is_array() || requirements[key].is_null() {
                        filtered.insert(key.to_owned(), requirements[key].clone());
                    }
                }
                Value::Object(filtered)
            } else {
                return Err(Error::MalformedMessage);
            };
            *result = json!({"requirements":filtered});
        }
        Action::WriteSettings { .. } => {
            if !matches!(result["status"].as_str(), Some("ok" | "okOverridden"))
                || !result["version"].is_string()
            {
                return Err(Error::MalformedMessage);
            }
            catalog.config_version = None;
            *result = json!({"status":result["status"],"version":result["version"]});
        }
        Action::Fork { thread_id } => {
            let id = result["thread"]["id"]
                .as_str()
                .ok_or(Error::MalformedMessage)?;
            if id == thread_id || state.threads.contains_key(id) {
                return Err(Error::MalformedMessage);
            }
            hydrate(state, &result["thread"])?;
        }
        Action::Rename { thread_id, name } => {
            state
                .threads
                .get_mut(thread_id)
                .ok_or(Error::InvalidRequest)?["name"] = json!(name);
        }
        Action::Archive { thread_id } => {
            state
                .threads
                .get_mut(thread_id)
                .ok_or(Error::InvalidRequest)?["clientArchived"] = json!(true);
        }
        Action::Unarchive { thread_id } => {
            if result["thread"]["id"].as_str() != Some(thread_id) {
                return Err(Error::MalformedMessage);
            }
            hydrate(state, &result["thread"])?;
            state
                .threads
                .get_mut(thread_id)
                .ok_or(Error::InvalidRequest)?["clientArchived"] = json!(false);
        }
        Action::Review { thread_id, .. } => {
            if result["reviewThreadId"].as_str() != Some(thread_id)
                || !result["turn"]["id"].is_string()
                || !matches!(
                    result["turn"]["status"].as_str(),
                    Some("inProgress" | "completed" | "failed" | "interrupted")
                )
            {
                return Err(Error::MalformedMessage);
            }
            let turn = &result["turn"];
            let already_terminal =
                state.threads[thread_id]["turns"]
                    .as_array()
                    .is_some_and(|turns| {
                        turns.iter().any(|known| {
                            known["id"] == turn["id"]
                                && matches!(
                                    known["status"].as_str(),
                                    Some("completed" | "failed" | "interrupted")
                                )
                        })
                    });
            if !already_terminal {
                record(
                    &mut state.threads,
                    &json!({"method":if turn["status"] == "inProgress" {"turn/started"} else {"turn/completed"},"params":{"threadId":thread_id,"turn":turn}}),
                );
                let key = (thread_id.clone(), turn["id"].as_str().unwrap().to_owned());
                if turn["status"] == "inProgress" {
                    state.active.insert(key);
                } else {
                    state.active.remove(&key);
                }
                state.snapshot.active_turns = state.active.len();
            }
        }
        Action::Compact { .. } => {} // Acceptance is not completion; official events own progress.
        Action::NewThread | Action::Resume { .. } => hydrate(state, &result["thread"])?,
        Action::History { thread_id, .. } => {
            let turns = result["data"].as_array().ok_or(Error::MalformedMessage)?;
            let thread = state
                .threads
                .get_mut(thread_id)
                .ok_or(Error::InvalidRequest)?;
            let existing = thread["turns"]
                .as_array_mut()
                .ok_or(Error::MalformedMessage)?;
            // Older pages arrive newest-first. Prepend only missing turns; never overwrite live events.
            for turn in turns {
                if !existing.iter().any(|known| known["id"] == turn["id"]) {
                    existing.insert(0, turn.clone());
                }
            }
            for turn in existing
                .iter()
                .filter(|turn| turn["status"] == "inProgress")
            {
                if let Some(id) = turn["id"].as_str() {
                    state.active.insert((thread_id.clone(), id.to_owned()));
                }
            }
            state.snapshot.active_turns = state.active.len();
        }
        Action::Send {
            thread_id,
            text,
            images,
            ..
        } => {
            let turn = &result["turn"];
            if turn["id"].as_str().is_none() {
                return Err(Error::MalformedMessage);
            }
            let terminal = state
                .threads
                .get(thread_id)
                .and_then(|thread| thread["turns"].as_array())
                .is_some_and(|turns| {
                    turns.iter().any(|known| {
                        known["id"] == turn["id"]
                            && matches!(
                                known["status"].as_str(),
                                Some("completed" | "failed" | "interrupted")
                            )
                    })
                });
            if !terminal {
                record(
                    &mut state.threads,
                    &json!({"method":"turn/started","params":{"threadId":thread_id,"turn":turn}}),
                );
            }
            if let Some(items) = state
                .threads
                .get_mut(thread_id)
                .and_then(|thread| thread["turns"].as_array_mut())
                .and_then(|turns| turns.iter_mut().find(|cached| cached["id"] == turn["id"]))
                .and_then(|turn| turn["items"].as_array_mut())
            {
                if !items.iter().any(|item| item["type"] == "userMessage") {
                    let mut content = vec![json!({"type":"text","text":text})];
                    content.extend(images.iter().map(|url| json!({"type":"image","url":url})));
                    items.insert(0,json!({"id":format!("corpstex-submitted:{}",turn["id"].as_str().unwrap()),"type":"userMessage","clientGenerated":true,"content":content}));
                }
            }
            if terminal {
                return Ok(());
            }
            if turn["status"] == "inProgress" {
                state
                    .active
                    .insert((thread_id.clone(), turn["id"].as_str().unwrap().to_owned()));
                state.snapshot.active_turns = state.active.len();
            }
        }
        Action::Steer {
            expected_turn_id, ..
        } => {
            if result["turnId"].as_str() != Some(expected_turn_id.as_str()) {
                return Err(Error::MalformedMessage);
            }
            // Only official notifications update the transcript; acceptance is not completion.
        }
        Action::OpenCodeConfig { .. } | Action::Interrupt { .. } => {} // Only turn/completed confirms interruption.
    }
    Ok(())
}

pub fn serve(
    process: &mut Process,
    state: &Arc<Mutex<ServerState>>,
    stop: &AtomicBool,
    commands: mpsc::Receiver<super::Command>,
) -> Result<(), Error> {
    let mut pending: HashMap<u64, Pending> = HashMap::new();
    let mut catalog = Catalog::new();
    let mut id = 1;
    while !stop.load(Ordering::Relaxed) {
        for _ in 0..8 {
            let request = match commands.try_recv() {
                Ok(super::Command::Conversation(request)) => request,
                Ok(super::Command::Approval { reply, response }) => {
                    let result = super::approvals::respond(
                        process,
                        &mut *state.lock().map_err(|_| Error::Transport)?,
                        &reply,
                    );
                    let transport_failed = result == Err(Error::Transport);
                    let _ = response.send(result);
                    if transport_failed {
                        return Err(Error::Transport);
                    }
                    continue;
                }
                Err(_) => break,
            };
            let duplicate_send = pending.values().any(|pending| {
                let thread = request.action.mutation_thread();
                (thread.is_some() && thread == pending.action.mutation_thread())
                    || (matches!(request.action, Action::Archive { .. })
                        && pending.action.mutation_thread().is_some())
                    || (matches!(pending.action, Action::Archive { .. }) && thread.is_some())
            });
            if pending.len() >= 32 || duplicate_send {
                let _ = request.reply.send(Err(Error::Busy));
                continue;
            }
            let parsed = params(
                &request.action,
                &*state.lock().map_err(|_| Error::Transport)?,
                &catalog,
            );
            let (method, params) = match parsed {
                Ok(parsed) => parsed,
                Err(error) => {
                    let _ = request.reply.send(Err(error));
                    continue;
                }
            };
            id += 1;
            process.send(&json!({"id":id,"method":method,"params":params}))?;
            pending.insert(
                id,
                Pending {
                    action: request.action,
                    reply: Some(request.reply),
                    deadline: Instant::now() + Duration::from_secs(60),
                },
            );
        }
        if let Some(line) = process.poll()? {
            let message: Value =
                serde_json::from_str(&line).map_err(|_| Error::MalformedMessage)?;
            let mut state = state.lock().map_err(|_| Error::Transport)?;
            if message.get("method").is_some() {
                if message["method"] == "skills/changed" {
                    catalog.skills.clear();
                }
                state.record(&line)?;
            } else {
                let id = message["id"].as_u64().ok_or(Error::UnsupportedProtocol)?;
                let request = pending.remove(&id).ok_or(Error::UnsupportedProtocol)?;
                let result = runtime::response(&message, id).and_then(|mut result| {
                    apply(&request.action, &mut result, &mut state, &mut catalog)?;
                    Ok(result)
                });
                if let Some(reply) = request.reply {
                    let _ = reply.send(result);
                }
            }
        }
        for request in pending.values_mut() {
            if Instant::now() >= request.deadline {
                if let Some(reply) = request.reply.take() {
                    let _ = reply.send(Err(Error::Timeout));
                }
            }
        }
    }
    Ok(())
}

const LIVE_OUTPUT_BYTES: usize = 1024 * 1024;

fn record_activity(threads: &mut HashMap<String, Value>, message: &Value) -> bool {
    let method = message["method"].as_str().unwrap_or("");
    if !matches!(
        method,
        "turn/plan/updated"
            | "turn/diff/updated"
            | "item/commandExecution/outputDelta"
            | "item/fileChange/outputDelta"
            | "item/fileChange/patchUpdated"
            | "item/mcpToolCall/progress"
    ) {
        return false;
    }
    let p = &message["params"];
    let (Some(thread), Some(turn_id)) = (
        p["threadId"].as_str().and_then(|id| threads.get_mut(id)),
        p["turnId"].as_str(),
    ) else {
        return true;
    };
    let Some(turns) = thread["turns"].as_array_mut() else {
        return true;
    };
    // Do not infer an active turn from a plan, diff or output fragment.
    let index = turns
        .iter()
        .position(|turn| turn["id"] == turn_id)
        .unwrap_or_else(|| {
            turns.push(json!({"id":turn_id,"status":"unknown","items":[]}));
            turns.len() - 1
        });
    let turn = &mut turns[index];
    if method == "turn/plan/updated" {
        if p["plan"].as_array().is_some_and(|steps| {
            steps
                .iter()
                .all(|step| step["step"].is_string() && step["status"].is_string())
        }) {
            turn["clientPlan"] = json!({"explanation":p["explanation"].as_str(),"steps":p["plan"]});
        }
    } else if method == "turn/diff/updated" {
        if p["diff"].is_string() {
            turn["clientDiff"] = p["diff"].clone();
        }
    } else {
        let Some(item_id) = p["itemId"].as_str() else {
            return true;
        };
        let Some(items) = turn["items"].as_array_mut() else {
            return true;
        };
        let index = items
            .iter()
            .position(|item| item["id"] == item_id)
            .unwrap_or_else(|| {
                let kind = if method.starts_with("item/commandExecution/") {
                    "commandExecution"
                } else if method.starts_with("item/fileChange/") {
                    "fileChange"
                } else {
                    "mcpToolCall"
                };
                items.push(json!({"id":item_id,"type":kind}));
                items.len() - 1
            });
        let item = &mut items[index];
        if item["clientActivityStatus"] == "completed"
            || matches!(
                item["status"].as_str(),
                Some("completed" | "failed" | "declined")
            )
        {
            return true;
        }
        if method == "item/mcpToolCall/progress" {
            if p["message"].is_string() {
                item["clientProgress"] = p["message"].clone();
            }
        } else if method == "item/fileChange/patchUpdated" {
            if p["changes"].is_array() {
                item["changes"] = p["changes"].clone();
            }
        } else if let Some(delta) = p["delta"].as_str() {
            let mut text = item["clientOutput"].as_str().unwrap_or("").to_owned();
            text.push_str(delta);
            let mut omitted = text.len().saturating_sub(LIVE_OUTPUT_BYTES);
            while !text.is_char_boundary(omitted) {
                omitted += 1;
            }
            let previous = item["clientOutputOmittedBytes"].as_u64().unwrap_or(0);
            item["clientOutput"] = json!(&text[omitted..]);
            item["clientOutputOmittedBytes"] = json!(previous.saturating_add(omitted as u64));
        }
    }
    true
}

pub fn record(threads: &mut HashMap<String, Value>, message: &Value) {
    if matches!(message["method"].as_str(), Some("error" | "warning")) {
        let p = &message["params"];
        if let Some(thread) = p["threadId"].as_str().and_then(|id| threads.get_mut(id)) {
            let text = if message["method"] == "error" {
                p["error"]["message"].as_str()
            } else {
                p["message"].as_str()
            };
            if let Some(text) = text {
                thread["clientRuntimeNotice"] = json!({"message":text,"kind":message["method"],"willRetry":p["willRetry"]==true});
            }
        }
        return;
    }
    if record_activity(threads, message) {
        return;
    }
    if let Some("thread/name/updated" | "thread/archived" | "thread/unarchived") =
        message["method"].as_str()
    {
        if let Some(thread) = message["params"]["threadId"]
            .as_str()
            .and_then(|id| threads.get_mut(id))
        {
            match message["method"].as_str() {
                Some("thread/name/updated") if message["params"]["threadName"].is_string() => {
                    thread["name"] = message["params"]["threadName"].clone()
                }
                Some("thread/archived") => thread["clientArchived"] = json!(true),
                Some("thread/unarchived") => thread["clientArchived"] = json!(false),
                _ => (),
            }
        }
        return;
    }
    if message["method"] == "thread/tokenUsage/updated" {
        if let Some(thread) = message["params"]["threadId"]
            .as_str()
            .and_then(|id| threads.get_mut(id))
        {
            let usage = &message["params"]["tokenUsage"];
            if usage["total"]["totalTokens"].as_u64().is_some() {
                thread["tokenUsage"] = usage.clone();
            }
        }
        return;
    }
    if !matches!(
        message["method"].as_str(),
        Some(
            "turn/started"
                | "turn/completed"
                | "item/started"
                | "item/completed"
                | "item/agentMessage/delta"
                | "item/reasoning/summaryTextDelta"
                | "item/reasoning/textDelta"
        )
    ) {
        return;
    }
    let params = &message["params"];
    let Some(thread_id) = params["threadId"].as_str() else {
        return;
    };
    let Some(thread) = threads.get_mut(thread_id) else {
        return;
    };
    let Some(turns) = thread["turns"].as_array_mut() else {
        return;
    };
    let method = message["method"].as_str().unwrap_or("");
    let turn_id = if method.starts_with("turn/") {
        params["turn"]["id"].as_str()
    } else {
        params["turnId"].as_str()
    };
    let Some(turn_id) = turn_id else { return };
    let index = turns
        .iter()
        .position(|turn| turn["id"] == turn_id)
        .unwrap_or_else(|| {
            turns.push(json!({"id":turn_id,"status":"inProgress","items":[]}));
            turns.len() - 1
        });
    let turn = &mut turns[index];
    if matches!(method, "turn/started" | "turn/completed") {
        let items = turn["items"].clone();
        let mut reported = params["turn"].clone();
        if reported["items"]
            .as_array()
            .is_none_or(|list| list.is_empty())
        {
            reported["items"] = items;
        } else if let Some(reported_items) = reported["items"].as_array_mut() {
            if !reported_items
                .iter()
                .any(|item| item["type"] == "userMessage")
            {
                for user in items
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["type"] == "userMessage")
                    .rev()
                {
                    reported_items.insert(0, user.clone());
                }
            }
        }
        for key in ["clientPlan", "clientDiff"] {
            if reported.get(key).is_none() && turn.get(key).is_some() {
                reported[key] = turn[key].clone();
            }
        }
        *turn = reported;
    } else if matches!(
        method,
        "item/started"
            | "item/completed"
            | "item/agentMessage/delta"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/textDelta"
    ) {
        let item_id = if method.ends_with("Delta") || method == "item/agentMessage/delta" {
            params["itemId"].as_str()
        } else {
            params["item"]["id"].as_str()
        };
        let Some(item_id) = item_id else { return };
        let Some(items) = turn["items"].as_array_mut() else {
            return;
        };
        let index = items
            .iter()
            .position(|item| item["id"] == item_id)
            .unwrap_or_else(|| {
                items.push(if method.starts_with("item/reasoning/") {
                    json!({"id":item_id,"type":"reasoning"})
                } else {
                    json!({"id":item_id,"type":"agentMessage","text":""})
                });
                items.len() - 1
            });
        if method == "item/agentMessage/delta" {
            if let Some(delta) = params["delta"].as_str() {
                let text = items[index]["text"].as_str().unwrap_or("").to_owned() + delta;
                items[index]["text"] = json!(text);
            }
        } else if method.starts_with("item/reasoning/") {
            let (field, position) = if method == "item/reasoning/summaryTextDelta" {
                ("summaryDelta", "summaryIndex")
            } else {
                ("contentDelta", "contentIndex")
            };
            if let (Some(part), Some(delta)) = (params[position].as_u64(), params["delta"].as_str())
            {
                // Keep indexed fragments in a map: a reported index must never allocate a sparse array.
                if items[index][field].is_null() {
                    items[index][field] = json!({});
                }
                let key = part.to_string();
                let text = items[index][field][&key].as_str().unwrap_or("").to_owned() + delta;
                items[index][field][key] = json!(text);
            }
        } else {
            let output = items[index]["clientOutput"].clone();
            let omitted = items[index]["clientOutputOmittedBytes"].clone();
            items[index] = params["item"].clone();
            if !output.is_null() && items[index]["aggregatedOutput"].is_null() {
                items[index]["clientOutput"] = output;
                items[index]["clientOutputOmittedBytes"] = omitted;
            }
            if params["item"]["type"] == "userMessage" {
                items.retain(|item| item["clientGenerated"] != true);
            }
        }
        if let Some(item) = items.iter_mut().find(|item| item["id"] == item_id) {
            item["clientActivityStatus"] = json!(if method == "item/completed" {
                "completed"
            } else {
                "inProgress"
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> ServerState {
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let mut state = ServerState::new(&root, 1);
        for id in ["one", "two"] {
            hydrate(&mut state, &json!({"id":id,"cwd":root,"turns":[]})).unwrap();
        }
        state
    }
    fn event(state: &mut ServerState, value: Value) {
        state.record(&value.to_string()).unwrap();
    }

    #[test]
    fn replies_and_notifications_invalidate_only_their_thread() {
        let mut state = state();
        let root = state.snapshot.root.clone();
        let idle_revision = state.conversation_revisions["two"];
        let mut catalog = Catalog::new();
        for (action, mut result, affected) in [
            (
                Action::NewThread,
                json!({"thread":{"id":"new","cwd":root}}),
                "new",
            ),
            (
                Action::Fork {
                    thread_id: "one".into(),
                },
                json!({"thread":{"id":"fork","cwd":root}}),
                "fork",
            ),
            (
                Action::Resume {
                    thread_id: "one".into(),
                },
                json!({"thread":{"id":"one","cwd":root}}),
                "one",
            ),
            (
                Action::ListThreads { cursor: None },
                json!({"data":[{"id":"one","cwd":root}]}),
                "one",
            ),
            (
                Action::Rename {
                    thread_id: "one".into(),
                    name: "renamed".into(),
                },
                json!({}),
                "one",
            ),
            (
                Action::Archive {
                    thread_id: "one".into(),
                },
                json!({}),
                "one",
            ),
            (
                Action::Unarchive {
                    thread_id: "one".into(),
                },
                json!({"thread":{"id":"one","cwd":root}}),
                "one",
            ),
            (
                Action::History {
                    thread_id: "one".into(),
                    cursor: None,
                },
                json!({"data":[{"id":"old","status":"completed","items":[]}]}),
                "one",
            ),
            (
                Action::Send {
                    thread_id: "one".into(),
                    text: "hi".into(),
                    images: vec![],
                    skills: vec![],
                    model: None,
                    effort: None,
                },
                json!({"turn":{"id":"sent","status":"inProgress","items":[]}}),
                "one",
            ),
            (
                Action::Review {
                    thread_id: "one".into(),
                    target: ReviewTarget::UncommittedChanges,
                },
                json!({"reviewThreadId":"one","turn":{"id":"review","status":"completed","items":[]}}),
                "one",
            ),
        ] {
            let before = state
                .conversation_revisions
                .get(affected)
                .copied()
                .unwrap_or(0);
            apply(&action, &mut result, &mut state, &mut catalog).unwrap();
            assert!(
                state.conversation_revisions[affected] > before,
                "{action:?}"
            );
            assert_eq!(
                state.conversation_revisions["two"], idle_revision,
                "{action:?}"
            );
        }
        let before = state.conversation_revisions["one"];
        for message in [
            json!({"method":"turn/started","params":{"threadId":"one","turn":{"id":"sent","status":"inProgress","items":[]}}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"one","turnId":"sent","itemId":"reply","delta":"final"}}),
            json!({"method":"turn/completed","params":{"threadId":"one","turn":{"id":"sent","status":"completed","items":[]}}}),
            json!({"method":"thread/name/updated","params":{"threadId":"one","threadName":"named"}}),
        ] {
            event(&mut state, message);
        }
        assert!(state.conversation_revisions["one"] > before);
        assert_eq!(state.conversation_revisions["two"], idle_revision);
        assert_eq!(state.threads["one"]["turns"][1]["status"], "completed");
        assert_eq!(
            state.threads["one"]["turns"][1]["items"][1]["text"],
            "final"
        );
        let revisions = state.conversation_revisions.clone();
        event(&mut state, json!({"method":"account/updated","params":{}}));
        event(
            &mut state,
            json!({"method":"future/projectEvent","params":{}}),
        );
        apply(
            &Action::ListModels {
                cursor: None,
                thread_id: None,
            },
            &mut json!({"data":[]}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert_eq!(state.conversation_revisions, revisions);
        assert_eq!(state.snapshot.sequence, 6);
    }

    #[test]
    fn plans_diffs_and_tool_fragments_preserve_lifecycle_authority_and_isolation() {
        let mut state = state();
        event(
            &mut state,
            json!({"method":"turn/plan/updated","params":{"threadId":"one","turnId":"work","explanation":"why","plan":[{"step":"inspect","status":"inProgress"}]}}),
        );
        assert!(state.active.is_empty());
        assert_eq!(state.threads["one"]["turns"][0]["status"], "unknown");
        event(
            &mut state,
            json!({"method":"turn/started","params":{"threadId":"one","turn":{"id":"work","status":"inProgress","items":[]}}}),
        );
        event(
            &mut state,
            json!({"method":"turn/diff/updated","params":{"threadId":"one","turnId":"work","diff":"+change"}}),
        );
        event(
            &mut state,
            json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"one","turnId":"work","itemId":"cmd","delta":"early "}}),
        );
        event(
            &mut state,
            json!({"method":"item/started","params":{"threadId":"one","turnId":"work","item":{"id":"cmd","type":"commandExecution","status":"inProgress","aggregatedOutput":null}}}),
        );
        event(
            &mut state,
            json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"one","turnId":"work","itemId":"cmd","delta":"output"}}),
        );
        assert_eq!(
            state.threads["one"]["turns"][0]["items"][0]["clientOutput"],
            "early output"
        );
        event(
            &mut state,
            json!({"method":"item/completed","params":{"threadId":"one","turnId":"work","item":{"id":"cmd","type":"commandExecution","status":"completed","aggregatedOutput":"official","exitCode":0}}}),
        );
        event(
            &mut state,
            json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"one","turnId":"work","itemId":"cmd","delta":"late"}}),
        );
        assert_eq!(
            state.threads["one"]["turns"][0]["items"][0]["aggregatedOutput"],
            "official"
        );
        assert!(state.threads["one"]["turns"][0]["items"][0]["clientOutput"].is_null());
        event(
            &mut state,
            json!({"method":"item/fileChange/patchUpdated","params":{"threadId":"one","turnId":"work","itemId":"file","changes":[{"path":"a","diff":"+b"}]}}),
        );
        event(
            &mut state,
            json!({"method":"item/mcpToolCall/progress","params":{"threadId":"one","turnId":"work","itemId":"mcp","message":"working"}}),
        );
        event(
            &mut state,
            json!({"method":"turn/completed","params":{"threadId":"one","turn":{"id":"work","status":"completed","items":[]}}}),
        );
        let turn = &state.threads["one"]["turns"][0];
        assert_eq!(turn["clientPlan"]["steps"][0]["step"], "inspect");
        assert_eq!(turn["clientDiff"], "+change");
        assert_eq!(turn["items"][1]["type"], "fileChange");
        assert_eq!(turn["items"][2]["clientProgress"], "working");
        for method in [
            "turn/diff/updated",
            "item/commandExecution/outputDelta",
            "item/mcpToolCall/progress",
        ] {
            event(
                &mut state,
                json!({"method":method,"params":{"threadId":"foreign","turnId":"work","itemId":"cmd","delta":"foreign","diff":"foreign","message":"foreign"}}),
            );
        }
        assert!(!state.threads.contains_key("foreign"));
        assert_eq!(state.threads["two"]["turns"], json!([]));
        assert!(state.active.is_empty());
        event(
            &mut state,
            json!({"method":"error","params":{"threadId":"one","turnId":"work","error":{"message":"retryable"},"willRetry":true}}),
        );
        assert_eq!(
            state.threads["one"]["clientRuntimeNotice"]["willRetry"],
            true
        );
        assert!(state.active.is_empty());
        event(
            &mut state,
            json!({"method":"warning","params":{"threadId":"two","message":"warning"}}),
        );
        assert_eq!(
            state.threads["two"]["clientRuntimeNotice"]["message"],
            "warning"
        );
    }

    #[test]
    fn live_command_output_is_utf8_bounded_with_explicit_omission_counts() {
        let mut state = state();
        let delta = "🦀".repeat(LIVE_OUTPUT_BYTES / 4 + 17);
        for count in 1..=3 {
            event(
                &mut state,
                json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"one","turnId":"work","itemId":"cmd","delta":delta}}),
            );
            let item = &state.threads["one"]["turns"][0]["items"][0];
            let output = item["clientOutput"].as_str().unwrap();
            assert_eq!(output.len(), LIVE_OUTPUT_BYTES);
            assert!(output.starts_with('🦀'));
            assert_eq!(
                item["clientOutputOmittedBytes"].as_u64().unwrap(),
                (count * delta.len() - LIVE_OUTPUT_BYTES) as u64
            );
        }
        assert!(state.active.is_empty());
    }

    #[test]
    fn mcp_inventory_is_thread_bound_and_excludes_credentials_schemas_and_raw_errors() {
        let mut state = state();
        let mut catalog = Catalog::new();
        let action = Action::ListMcpStatus {
            thread_id: "one".into(),
            cursor: Some("page".into()),
        };
        let (method, wire) = params(&action, &state, &catalog).unwrap();
        assert_eq!(method, "mcpServerStatus/list");
        assert_eq!(
            wire,
            json!({"threadId":"one","cursor":"page","limit":30,"detail":"toolsAndAuthOnly"})
        );
        assert_eq!(
            params(
                &Action::ListMcpStatus {
                    thread_id: "foreign".into(),
                    cursor: None
                },
                &state,
                &catalog
            ),
            Err(Error::InvalidRequest)
        );
        let mut result = json!({"data":[{"name":"fixture","runtimeStatus":"connected","authStatus":"notLoggedIn","tools":{"tool":{"inputSchema":{"secret":"SECRET"}}},"toolsError":"SECRET","httpOrigin":"SECRET","toolListMeta":{"token":"SECRET"},"resources":[{"uri":"SECRET"}]}],"nextCursor":"next"});
        apply(&action, &mut result, &mut state, &mut catalog).unwrap();
        assert_eq!(result["data"][0]["toolNames"], json!(["tool"]));
        assert_eq!(result["data"][0]["toolsUnavailable"], true);
        assert_eq!(result["nextCursor"], "next");
        assert!(!result.to_string().contains("SECRET"));
        assert!(apply(
            &action,
            &mut json!({"data":[{"name":"fixture","tools":[]}]}),
            &mut state,
            &mut catalog
        )
        .is_err());
        assert!(serde_json::from_value::<Action>(
            json!({"type":"listMcpStatus","threadId":"one","serverName":"override","cursor":null})
        )
        .is_err());
    }

    #[test]
    fn all_sources_are_explicit_and_subagent_history_cannot_cross_project_roots() {
        let mut state = state();
        let mut catalog = Catalog::new();
        let root = state.snapshot.root.clone();
        let action = Action::ListAllThreads { cursor: None };
        let (_, wire) = params(&action, &state, &catalog).unwrap();
        let sources = wire["sourceKinds"].as_array().unwrap();
        assert!(sources.contains(&json!("subAgentThreadSpawn")));
        assert!(sources.contains(&json!("exec")));
        assert_eq!(sources.len(), 10);
        let mut result = json!({"data":[{"id":"child","parentThreadId":"one","agentRole":"reviewer","cwd":root,"turns":[]},{"id":"foreign","parentThreadId":"one","cwd":"/corpstex-missing-project","turns":[]}]});
        apply(&action, &mut result, &mut state, &mut catalog).unwrap();
        assert_eq!(result["data"].as_array().unwrap().len(), 1);
        assert_eq!(state.threads["child"]["parentThreadId"], "one");
        assert!(!state.threads.contains_key("foreign"));
        assert!(params(
            &Action::Resume {
                thread_id: "child".into()
            },
            &state,
            &catalog
        )
        .is_ok());
    }

    #[test]
    fn thread_controls_validate_protocol_targets_and_project_ownership() {
        let state = state();
        let catalog = Catalog::new();
        for (action, method, expected) in [
            (
                json!({"type":"fork","threadId":"one"}),
                "thread/fork",
                json!({"threadId":"one","excludeTurns":true}),
            ),
            (
                json!({"type":"rename","threadId":"one","name":"Named"}),
                "thread/name/set",
                json!({"threadId":"one","name":"Named"}),
            ),
            (
                json!({"type":"archive","threadId":"one"}),
                "thread/archive",
                json!({"threadId":"one"}),
            ),
            (
                json!({"type":"unarchive","threadId":"one"}),
                "thread/unarchive",
                json!({"threadId":"one"}),
            ),
            (
                json!({"type":"compact","threadId":"one"}),
                "thread/compact/start",
                json!({"threadId":"one"}),
            ),
        ] {
            let parsed: Action = serde_json::from_value(action.clone()).unwrap();
            assert_eq!(
                params(&parsed, &state, &catalog).unwrap(),
                (method, expected)
            );
            let mut foreign = action;
            foreign["threadId"] = json!("foreign");
            assert_eq!(
                params(&serde_json::from_value(foreign).unwrap(), &state, &catalog),
                Err(Error::InvalidRequest)
            );
        }
        for target in [
            json!({"type":"uncommittedChanges"}),
            json!({"type":"baseBranch","branch":"main"}),
            json!({"type":"commit","sha":"abc1234"}),
            json!({"type":"custom","instructions":"Review carefully"}),
        ] {
            let action =
                serde_json::from_value(json!({"type":"review","threadId":"one","target":target}))
                    .unwrap();
            let (method, wire) = params(&action, &state, &catalog).unwrap();
            assert_eq!(method, "review/start");
            assert_eq!(wire["delivery"], "inline");
            assert_eq!(wire["target"]["type"], target["type"]);
            assert!(wire.get("approvalPolicy").is_none());
            assert!(wire.get("sandbox").is_none());
            if target["type"] == "commit" {
                assert_eq!(wire["target"]["title"], Value::Null);
            }
        }
        for action in [
            json!({"type":"rename","threadId":"one","name":" \n"}),
            json!({"type":"rename","threadId":"one","name":"x".repeat(257)}),
            json!({"type":"review","threadId":"one","target":{"type":"baseBranch","branch":""}}),
            json!({"type":"review","threadId":"one","target":{"type":"commit","sha":"not-a-sha"}}),
            json!({"type":"review","threadId":"one","target":{"type":"custom","instructions":"x".repeat(65537)}}),
        ] {
            assert!(params(&serde_json::from_value(action).unwrap(), &state, &catalog).is_err());
        }
        for action in [
            json!({"type":"fork","threadId":"one","sandbox":"danger-full-access"}),
            json!({"type":"review","threadId":"one","target":{"type":"custom","instructions":"ok","command":"x"}}),
        ] {
            assert!(serde_json::from_value::<Action>(action).is_err());
        }
    }

    #[test]
    fn archive_is_reversible_and_cannot_hide_active_work_or_approvals() {
        let mut state = state();
        let mut catalog = Catalog::new();
        let archive = Action::Archive {
            thread_id: "one".into(),
        };
        state.active.insert(("two".into(), "active".into()));
        assert_eq!(params(&archive, &state, &catalog), Err(Error::Busy));
        state.active.clear();
        state
            .pending
            .insert("approval".into(), json!({"params":{"threadId":"two"}}));
        assert_eq!(params(&archive, &state, &catalog), Err(Error::Busy));
        state.pending.clear();
        state.snapshot.uncertain = true;
        assert_eq!(params(&archive, &state, &catalog), Err(Error::Busy));
        state.snapshot.uncertain = false;
        assert_eq!(
            apply(&archive, &mut Value::Null, &mut state, &mut catalog),
            Err(Error::MalformedMessage)
        );
        assert!(state.threads["one"]["clientArchived"].is_null());
        apply(&archive, &mut json!({}), &mut state, &mut catalog).unwrap();
        assert_eq!(state.threads["one"]["clientArchived"], true);
        assert!(params(
            &Action::Resume {
                thread_id: "one".into()
            },
            &state,
            &catalog
        )
        .is_err());
        assert!(params(
            &Action::Compact {
                thread_id: "one".into()
            },
            &state,
            &catalog
        )
        .is_err());
        let unarchive = Action::Unarchive {
            thread_id: "one".into(),
        };
        assert!(params(&unarchive, &state, &catalog).is_ok());
        let root = state.snapshot.root.clone();
        assert!(apply(
            &unarchive,
            &mut json!({"thread":{"id":"two","cwd":root}}),
            &mut state,
            &mut catalog
        )
        .is_err());
        apply(
            &unarchive,
            &mut json!({"thread":{"id":"one","cwd":root,"turns":[]}}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert_eq!(state.threads["one"]["clientArchived"], false);
        event(
            &mut state,
            json!({"method":"thread/archived","params":{"threadId":"two"}}),
        );
        assert_eq!(state.threads["two"]["clientArchived"], true);
        event(
            &mut state,
            json!({"method":"thread/name/updated","params":{"threadId":"two","threadName":"<new>"}}),
        );
        assert_eq!(state.threads["two"]["name"], "<new>");
        assert!(state.threads["one"]["name"].is_null());
    }

    #[test]
    fn archived_listing_and_fork_preserve_history_without_cross_project_hydration() {
        let mut state = state();
        let mut catalog = Catalog::new();
        let root = state.snapshot.root.clone();
        let action = Action::ListArchivedThreads {
            cursor: Some("next".into()),
        };
        let (_, wire) = params(&action, &state, &catalog).unwrap();
        assert_eq!(wire["archived"], true);
        assert_eq!(wire["cursor"], "next");
        let mut page = json!({"data":[{"id":"saved","cwd":root,"turns":[]},{"id":"foreign","cwd":"/corpstex-missing-project"}]});
        apply(&action, &mut page, &mut state, &mut catalog).unwrap();
        assert_eq!(page["data"].as_array().unwrap().len(), 1);
        assert_eq!(page["data"][0]["clientArchived"], true);
        let source = state.threads["one"].clone();
        let fork = Action::Fork {
            thread_id: "one".into(),
        };
        for thread in [
            json!({"id":"one","cwd":root}),
            json!({"id":"foreign","cwd":"/corpstex-missing-project"}),
        ] {
            assert!(apply(
                &fork,
                &mut json!({"thread":thread}),
                &mut state,
                &mut catalog
            )
            .is_err());
        }
        apply(
            &fork,
            &mut json!({"thread":{"id":"forked","cwd":root,"turns":[]}}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert_eq!(state.threads["one"], source);
        assert!(state.threads.contains_key("forked"));
        assert!(!state.threads.contains_key("foreign"));
    }

    #[test]
    fn review_tracks_official_turn_and_late_acceptance_does_not_reactivate_it() {
        let mut state = state();
        let mut catalog = Catalog::new();
        let action = Action::Review {
            thread_id: "one".into(),
            target: ReviewTarget::UncommittedChanges,
        };
        let mut response =
            json!({"reviewThreadId":"one","turn":{"id":"review","status":"inProgress","items":[]}});
        let mut foreign = response.clone();
        foreign["reviewThreadId"] = json!("two");
        assert!(apply(&action, &mut foreign, &mut state, &mut catalog).is_err());
        apply(&action, &mut response, &mut state, &mut catalog).unwrap();
        assert!(state.active.contains(&("one".into(), "review".into())));
        assert_eq!(params(&action, &state, &catalog), Err(Error::Busy));
        event(
            &mut state,
            json!({"method":"turn/completed","params":{"threadId":"one","turn":{"id":"review","status":"completed","items":[{"id":"result","type":"exitedReviewMode","review":"No findings"}]}}}),
        );
        apply(&action, &mut response, &mut state, &mut catalog).unwrap();
        assert!(state.active.is_empty());
        assert_eq!(state.threads["one"]["turns"][0]["status"], "completed");
        let before = state.threads.clone();
        apply(
            &Action::Compact {
                thread_id: "one".into(),
            },
            &mut json!({}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert_eq!(state.threads, before);
        assert!(state.threads["two"]["turns"].as_array().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn thread_mutations_serialize_and_unsupported_methods_fail_without_retry() {
        let mut process=Process::spawn_at(std::path::Path::new("/bin/sh"), &["-c", "read -r compact; read -r rename; printf '%s\\n' '{\"id\":3,\"error\":{\"code\":-32601,\"message\":\"unavailable\"}}' '{\"id\":2,\"result\":{}}'; read -r ignored"], None).unwrap();
        let state = Arc::new(Mutex::new(state()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let mut replies = Vec::new();
        for action in [
            Action::Compact {
                thread_id: "one".into(),
            },
            Action::Rename {
                thread_id: "one".into(),
                name: "conflict".into(),
            },
            Action::Archive {
                thread_id: "two".into(),
            },
            Action::Rename {
                thread_id: "two".into(),
                name: "independent".into(),
            },
        ] {
            let (reply, result) = mpsc::sync_channel(1);
            sender
                .send(super::super::Command::Conversation(Request {
                    action,
                    reply,
                }))
                .unwrap();
            replies.push(result);
        }
        let task_state = state.clone();
        let task_stop = stop.clone();
        let task =
            std::thread::spawn(move || serve(&mut process, &task_state, &task_stop, receiver));
        let results: Vec<_> = replies
            .into_iter()
            .map(|reply| reply.recv_timeout(Duration::from_secs(3)).unwrap())
            .collect();
        stop.store(true, Ordering::Relaxed);
        task.join().unwrap().unwrap();
        assert_eq!(
            results,
            vec![
                Ok(json!({})),
                Err(Error::Busy),
                Err(Error::Busy),
                Err(Error::Remote { code: -32601 })
            ]
        );
        let state = state.lock().unwrap();
        assert!(state.active.is_empty());
        assert!(state.threads["two"]["name"].is_null());
    }

    #[test]
    fn images_are_protocol_inputs_for_start_and_steer_and_stay_in_transcript() {
        let mut state = state();
        let mut catalog = Catalog::new();
        let image = "data:image/png;base64,iVBORw0KGgo=";
        let mut action: Action = serde_json::from_value(
            json!({"type":"send","threadId":"one","text":"","images":[image]}),
        )
        .unwrap();
        let (method, wire) = params(&action, &state, &catalog).unwrap();
        assert_eq!(method, "turn/start");
        assert_eq!(wire["input"][1], json!({"type":"image","url":image}));
        assert!(wire.get("approvalPolicy").is_none());
        apply(
            &action,
            &mut json!({"turn":{"id":"images","status":"inProgress","items":[]}}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert_eq!(
            state.threads["one"]["turns"][0]["items"][0]["content"][1]["url"],
            image
        );
        action = serde_json::from_value(json!({"type":"steer","threadId":"one","expectedTurnId":"images","text":"","images":[image]})).unwrap();
        assert_eq!(params(&action, &state, &catalog).unwrap().0, "turn/steer");
        for images in [
            json!(["https://example.com/tracker.png"]),
            json!(["data:image/svg+xml;base64,PHN2Zz4="]),
            json!(vec![image; 9]),
            json!([format!(
                "data:image/png;base64,{}",
                "A".repeat(crate::media::MAX_DATA)
            )]),
        ] {
            let bad: Action = serde_json::from_value(
                json!({"type":"send","threadId":"two","text":"image","images":images}),
            )
            .unwrap();
            assert!(params(&bad, &state, &catalog).is_err());
        }
        let bad: Action = serde_json::from_value(json!({"type":"steer","threadId":"two","expectedTurnId":"images","text":"","images":[image]})).unwrap();
        assert!(params(&bad, &state, &catalog).is_err());
    }

    #[test]
    fn usage_is_thread_scoped_and_account_costs_are_not_invented() {
        let mut state = state();
        let mut catalog = Catalog::new();
        event(
            &mut state,
            json!({"method":"thread/tokenUsage/updated","params":{"threadId":"one","turnId":"turn","tokenUsage":{"total":{"totalTokens":420},"last":{"totalTokens":200},"modelContextWindow":1000}}}),
        );
        assert_eq!(
            state.threads["one"]["tokenUsage"]["total"]["totalTokens"],
            420
        );
        assert!(state.threads["two"].get("tokenUsage").is_none());
        assert!(params(
            &Action::ReadUsage {
                thread_id: "foreign".into()
            },
            &state,
            &catalog
        )
        .is_err());
        let action = Action::ReadUsage {
            thread_id: "one".into(),
        };
        assert_eq!(
            params(&action, &state, &catalog).unwrap().0,
            "account/usage/read"
        );
        assert!(apply(
            &action,
            &mut json!({"threadUsage":{"threadId":"two","nativeUsageUsdMicros":200}}),
            &mut state,
            &mut catalog
        )
        .is_err());
        let mut result = json!({"summary":{"secret":"DO_NOT_EXPOSE"},"threadUsage":{"threadId":"one","estimatedUsageUsdMicros":5000,"nativeUsageUsdMicros":null}});
        apply(&action, &mut result, &mut state, &mut catalog).unwrap();
        assert!(result["threadUsage"]["nativeUsageUsdMicros"].is_null());
        assert_eq!(result["threadUsage"]["estimatedUsageUsdMicros"], 5000);
        assert!(!result.to_string().contains("DO_NOT_EXPOSE"));
    }

    #[test]
    fn settings_filter_secrets_and_constrain_versioned_writes() {
        let mut state = state();
        let mut catalog = Catalog::new();
        let (method, read) = params(&Action::ReadSettings, &state, &catalog).unwrap();
        assert_eq!(method, "config/read");
        assert_eq!(read["cwd"], state.snapshot.root);
        let mut result = json!({"config":{"model":"fixture","approval_policy":{"granular":{"rules":true}},"sandbox_mode":"read-only","provider_token":"DO_NOT_EXPOSE"},"origins":{"model":{"name":{"type":"project"}}},"layers":[{"name":{"type":"user","file":"/fixture/config.toml"},"version":"v1","config":{"secret":"DO_NOT_EXPOSE"}}]});
        apply(&Action::ReadSettings, &mut result, &mut state, &mut catalog).unwrap();
        assert!(!result.to_string().contains("DO_NOT_EXPOSE"));
        assert!(result.get("layers").is_none());
        assert_eq!(result["version"], "v1");
        assert_eq!(result["origins"]["model"], "project");
        let write = |version: &str, key: &str, value: &str| Action::WriteSettings {
            expected_version: version.into(),
            changes: vec![SettingChange {
                key: key.into(),
                value: value.into(),
            }],
        };
        for (version, key, value) in [
            ("old", "sandbox_mode", "workspace-write"),
            ("v1", "arbitrary.path", "x"),
            ("v1", "sandbox_mode", "invalid"),
        ] {
            assert!(params(&write(version, key, value), &state, &catalog).is_err());
        }
        let action = write("v1", "sandbox_mode", "workspace-write");
        let (method, payload) = params(&action, &state, &catalog).unwrap();
        assert_eq!(method, "config/batchWrite");
        assert_eq!(payload["expectedVersion"], "v1");
        assert_eq!(payload["reloadUserConfig"], false);
        assert!(payload.get("filePath").is_none());
        assert_eq!(payload["edits"][0]["mergeStrategy"], "replace");
        apply(
            &action,
            &mut json!({"status":"okOverridden","version":"v2","filePath":"/fixture"}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert!(params(&action, &state, &catalog).is_err());
        let mut constraints =
            json!({"requirements":{"allowedSandboxModes":["readOnly"],"secret":"DO_NOT_EXPOSE"}});
        apply(
            &Action::ReadRequirements,
            &mut constraints,
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert!(!constraints.to_string().contains("DO_NOT_EXPOSE"));
        assert_eq!(
            constraints["requirements"]["allowedSandboxModes"][0],
            "readOnly"
        );
        assert!(serde_json::from_value::<Action>(json!({"type":"writeSettings","expectedVersion":"v1","changes":[{"key":"sandbox_mode","value":"read-only","filePath":"/escape"}]})).is_err());
    }

    #[test]
    fn reasoning_fragments_preserve_sections_and_thread_isolation() {
        let mut state = state();
        for (method, position, part, delta) in [
            (
                "item/reasoning/summaryTextDelta",
                "summaryIndex",
                0,
                "First ",
            ),
            (
                "item/reasoning/summaryTextDelta",
                "summaryIndex",
                0,
                "section",
            ),
            (
                "item/reasoning/summaryTextDelta",
                "summaryIndex",
                1,
                "Second section",
            ),
            (
                "item/reasoning/textDelta",
                "contentIndex",
                0,
                "Reported content",
            ),
        ] {
            let mut params =
                json!({"threadId":"one","turnId":"turn","itemId":"reason","delta":delta});
            params[position] = json!(part);
            event(&mut state, json!({"method":method,"params":params}));
        }
        let item = &state.threads["one"]["turns"][0]["items"][0];
        assert_eq!(item["type"], "reasoning");
        assert_eq!(item["summaryDelta"]["0"], "First section");
        assert_eq!(item["summaryDelta"]["1"], "Second section");
        assert_eq!(item["contentDelta"]["0"], "Reported content");
        assert_eq!(state.threads["two"]["turns"], json!([]));
        event(
            &mut state,
            json!({"method":"item/completed","params":{"threadId":"one","turnId":"turn","item":{"id":"reason","type":"reasoning","summary":["Final summary"],"content":[]}}}),
        );
        let item = &state.threads["one"]["turns"][0]["items"][0];
        assert_eq!(item["summary"], json!(["Final summary"]));
        assert!(item.get("summaryDelta").is_none());
    }

    #[test]
    fn messages_stream_complete_fail_and_interrupt_without_cross_thread_leakage() {
        let mut state = state();
        for (thread, status) in [("one", "completed"), ("two", "failed")] {
            event(
                &mut state,
                json!({"method":"turn/started","params":{"threadId":thread,"turn":{"id":"same-turn","status":"inProgress","items":[]}}}),
            );
            for delta in ["hello ", thread] {
                event(
                    &mut state,
                    json!({"method":"item/agentMessage/delta","params":{"threadId":thread,"turnId":"same-turn","itemId":"same-item","delta":delta}}),
                );
            }
            event(
                &mut state,
                json!({"method":"item/completed","params":{"threadId":thread,"turnId":"same-turn","item":{"id":"same-item","type":"agentMessage","text":format!("final {thread}")}}}),
            );
            event(
                &mut state,
                json!({"method":"turn/completed","params":{"threadId":thread,"turn":{"id":"same-turn","status":status,"items":[],"error":if status=="failed" {json!({"message":"fixture failure"})} else {Value::Null}}}}),
            );
            assert_eq!(state.threads[thread]["turns"][0]["status"], status);
            assert_eq!(
                state.threads[thread]["turns"][0]["items"][0]["text"],
                format!("final {thread}")
            );
        }
        event(
            &mut state,
            json!({"method":"turn/started","params":{"threadId":"one","turn":{"id":"stop","status":"inProgress","items":[]}}}),
        );
        assert!(params(
            &Action::Interrupt {
                thread_id: "one".into(),
                turn_id: "stop".into()
            },
            &state,
            &Catalog::new()
        )
        .is_ok());
        event(
            &mut state,
            json!({"method":"turn/completed","params":{"threadId":"one","turn":{"id":"stop","status":"interrupted","items":[]}}}),
        );
        assert!(state.active.is_empty());
        assert_eq!(state.threads["one"]["turns"][1]["status"], "interrupted");
    }

    #[test]
    fn resume_history_preserves_live_turns_and_deduplicates_old_pages() {
        let mut state = state();
        let mut catalog = Catalog::new();
        event(
            &mut state,
            json!({"method":"turn/started","params":{"threadId":"one","turn":{"id":"live","status":"inProgress","items":[]}}}),
        );
        let root = state.snapshot.root.clone();
        apply(
            &Action::Resume {
                thread_id: "one".into(),
            },
            &mut json!({"thread":{"id":"one","cwd":root,"turns":[]}}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        let page = json!({"data":[{"id":"newer","status":"completed","items":[]},{"id":"older","status":"failed","items":[]}]});
        for _ in 0..2 {
            apply(
                &Action::History {
                    thread_id: "one".into(),
                    cursor: None,
                },
                &mut page.clone(),
                &mut state,
                &mut catalog,
            )
            .unwrap();
        }
        let turns = state.threads["one"]["turns"].as_array().unwrap();
        assert_eq!(
            turns
                .iter()
                .map(|turn| turn["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["older", "newer", "live"]
        );
        assert_eq!(turns[2]["status"], "inProgress");
    }

    #[test]
    fn model_catalog_refresh_removes_stale_models_and_validates_effort() {
        let mut state = state();
        let mut catalog = Catalog::new();
        apply(&Action::ListModels{cursor:None,thread_id:None},&mut json!({"data":[{"model":"fixture","supportedReasoningEfforts":[{"reasoningEffort":"high"}]}]}),&mut state,&mut catalog).unwrap();
        let action = Action::Send {
            thread_id: "one".into(),
            text: "hello".into(),
            skills: vec![],
            images: vec![],
            model: Some("fixture".into()),
            effort: Some("high".into()),
        };
        let (_, params) = params(&action, &state, &catalog).unwrap();
        assert_eq!(params["model"], "fixture");
        assert_eq!(params["effort"], "high");
        assert!(params.get("approvalPolicy").is_none());
        assert!(params.get("sandboxPolicy").is_none());
        apply(
            &Action::ListModels {
                cursor: None,
                thread_id: None,
            },
            &mut json!({"data":[]}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert!(super::params(&action, &state, &catalog).is_err());
        let (_, defaults) = super::params(
            &Action::Send {
                thread_id: "one".into(),
                text: "hello".into(),
                skills: vec![],
                images: vec![],
                model: None,
                effort: None,
            },
            &state,
            &catalog,
        )
        .unwrap();
        assert!(defaults.get("model").is_none());
        assert!(defaults.get("effort").is_none());
    }

    #[test]
    fn steering_requires_exact_active_turn_and_preserves_transcript() {
        let mut state = state();
        state.active.insert(("one".into(), "active".into()));
        let catalog = Catalog::new();
        let action = Action::Steer {
            thread_id: "one".into(),
            expected_turn_id: "active".into(),
            text: "Focus on tests".into(),
            skills: vec![],
            images: vec![],
        };
        let (method, wire) = params(&action, &state, &catalog).unwrap();
        assert_eq!(method, "turn/steer");
        assert_eq!(
            wire,
            json!({"threadId":"one","expectedTurnId":"active","input":[{"type":"text","text":"Focus on tests"}]})
        );
        for (thread, turn, text) in [
            ("one", "stale", "text"),
            ("two", "active", "text"),
            ("foreign", "active", "text"),
            ("one", "active", " "),
            ("one", "active", &"x".repeat(65537)),
        ] {
            assert!(params(
                &Action::Steer {
                    thread_id: thread.into(),
                    expected_turn_id: turn.into(),
                    text: text.into(),
                    skills: vec![],
                    images: vec![],
                },
                &state,
                &catalog
            )
            .is_err());
        }
        let before = state.threads.clone();
        let mut catalog = catalog;
        apply(
            &action,
            &mut json!({"turnId":"active"}),
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert_eq!(state.threads, before);
        assert!(apply(
            &action,
            &mut json!({"turnId":"other"}),
            &mut state,
            &mut catalog
        )
        .is_err());
        state.active.clear();
        assert!(params(&action, &state, &catalog).is_err());
        assert!(serde_json::from_value::<Action>(json!({"type":"steer","threadId":"one","expectedTurnId":"active","text":"text","approvalPolicy":"never"})).is_err());
    }

    #[test]
    fn listing_and_requests_cannot_bind_another_projects_thread() {
        let mut state = state();
        let root = state.snapshot.root.clone();
        let mut catalog = Catalog::new();
        let mut result = json!({"data":[{"id":"valid","cwd":root,"turns":[]},{"id":"foreign","cwd":"/corpstex-missing-project","turns":[]}]});
        apply(
            &Action::ListThreads { cursor: None },
            &mut result,
            &mut state,
            &mut catalog,
        )
        .unwrap();
        assert_eq!(result["data"].as_array().unwrap().len(), 1);
        assert!(!state.threads.contains_key("foreign"));
        assert!(params(
            &Action::Resume {
                thread_id: "foreign".into()
            },
            &state,
            &catalog
        )
        .is_err());
        assert!(serde_json::from_value::<Action>(
            json!({"type":"send","threadId":"one","text":"hello","approvalPolicy":"never"})
        )
        .is_err());
    }

    #[test]
    fn assistant_only_completion_keeps_the_accepted_user_message() {
        let mut state = state();
        apply(
            &Action::Send {
                thread_id: "one".into(),
                text: "my message".into(),
                skills: vec![],
                images: vec![],
                model: None,
                effort: None,
            },
            &mut json!({"turn":{"id":"t","status":"inProgress","items":[]}}),
            &mut state,
            &mut Catalog::new(),
        )
        .unwrap();
        event(
            &mut state,
            json!({"method":"turn/completed","params":{"threadId":"one","turn":{"id":"t","status":"completed","items":[{"id":"a","type":"agentMessage","text":"reply"}]}}}),
        );
        assert_eq!(
            state.threads["one"]["turns"][0]["items"][0]["content"][0]["text"],
            "my message"
        );
        assert_eq!(
            state.threads["one"]["turns"][0]["items"][1]["text"],
            "reply"
        );
    }

    #[test]
    fn late_start_response_cannot_reactivate_completed_turn() {
        let mut state = state();
        event(
            &mut state,
            json!({"method":"turn/completed","params":{"threadId":"one","turn":{"id":"late","status":"completed","items":[]}}}),
        );
        apply(
            &Action::Send {
                thread_id: "one".into(),
                text: "hi".into(),
                skills: vec![],
                images: vec![],
                model: None,
                effort: None,
            },
            &mut json!({"turn":{"id":"late","status":"inProgress","items":[]}}),
            &mut state,
            &mut Catalog::new(),
        )
        .unwrap();
        assert_eq!(state.threads["one"]["turns"][0]["status"], "completed");
        assert!(state.active.is_empty());
        assert_eq!(
            state.threads["one"]["turns"][0]["items"][0]["content"][0]["text"],
            "hi"
        );
        event(
            &mut state,
            json!({"method":"item/completed","params":{"threadId":"one","turnId":"late","item":{"id":"official-user","type":"userMessage","content":[{"type":"text","text":"hi"}]}}}),
        );
        assert_eq!(
            state.threads["one"]["turns"][0]["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            state.threads["one"]["turns"][0]["items"][0]["id"],
            "official-user"
        );
    }
}
