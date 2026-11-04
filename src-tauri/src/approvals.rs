//! User-driven, request-bound responses. No arbitrary JSON-RPC response escape hatch.
use super::{Error, Process, ServerState, Status};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Decision {
    Select {
        #[serde(rename = "optionId")]
        option_id: String,
    },
    Accept {},
    Decline {},
    Cancel {},
    Answers {
        answers: HashMap<String, String>,
    },
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reply {
    pub key: String,
    pub ticket: u64,
    pub decision: Decision,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub runtime: super::Runtime,
    pub root: String,
    pub generation: u64,
    pub ticket: u64,
    pub key: String,
    pub request: Value,
    pub item: Option<Value>,
    pub status: Status,
    pub decisions: Vec<&'static str>,
}

pub fn choices(request: &Value) -> Vec<&'static str> {
    let p = &request["params"];
    let method = request["method"].as_str().unwrap_or("");
    if method == "session/request_permission" {
        let Some(options) = p["options"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 32)
        else {
            return vec![];
        };
        let mut ids = std::collections::HashSet::new();
        if !p["sessionId"].is_string()
            || !p["toolCall"]["toolCallId"].is_string()
            || options.iter().any(|o| {
                o["optionId"]
                    .as_str()
                    .is_none_or(|id| id.is_empty() || id.len() > 256 || !ids.insert(id))
                    || !o["name"].is_string()
                    || !matches!(
                        o["kind"].as_str(),
                        Some("allow_once" | "allow_always" | "reject_once" | "reject_always")
                    )
            })
        {
            return vec![];
        }
        return vec!["options", "cancel"];
    }
    let legacy = matches!(method, "execCommandApproval" | "applyPatchApproval");
    if !(if legacy {
        p["conversationId"].is_string() && p["callId"].is_string()
    } else {
        p["threadId"].is_string()
    }) {
        return vec![];
    }
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            if !p["turnId"].is_string() || !p["itemId"].is_string() {
                return vec![];
            }
            let mut choices = vec!["accept", "decline", "cancel"];
            if method == "item/commandExecution/requestApproval" {
                if let Some(available) = p.get("availableDecisions").filter(|v| !v.is_null()) {
                    let Some(available) = available.as_array() else {
                        return vec![];
                    };
                    choices.retain(|choice| available.iter().any(|v| v.as_str() == Some(choice)));
                }
            }
            choices
        }
        "execCommandApproval" | "applyPatchApproval" => vec!["accept", "decline", "cancel"],
        "item/permissions/requestApproval" => {
            if !p["turnId"].is_string() || !p["itemId"].is_string() {
                return vec![];
            }
            // Echo exactly the requested grant, never a frontend-supplied permission profile.
            if p["permissions"].as_object().is_some_and(|p| {
                p.keys()
                    .all(|k| matches!(k.as_str(), "network" | "fileSystem"))
            }) {
                vec!["accept", "decline"]
            } else {
                vec!["decline"]
            }
        }
        "item/tool/requestUserInput" => {
            if p["questions"].as_array().is_some_and(|qs| {
                !qs.is_empty()
                    && qs
                        .iter()
                        .all(|q| q["id"].is_string() && q["question"].is_string())
            }) {
                vec!["answers"]
            } else {
                vec![]
            }
        }
        // Arbitrary MCP form schemas and URL authentication are not implemented yet.
        "mcpServer/elicitation/request" => vec!["decline", "cancel"],
        _ => vec![],
    }
}

