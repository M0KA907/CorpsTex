use super::*;
use crate::projects::Registry;
#[cfg(unix)]
use crate::projects::Runtime;
use std::{path::PathBuf, sync::atomic::AtomicU64};
static NEXT: AtomicU64 = AtomicU64::new(1);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "corpstex-oc01-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
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
fn wait(
    registry: &Registry,
    predicate: impl Fn(&super::super::Snapshot) -> bool,
) -> super::super::Snapshot {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let s = registry.view("main").unwrap().projects.remove(0);
        if predicate(&s) {
            return s;
        }
        assert!(
            Instant::now() < deadline,
            "runtime state did not arrive: {s:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn call(registry: &Registry, s: &super::super::Snapshot, action: Action) -> Result<Value, Error> {
    registry.request("main", &s.root, s.generation, action)
}
#[cfg(unix)]
fn send(id: &str, text: &str) -> Action {
    Action::Send {
        thread_id: id.into(),
        text: text.into(),
        images: vec![],
        skills: vec![],
        model: Some("fixture/free:free".into()),
        effort: Some("high".into()),
    }
}
#[cfg(unix)]
const FIXTURE: &str = r#"
import json,sys,os
root=os.getcwd();pending=None;model='fixture/free:free';effort='default';count=0

def emit(v): print(json.dumps(dict(jsonrpc='2.0',**v)),flush=True)
def result(i,v): emit(dict(id=i,result=v))
def configs():return [dict(id='model',type='select',currentValue=model,options=[dict(value='fixture/free:free',name='fixture/Free'),dict(value='fixture/paid',name='fixture/Paid')]),dict(id='effort',type='select',currentValue=effort,options=[dict(value=x,name=x) for x in ['default','low','high']]),dict(id='mode',type='select',currentValue='build',options=[dict(value='build',name='Build'),dict(value='plan',name='Plan')])]
def update(s,u):emit(dict(method='session/update',params=dict(sessionId=s,update=u)))
for line in sys.stdin:
 v=json.loads(line);assert v.get('jsonrpc')=='2.0'
 i=v.get('id');m=v.get('method');p=v.get('params',{})
 if m=='initialize': result(i,dict(protocolVersion=1,agentInfo=dict(name='OpenCode',version='fixture'),agentCapabilities=dict(loadSession=True,promptCapabilities=dict(image=True),sessionCapabilities=dict(list={}))))
 elif m=='session/list': result(i,dict(sessions=[dict(sessionId='saved',cwd=root,title='Saved <session>'),dict(sessionId='foreign',cwd='/',title='Foreign')]))
 elif m=='session/new':
  count+=1;result(i,dict(sessionId='new-'+str(count),configOptions=configs()))
 elif m=='session/load':
  assert p['cwd']==root and p['sessionId']=='saved'
  for n in range(2):
   update('saved',dict(sessionUpdate='user_message_chunk',messageId='u'+str(n),content=dict(type='text',text='Question '+str(n))))
   update('saved',dict(sessionUpdate='agent_message_chunk',messageId='a'+str(n),content=dict(type='text',text='Saved reply '+str(n))))
  result(i,dict(configOptions=configs()))
 elif m=='session/set_config_option':
  if p['value']=='fail':emit(dict(id=i,error=dict(code=-32602,message='fixture error')));continue
  if p['configId']=='model':model=p['value']
  if p['configId']=='effort':effort=p['value']
  result(i,dict(configOptions=configs()))
 elif m=='session/prompt':
  pending=(i,p['sessionId']);s=p['sessionId']
  update('foreign',dict(sessionUpdate='agent_message_chunk',content=dict(type='text',text='foreign content')))
  update(s,dict(sessionUpdate='future_update',detail='<script>inert</script>'))
  update(s,dict(sessionUpdate='agent_thought_chunk',messageId='thought',content=dict(type='text',text='Thinking')))
  update(s,dict(sessionUpdate='agent_message_chunk',messageId='reply',content=dict(type='text',text='Hello ')))
  update(s,dict(sessionUpdate='agent_message_chunk',messageId='reply',content=dict(type='text',text='world')))
  update(s,dict(sessionUpdate='tool_call',toolCallId='tool',title='Read fixture',kind='read',status='pending',rawInput=dict(path='<fixture>')))
  emit(dict(id='permission',method='session/request_permission',params=dict(sessionId=s,toolCall=dict(toolCallId='tool',title='Read fixture'),options=[dict(optionId='once',kind='allow_once',name='Allow once'),dict(optionId='always',kind='allow_always',name='Always allow'),dict(optionId='reject',kind='reject_once',name='Reject')])))
 elif m=='session/cancel':
  assert pending
  result(pending[0],dict(stopReason='cancelled'));pending=None
 elif m is None and i=='permission':
  assert pending
  assert v['result']['outcome']==dict(outcome='selected',optionId='once')
  update(pending[1],dict(sessionUpdate='tool_call_update',toolCallId='tool',status='completed',rawOutput='fixture output'))
  result(pending[0],dict(stopReason='end_turn'));pending=None
 else:emit(dict(id=i,error=dict(code=-32601,message='unsupported')))
"#;
#[cfg(unix)]
fn fixture() -> Registry {
    let mut registry = Registry::opencode();
    registry.spawn = Arc::new(|root, _| {
        Ok((
            Process::spawn_at(
                Path::new("/usr/bin/python3"),
                &["-u", "-c", FIXTURE],
                Some(root),
            )?,
            "fixture".into(),
        ))
    });
    registry
}
#[cfg(unix)]
#[test]
fn acp_lifecycle_catalog_history_streaming_permissions_and_cancel() {
    let temp = Temp::new();
    let registry = fixture();
    registry.connect("main", temp.root(), None, false).unwrap();
    let s = wait(&registry, |s| s.status == Status::Ready);
    assert_eq!(s.runtime, Runtime::Opencode);
    let alias = registry
        .connect("second", &format!("{}/.", temp.root()), None, false)
        .unwrap();
    assert_eq!(alias.pid, s.pid);
    let page = call(&registry, &s, Action::ListThreads { cursor: None }).unwrap();
    assert_eq!(page["data"].as_array().unwrap().len(), 1);
    assert_eq!(page["data"][0]["id"], "saved");
    for action in [
        Action::ListArchivedThreads { cursor: None },
        Action::ListAllThreads { cursor: None },
        Action::ListMcpStatus {
            thread_id: "saved".into(),
            cursor: None,
        },
        Action::Fork {
            thread_id: "saved".into(),
        },
        Action::Rename {
            thread_id: "saved".into(),
            name: "name".into(),
        },
        Action::Archive {
            thread_id: "saved".into(),
        },
        Action::Unarchive {
            thread_id: "saved".into(),
        },
        Action::Compact {
            thread_id: "saved".into(),
        },
        Action::Review {
            thread_id: "saved".into(),
            target: super::super::conversations::ReviewTarget::UncommittedChanges,
        },
    ] {
        assert_eq!(call(&registry, &s, action), Err(Error::OpenCodeUnsupported));
    }
    assert_eq!(
        call(
            &registry,
            &s,
            Action::Resume {
                thread_id: "foreign".into()
            }
        ),
        Err(Error::InvalidRequest)
    );
    let models = call(
        &registry,
        &s,
        Action::ListModels {
            cursor: None,
            thread_id: None,
        },
    )
    .unwrap();
    assert_eq!(models["data"].as_array().unwrap().len(), 2);
    assert_eq!(models["data"][0]["freeLabel"], true);
    assert_eq!(models["data"][1]["freeLabel"], false);
    let new = call(&registry, &s, Action::NewThread).unwrap();
    let id = new["thread"]["id"].as_str().unwrap();
    assert_eq!(
        registry.request("foreign-window", &s.root, s.generation, send(id, "hello")),
        Err(Error::NotSubscribed)
    );
    assert_eq!(
        registry.request("main", &s.root, s.generation + 1, send(id, "hello")),
        Err(Error::StaleGeneration)
    );
    let accepted = call(&registry, &s, send(id, "hello")).unwrap();
    let turn = accepted["turn"]["id"].as_str().unwrap();
    wait(&registry, |s| {
        s.active_turns == 1 && s.pending_requests == 1
    });
    assert_eq!(call(&registry, &s, send(id, "duplicate")), Err(Error::Busy));
    // A thread waiting on an approval can still be re-opened (e.g. after the
    // user switched to another thread and came back); only sending is blocked.
    let reopened = call(
        &registry,
        &s,
        Action::Resume {
            thread_id: id.into(),
        },
    )
    .unwrap();
    assert_eq!(reopened["thread"]["id"], id);
    assert_eq!(
        call(&registry, &s, send(id, "still duplicate")),
        Err(Error::Busy)
    );
    let a = registry.view("main").unwrap().approvals.remove(0);
    assert_eq!(a.runtime, Runtime::Opencode);
    assert!(a.decisions.contains(&"options"));
    let invalid = super::super::ApprovalReply {
        key: a.key.clone(),
        ticket: a.ticket,
        decision: approvals::Decision::Select {
            option_id: "forged".into(),
        },
    };
    assert_eq!(
        registry.approval_reply(&s.root, s.generation, invalid),
        Err(Error::InvalidRequest)
    );
    assert_eq!(registry.view("main").unwrap().approvals.len(), 1); // never auto-approve
    registry.request_exit().unwrap();
    let reply = super::super::ApprovalReply {
        key: a.key.clone(),
        ticket: a.ticket,
        decision: approvals::Decision::Select {
            option_id: "once".into(),
        },
    };
    assert_eq!(
        registry.approval_reply(&s.root, s.generation, reply.clone()),
        Err(Error::ExitPending)
    );
    registry.cancel_exit();
    registry
        .approval_reply(&s.root, s.generation, reply.clone())
        .unwrap();
    wait(&registry, |s| {
        s.active_turns == 0 && s.pending_requests == 0
    });
    assert_eq!(
        registry.approval_reply(&s.root, s.generation, reply),
        Err(Error::InvalidRequest)
    );
    let transcript = registry
        .conversation("main", &s.root, s.generation, id)
        .unwrap();
    assert_eq!(transcript["turns"][0]["id"], turn);
    assert!(transcript.to_string().contains("Hello world"));
    assert!(transcript.to_string().contains("fixture output"));
    assert!(transcript.to_string().contains("Thinking"));
    let next = call(&registry, &s, send(id, "cancel")).unwrap();
    wait(&registry, |s| s.pending_requests == 1);
    call(
        &registry,
        &s,
        Action::Interrupt {
            thread_id: id.into(),
            turn_id: next["turn"]["id"].as_str().unwrap().into(),
        },
    )
    .unwrap();
    wait(&registry, |s| {
        s.active_turns == 0 && s.pending_requests == 0
    });
    let transcript = registry
        .conversation("main", &s.root, s.generation, id)
        .unwrap();
    assert_eq!(transcript["turns"][1]["status"], "interrupted");
    let loaded = call(
        &registry,
        &s,
        Action::Resume {
            thread_id: "saved".into(),
        },
    )
    .unwrap();
    assert_eq!(loaded["thread"]["turns"].as_array().unwrap().len(), 2);
    assert!(loaded.to_string().contains("Saved reply 1"));
    assert_eq!(
        call(
            &registry,
            &s,
            Action::Steer {
                thread_id: id.into(),
                expected_turn_id: turn.into(),
                text: "steer".into(),
                images: vec![],
                skills: vec![]
            }
        ),
        Err(Error::OpenCodeUnsupported)
    );
    let events = registry.events("main", &s.root, s.generation, 0).unwrap();
    assert!(events.events.iter().all(|e| e.runtime == Runtime::Opencode));
    let tools: Vec<_> = events
        .events
        .iter()
        .filter(|event| event.message["params"]["update"]["sessionUpdate"] == "tool_call")
        .collect();
    assert_eq!(tools.len(), 2); // One completed prompt, then one cancelled prompt.
    assert_eq!(tools[0].message["params"]["sessionId"], id);
    assert_eq!(tools[0].client_turn_id.as_deref(), Some(turn));
    assert!(events
        .events
        .iter()
        .all(|event| event.received_at_ms.is_some()));
    assert!(events
        .events
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    assert!(events
        .events
        .iter()
        .any(|event| event.message["params"]["update"]["sessionUpdate"] == "tool_call_update"));
    assert!(events
        .events
        .iter()
        .any(|event| event.message["params"]["update"]["sessionUpdate"] == "future_update"));
    assert!(events
        .events
        .iter()
        .all(|event| event.message["params"]["sessionId"] != "foreign"));
    registry.shutdown(true).unwrap();
    assert_eq!(
        registry.view("main").unwrap().projects[0].status,
        Status::Stopped
    );
    #[cfg(target_os = "linux")]
    assert!(!Path::new(&format!("/proc/{}", s.pid.unwrap())).exists());
}
#[test]
fn grouped_models_and_exact_options_do_not_invent_pricing_or_permissions() {
    let config = json!([{"id":"model","currentValue":"p/free:free","options":[{"group":"g","options":[{"value":"p/free:free","name":"Free"},{"value":"p/unknown","name":"Unknown"}]}]}]);
    let page = model_page(&config);
    assert_eq!(page["data"][0]["freeLabel"], true);
    assert_eq!(page["data"][1]["freeLabel"], false);
    assert_eq!(
        response(&json!({"error":{"code":-32000,"message":"secret"}})),
        Err(Error::OpenCodeAuthRequired)
    );
    let request = json!({"method":"session/request_permission","params":{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"optionId":"once","kind":"allow_once","name":"Allow"}]}});
    assert_eq!(approvals::choices(&request), vec!["options", "cancel"]);
    let mut malformed = request;
    malformed["params"]["options"][0]["kind"] = json!("unknown");
    assert!(approvals::choices(&malformed).is_empty());
}
#[test]
fn unknown_sessions_and_untrusted_content_do_not_cross_project_boundaries() {
    let mut state = ServerState::new(Path::new("/fixture"), 1);
    update(&mut state,&json!({"sessionId":"foreign","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"foreign"}}})).unwrap();
    assert!(state.threads.is_empty());
    state
        .threads
        .insert("s".into(), thread_value("s", "/fixture", None));
    update(
        &mut state,
        &json!({"sessionId":"s","update":{"sessionUpdate":"future_update","unsafe":"<script>"}}),
    )
    .unwrap();
    assert!(state.threads["s"].to_string().contains("future_update"));
}
#[test]
#[ignore = "explicit installed OpenCode handshake/catalog smoke; no inference or credential operations"]
fn installed_opencode_catalog() {
    let temp = Temp::new();
    let registry = Registry::opencode();
    registry.connect("main", temp.root(), None, false).unwrap();
    let s = wait(&registry, |s| s.status != Status::Starting);
    assert_eq!(s.status, Status::Ready, "{:?}", s.error);
    let models = call(
        &registry,
        &s,
        Action::ListModels {
            cursor: None,
            thread_id: None,
        },
    )
    .unwrap();
    let models = models["data"].as_array().unwrap();
    assert!(!models.is_empty());
    println!(
        "OpenCode {}: {} models, {} free-labeled; no inference or credential operations",
        s.version.as_deref().unwrap_or("unknown"),
        models.len(),
        models.iter().filter(|m| m["freeLabel"] == true).count()
    );
    let session = call(&registry, &s, Action::NewThread).unwrap();
    assert!(session["thread"]["id"].is_string());
    registry.shutdown(true).unwrap();
}

#[test]
#[ignore = "explicit one-turn free-labeled model inference; OpenCode uses its existing configuration; no credential operations"]
fn installed_opencode_free_prompt() {
    let model = std::env::var("CORPSTEX_OPENCODE_SMOKE_MODEL")
        .expect("explicit free-labeled catalog model required");
    let temp = Temp::new();
    let registry = Registry::opencode();
    registry.connect("main", temp.root(), None, false).unwrap();
    let s = wait(&registry, |s| s.status != Status::Starting);
    assert_eq!(s.status, Status::Ready, "{:?}", s.error);
    let catalog = call(
        &registry,
        &s,
        Action::ListModels {
            cursor: None,
            thread_id: None,
        },
    )
    .unwrap();
    assert!(
        catalog["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["model"] == model && m["freeLabel"] == true),
        "requested model must be free-labeled in this live catalog"
    );
    let new = call(&registry, &s, Action::NewThread).unwrap();
    let id = new["thread"]["id"].as_str().unwrap();
    call(
        &registry,
        &s,
        Action::Send {
            thread_id: id.into(),
            text: "Reply with exactly OK. Do not call tools, access files, or modify anything."
                .into(),
            images: vec![],
            skills: vec![],
            model: Some(model),
            effort: None,
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let view = registry.view("main").unwrap();
        assert!(view.approvals.is_empty(), "smoke must never approve tools");
        assert_eq!(
            view.projects[0].status,
            Status::Ready,
            "{:?}",
            view.projects[0].error
        );
        if view.projects[0].active_turns == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "live model did not complete within 60 seconds"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let transcript = registry
        .conversation("main", &s.root, s.generation, id)
        .unwrap();
    let turn = transcript["turns"].as_array().unwrap().last().unwrap();
    assert_eq!(turn["status"], "completed", "{:?}", turn["error"]);
    assert!(
        turn["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["type"] == "agentMessage"
                && i["text"].as_str().is_some_and(|t| t.contains("OK"))),
        "expected a model reply"
    );
    println!(
        "OpenCode {}: free-labeled model returned OK; no tools or credential operations",
        s.version.as_deref().unwrap_or("unknown")
    );
    registry.shutdown(true).unwrap();
}

#[test]
fn image_inputs_require_runtime_capability_and_bounded_raster_data() {
    let mut state = ServerState::new(Path::new("/fixture"), 1);
    state
        .threads
        .insert("s".into(), thread_value("s", "/fixture", None));
    let mut adapter = Adapter::default();
    adapter.loaded.insert("s".into());
    let image="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1kAAAAASUVORK5CYII=";
    let mut action = Action::Send {
        thread_id: "s".into(),
        text: "".into(),
        images: vec![image.into()],
        skills: vec![],
        model: None,
        effort: None,
    };
    assert_eq!(
        adapter.validate_send(&state, &action),
        Err(Error::InvalidRequest)
    );
    adapter.capabilities = json!({"promptCapabilities":{"image":true}});
    assert_eq!(adapter.validate_send(&state, &action), Ok(()));
    if let Action::Send { images, .. } = &mut action {
        images[0] = "data:image/svg+xml;base64,PHN2Zz4=".into();
    }
    assert_eq!(
        adapter.validate_send(&state, &action),
        Err(Error::InvalidRequest)
    );
}

#[test]
fn catalog_metadata_does_not_create_a_fake_history_turn() {
    let mut state = ServerState::new(Path::new("/fixture"), 1);
    state
        .threads
        .insert("s".into(), thread_value("s", "/fixture", None));
    update(&mut state,&json!({"sessionId":"s","update":{"sessionUpdate":"available_commands_update","availableCommands":[]}})).unwrap();
    assert_eq!(state.threads["s"]["turns"], json!([]));
}

#[cfg(unix)]
#[test]
fn idle_reconnect_reaps_the_old_child_and_rechecks_binding() {
    let temp = Temp::new();
    let registry = fixture();
    registry.connect("main", temp.root(), None, false).unwrap();
    let s = wait(&registry, |s| s.status == Status::Ready);
    assert_eq!(
        registry
            .reconnect_idle("foreign", &s.root, s.generation)
            .unwrap_err(),
        Error::NotSubscribed
    );
    assert_eq!(
        registry
            .reconnect_idle("main", &s.root, s.generation + 1)
            .unwrap_err(),
        Error::StaleGeneration
    );
    let restarted = registry
        .reconnect_idle("main", &s.root, s.generation)
        .unwrap();
    assert!(restarted.generation > s.generation);
    let ready = wait(&registry, |s| s.status == Status::Ready);
    assert_ne!(ready.pid, s.pid);
    #[cfg(target_os = "linux")]
    assert!(!Path::new(&format!("/proc/{}", s.pid.unwrap())).exists());
    registry.shutdown(true).unwrap();
}