fn result(request: &Value, decision: &Decision) -> Result<Value, Error> {
    let name = match decision {
        Decision::Select { .. } => "options",
        Decision::Accept {} => "accept",
        Decision::Decline {} => "decline",
        Decision::Cancel {} => "cancel",
        Decision::Answers { .. } => "answers",
    };
    if !choices(request).contains(&name) {
        return Err(Error::InvalidRequest);
    }
    let p = &request["params"];
    Ok(match request["method"].as_str().unwrap_or("") {
        "session/request_permission" => match decision {
            Decision::Cancel {} => json!({"outcome":{"outcome":"cancelled"}}),
            Decision::Select { option_id }
                if p["options"]
                    .as_array()
                    .is_some_and(|options| options.iter().any(|o| o["optionId"] == *option_id)) =>
            {
                json!({"outcome":{"outcome":"selected","optionId":option_id}})
            }
            _ => return Err(Error::InvalidRequest),
        },
        "execCommandApproval" | "applyPatchApproval" => json!({"decision":match decision {
            Decision::Accept {} => json!("approved"),
            Decision::Decline {} => json!({"denied":{"rejection":"Declined by the user in CorpsTex."}}),
            _ => json!("abort"),
        }}),
        "item/permissions/requestApproval" => {
            json!({"permissions":if name == "accept" { p["permissions"].clone() } else { json!({}) },"scope":"turn"})
        }
        "mcpServer/elicitation/request" => json!({"action":name}),
        "item/tool/requestUserInput" => {
            let Decision::Answers { answers } = decision else {
                return Err(Error::InvalidRequest);
            };
            let qs = p["questions"].as_array().ok_or(Error::InvalidRequest)?;
            if answers.len() != qs.len() {
                return Err(Error::InvalidRequest);
            }
            let mut values = serde_json::Map::new();
            for q in qs {
                let id = q["id"].as_str().ok_or(Error::InvalidRequest)?;
                let answer = answers
                    .get(id)
                    .filter(|s| !s.trim().is_empty() && s.len() <= 16 * 1024)
                    .ok_or(Error::InvalidRequest)?;
                if q["isOther"] != true {
                    if let Some(options) = q["options"].as_array().filter(|v| !v.is_empty()) {
                        if !options.iter().any(|o| o["label"].as_str() == Some(answer)) {
                            return Err(Error::InvalidRequest);
                        }
                    }
                }
                values.insert(id.to_owned(), json!({"answers":[answer]}));
            }
            json!({"answers":values})
        }
        _ => json!({"decision":name}),
    })
}

pub fn queue(state: &ServerState) -> Vec<Approval> {
    state
        .pending
        .iter()
        .map(|(key, request)| {
            let p = &request["params"];
            let item = state
                .threads
                .get(p["threadId"].as_str().unwrap_or(""))
                .and_then(|t| t["turns"].as_array())
                .and_then(|ts| ts.iter().find(|t| t["id"] == p["turnId"]))
                .and_then(|t| t["items"].as_array())
                .and_then(|items| items.iter().find(|i| i["id"] == p["itemId"]))
                .cloned();
            Approval {
                runtime: state.snapshot.runtime,
                root: state.snapshot.root.clone(),
                generation: state.snapshot.generation,
                ticket: state.tickets[key],
                key: key.clone(),
                request: request.clone(),
                item,
                status: state.snapshot.status.clone(),
                decisions: if state.snapshot.runtime == super::Runtime::Opencode
                    && !state
                        .threads
                        .contains_key(p["sessionId"].as_str().unwrap_or(""))
                {
                    vec![]
                } else {
                    choices(request)
                },
            }
        })
        .collect()
}

pub fn respond(
    process: &mut Process,
    state: &mut ServerState,
    reply: &Reply,
) -> Result<Value, Error> {
    let key = &reply.key;
    if state.approvals_frozen {
        return Err(Error::ExitPending);
    }
    if state.snapshot.status != Status::Ready {
        return Err(Error::ServerExit);
    }
    if state.tickets.get(key) != Some(&reply.ticket) {
        return Err(Error::InvalidRequest);
    }
    let request = state.pending.get(key).ok_or(Error::InvalidRequest)?;
    if state.snapshot.runtime == super::Runtime::Opencode
        && !state
            .threads
            .contains_key(request["params"]["sessionId"].as_str().unwrap_or(""))
    {
        return Err(Error::InvalidRequest);
    }
    let result = result(request, &reply.decision)?;
    // A failed/partial write is not retried: stop the connection; retain the request visibly.
    let mut response = json!({"id":request["id"],"result":result});
    if state.snapshot.runtime == super::Runtime::Opencode {
        response["jsonrpc"] = json!("2.0");
    }
    process.send(&response)?;
    state.pending.remove(key);
    state.tickets.remove(key);
    state.snapshot.pending_requests = state.pending.len();
    Ok(json!({"sent":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn resolved_reused_and_frozen_requests_reject_stale_decisions() {
        let mut state = ServerState::new(std::path::Path::new("/fixture"), 1);
        state.snapshot.status = Status::Ready;
        let mut process = Process::spawn_at(std::path::Path::new("/bin/cat"), &[], None).unwrap();
        let request = json!({"id":1,"method":"item/fileChange/requestApproval","params":{"threadId":"t","turnId":"v","itemId":"i"}});
        state.record(&request.to_string()).unwrap();
        let key = "1".to_owned();
        let old = Reply {
            key: key.clone(),
            ticket: state.tickets[&key],
            decision: Decision::Accept {},
        };
        // An unrelated thread cannot resolve this approval.
        state.record(&json!({"method":"serverRequest/resolved","params":{"threadId":"other","requestId":1}}).to_string()).unwrap();
        assert!(state.pending.contains_key(&key));
        state
            .record(
                &json!({"method":"serverRequest/resolved","params":{"threadId":"t","requestId":1}})
                    .to_string(),
            )
            .unwrap();
        assert_eq!(
            respond(&mut process, &mut state, &old),
            Err(Error::InvalidRequest)
        );
        state.record(&request.to_string()).unwrap();
        assert_eq!(
            respond(&mut process, &mut state, &old),
            Err(Error::InvalidRequest)
        );
        let new = Reply {
            ticket: state.tickets[&key],
            ..old
        };
        state.approvals_frozen = true;
        assert_eq!(
            respond(&mut process, &mut state, &new),
            Err(Error::ExitPending)
        );
        state.approvals_frozen = false;
        assert!(process.poll().unwrap().is_none()); // Nothing was silently sent.
        respond(&mut process, &mut state, &new).unwrap();
        let response: Value = serde_json::from_str(&process.poll().unwrap().unwrap()).unwrap();
        assert_eq!(response, json!({"id":1,"result":{"decision":"accept"}}));
        assert!(state.pending.is_empty());
        assert_eq!(
            respond(&mut process, &mut state, &new),
            Err(Error::InvalidRequest)
        );
    }

    #[test]
    fn protocol_decisions_and_server_choices_are_enforced() {
        for method in [
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
        ] {
            let r = json!({"method":method,"params":{"threadId":"t","turnId":"v","itemId":"i"}});
            assert_eq!(
                result(&r, &Decision::Decline {}).unwrap(),
                json!({"decision":"decline"})
            );
            assert_eq!(
                result(&r, &Decision::Cancel {}).unwrap(),
                json!({"decision":"cancel"})
            );
        }
        let r = json!({"method":"item/commandExecution/requestApproval","params":{"threadId":"t","turnId":"v","itemId":"i","availableDecisions":["decline"]}});
        assert_eq!(result(&r, &Decision::Accept {}), Err(Error::InvalidRequest));
        let r =
            json!({"method":"execCommandApproval","params":{"conversationId":"t","callId":"i"}});
        assert_eq!(
            result(&r, &Decision::Decline {}).unwrap()["decision"]["denied"]["rejection"],
            "Declined by the user in CorpsTex."
        );
        assert_eq!(
            result(&r, &Decision::Cancel {}).unwrap(),
            json!({"decision":"abort"})
        );
        let mut r = json!({"method":"item/permissions/requestApproval","params":{"threadId":"t","turnId":"v","itemId":"i","permissions":{"network":{"enabled":true},"fileSystem":{"read":["/fixture"]}}}});
        assert_eq!(
            result(&r, &Decision::Accept {}).unwrap(),
            json!({"permissions":r["params"]["permissions"],"scope":"turn"})
        );
        assert_eq!(
            result(&r, &Decision::Decline {}).unwrap(),
            json!({"permissions":{},"scope":"turn"})
        );
        r["params"]["permissions"]["futurePermission"] = json!(true);
        assert_eq!(result(&r, &Decision::Accept {}), Err(Error::InvalidRequest));
        assert!(choices(&json!({"method":"unknown","params":{"threadId":"t"}})).is_empty());
        assert!(
            serde_json::from_value::<Decision>(json!({"type":"accept","permissions":{}})).is_err()
        );
    }
    #[test]
    fn input_answers_and_mcp_denial_use_distinct_protocols() {
        let r = json!({"method":"item/tool/requestUserInput","params":{"threadId":"t","questions":[{"id":"q","question":"Proceed?","options":[{"label":"Decline"}]}]}});
        let answer = |a: &str| Decision::Answers {
            answers: HashMap::from([("q".into(), a.into())]),
        };
        assert_eq!(
            result(&r, &answer("Decline")).unwrap(),
            json!({"answers":{"q":{"answers":["Decline"]}}})
        );
        assert_eq!(result(&r, &answer("Accept")), Err(Error::InvalidRequest));
        let r = json!({"method":"mcpServer/elicitation/request","params":{"threadId":"t","mode":"url"}});
        assert_eq!(
            result(&r, &Decision::Decline {}).unwrap(),
            json!({"action":"decline"})
        );
        assert_eq!(result(&r, &Decision::Accept {}), Err(Error::InvalidRequest));
    }
}
