#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod editor;
mod editor_install;
mod editor_lsp;
mod media;
mod projects;
mod runtime;
mod runtimes;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

#[derive(Default)]
struct ExitState {
    started: AtomicBool,
    ready: AtomicBool,
}

#[tauri::command]
fn editor_install_plan(
    app: tauri::AppHandle,
    id: String,
    remove: Option<bool>,
) -> editor::Result<editor_install::Plan> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|_| editor::error("installer", "Application data path unavailable."))?;
    if remove.unwrap_or(false) {
        editor_install::removal_plan(&base, &id)
    } else {
        editor_install::plan(&base, &id)
    }
}
#[tauri::command]
async fn editor_install(
    app: tauri::AppHandle,
    id: String,
    approved: bool,
    remove: bool,
) -> editor::Result<serde_json::Value> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|_| editor::error("installer", "Application data path unavailable."))?;
    tauri::async_runtime::spawn_blocking(move || {
        if remove {
            editor_install::remove(&base, &id, approved)?;
            Ok(serde_json::Value::Null)
        } else {
            serde_json::to_value(editor_install::install(&base, &id, approved)?)
                .map_err(|_| editor::error("installer", "Could not encode installation result."))
        }
    })
    .await
    .map_err(|_| editor::error("bridge", "Installation worker failed."))?
}

#[tauri::command]
async fn editor_request(
    app: tauri::AppHandle,
    window: tauri::Window,
    root: String,
    action: editor::Action,
) -> editor::Result<serde_json::Value> {
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<editor::Editor>().request(&label, &root, action)
    })
    .await
    .map_err(|_| editor::error("bridge", "Editor worker failed."))?
}
#[tauri::command]
fn editor_dirty(
    window: tauri::Window,
    state: tauri::State<'_, editor::Editor>,
    dirty: bool,
) -> editor::Result<()> {
    state.dirty(window.label(), dirty)
}
#[tauri::command]
fn editor_exit_pending(state: tauri::State<'_, editor::Editor>) -> bool {
    state.exit_pending.load(Ordering::SeqCst)
}
#[tauri::command]
fn editor_install_busy() -> bool {
    editor_install::busy()
}
#[tauri::command]
fn editor_resolve_exit(app: tauri::AppHandle, window: tauri::Window, discard: bool) {
    let state = app.state::<editor::Editor>();
    state.exit_pending.store(false, Ordering::SeqCst);
    if discard {
        let _ = state.dirty(window.label(), false);
        request_exit(&app);
    }
}
#[tauri::command]
fn editor_lsp_discover() -> Vec<editor_lsp::Discovery> {
    editor_lsp::discoveries()
}
#[tauri::command]
async fn editor_lsp_start(
    app: tauri::AppHandle,
    window: tauri::Window,
    root: String,
    id: String,
    config: editor_lsp::Configuration,
) -> editor::Result<serde_json::Value> {
    let label = window.label().to_owned();
    let authorized = app.state::<editor::Editor>().authorized(&label, &root)?;
    let root = std::path::PathBuf::from(editor::root_label(&authorized));
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<editor_lsp::Servers>()
            .start(&label, &root, &id, config)
    })
    .await
    .map_err(|_| editor::error("bridge", "Language server worker failed."))?
}
#[tauri::command]
async fn editor_lsp_request(
    app: tauri::AppHandle,
    window: tauri::Window,
    root: String,
    id: String,
    method: String,
    params: serde_json::Value,
) -> editor::Result<serde_json::Value> {
    let label = window.label().to_owned();
    app.state::<editor::Editor>().authorized(&label, &root)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<editor_lsp::Servers>()
            .request(&label, &root, &id, &method, params)
    })
    .await
    .map_err(|_| editor::error("bridge", "Language server worker failed."))?
}
#[tauri::command]
fn editor_lsp_notify(
    window: tauri::Window,
    state: tauri::State<'_, editor_lsp::Servers>,
    root: String,
    id: String,
    method: String,
    params: serde_json::Value,
) -> editor::Result<()> {
    state.notify(window.label(), &root, &id, &method, params)
}
#[tauri::command]
fn editor_lsp_poll(
    window: tauri::Window,
    state: tauri::State<'_, editor_lsp::Servers>,
    root: String,
    id: String,
) -> editor::Result<serde_json::Value> {
    state.poll(window.label(), &root, &id)
}
#[tauri::command]
async fn editor_lsp_stop(
    app: tauri::AppHandle,
    window: tauri::Window,
    root: String,
    id: String,
) -> editor::Result<()> {
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<editor_lsp::Servers>().stop(&label, &root, &id)
    })
    .await
    .map_err(|_| editor::error("bridge", "Language server cleanup failed."))
}

#[tauri::command]
fn connect_project(
    window: tauri::Window,
    registry: tauri::State<'_, runtimes::Runtimes>,
    runtime: Option<projects::Runtime>,
    root: String,
    restart: Option<u64>,
    acknowledge_loss: bool,
) -> Result<projects::Snapshot, runtime::Error> {
    registry
        .get(runtime)
        .connect(window.label(), &root, restart, acknowledge_loss)
}

#[tauri::command]
fn project_view(
    window: tauri::Window,
    registry: tauri::State<'_, runtimes::Runtimes>,
) -> Result<projects::View, runtime::Error> {
    registry.view(window.label())
}

#[tauri::command]
fn project_events(
    window: tauri::Window,
    registry: tauri::State<'_, runtimes::Runtimes>,
    runtime: Option<projects::Runtime>,
    root: String,
    generation: u64,
    after: u64,
) -> Result<projects::Events, runtime::Error> {
    registry
        .get(runtime)
        .events(window.label(), &root, generation, after)
}

#[tauri::command]
async fn project_browser(
    path: Option<String>,
    show_hidden: bool,
) -> Result<browser::Listing, runtime::Error> {
    tauri::async_runtime::spawn_blocking(move || browser::browse(path, show_hidden))
        .await
        .map_err(|_| runtime::Error::Transport)?
}

#[tauri::command]
fn default_project_root() -> Result<String, runtime::Error> {
    std::env::current_dir()
        .ok()
        .and_then(|root| root.to_str().map(str::to_owned))
        .ok_or(runtime::Error::InvalidProject)
}

#[tauri::command]
async fn conversation_request(
    app: tauri::AppHandle,
    window: tauri::Window,
    runtime: Option<projects::Runtime>,
    root: String,
    generation: u64,
    action: projects::Action,
) -> Result<serde_json::Value, runtime::Error> {
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<runtimes::Runtimes>()
            .get(runtime)
            .request(&label, &root, generation, action)
    })
    .await
    .map_err(|_| runtime::Error::Transport)?
}

#[tauri::command]
async fn approval_reply(
    app: tauri::AppHandle,
    runtime: Option<projects::Runtime>,
    root: String,
    generation: u64,
    reply: projects::ApprovalReply,
) -> Result<serde_json::Value, runtime::Error> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<runtimes::Runtimes>()
            .get(runtime)
            .approval_reply(&root, generation, reply)
    })
    .await
    .map_err(|_| runtime::Error::Transport)?
}

#[tauri::command]
fn conversation_view(
    window: tauri::Window,
    registry: tauri::State<'_, runtimes::Runtimes>,
    runtime: Option<projects::Runtime>,
    root: String,
    generation: u64,
    thread_id: String,
    after: Option<u64>,
) -> Result<serde_json::Value, runtime::Error> {
    match after {
        Some(after) => registry.get(runtime).conversation_since(
            window.label(),
            &root,
            generation,
            &thread_id,
            Some(after),
        ),
        None => registry
            .get(runtime)
            .conversation(window.label(), &root, generation, &thread_id),
    }
}

#[tauri::command]
async fn conversation_image(
    app: tauri::AppHandle,
    window: tauri::Window,
    runtime: Option<projects::Runtime>,
    root: String,
    generation: u64,
    thread_id: String,
    path: String,
) -> Result<String, runtime::Error> {
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let path = app
            .state::<runtimes::Runtimes>()
            .get(runtime)
            .image_path(&label, &root, generation, &thread_id, &path)?;
        media::read(&path)
    })
    .await
    .map_err(|_| runtime::Error::Transport)?
}

#[tauri::command]
async fn reconnect_opencode(
    app: tauri::AppHandle,
    window: tauri::Window,
    root: String,
    generation: u64,
) -> Result<projects::Snapshot, runtime::Error> {
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<runtimes::Runtimes>()
            .get(Some(projects::Runtime::Opencode))
            .reconnect_idle(&label, &root, generation)
    })
    .await
    .map_err(|_| runtime::Error::Transport)?
}

#[tauri::command]
fn cancel_exit(registry: tauri::State<'_, runtimes::Runtimes>) {
    registry.cancel_exit();
}

#[tauri::command]
async fn confirm_exit(app: tauri::AppHandle) -> Result<(), runtime::Error> {
    finish_exit(app, true).await
}

async fn finish_exit(app: tauri::AppHandle, confirmed: bool) -> Result<(), runtime::Error> {
    if editor_install::busy() {
        app.state::<editor::Editor>().request_exit();
        return Err(runtime::Error::Busy);
    }
    if app.state::<editor::Editor>().has_dirty() {
        app.state::<editor::Editor>().request_exit();
        return Err(runtime::Error::ConfirmationRequired);
    }
    if app
        .state::<ExitState>()
        .started
        .swap(true, Ordering::SeqCst)
    {
        return Err(runtime::Error::Busy);
    }
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        worker_app.state::<runtimes::Runtimes>().shutdown(confirmed)
    })
    .await
    .map_err(|_| runtime::Error::Transport)
    .and_then(|result| result);
    match result {
        Ok(()) => {
            app.state::<editor_lsp::Servers>().stop_all();
            app.state::<ExitState>().ready.store(true, Ordering::SeqCst);
            app.exit(0);
            Ok(())
        }
        Err(error) => {
            app.state::<ExitState>()
                .started
                .store(false, Ordering::SeqCst);
            Err(error)
        }
    }
}

fn request_exit(app: &tauri::AppHandle) {
    if app.state::<editor::Editor>().has_dirty() || editor_install::busy() {
        app.state::<editor::Editor>().request_exit();
        return;
    }
    // Failure leaves the application open rather than silently terminating work.
    if let Ok(false) = app.state::<runtimes::Runtimes>().request_exit() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = finish_exit(app, false).await;
        });
    }
}

fn main() {
    tauri::Builder::default()
        .manage(editor::Editor::default())
        .manage(editor_lsp::Servers::default())
        .manage(runtimes::Runtimes::default())
        .manage(ExitState::default())
        .invoke_handler(tauri::generate_handler![
            editor_install_plan,
            editor_install_busy,
            editor_install,
            editor_request,
            editor_dirty,
            editor_exit_pending,
            editor_resolve_exit,
            editor_lsp_discover,
            editor_lsp_start,
            editor_lsp_request,
            editor_lsp_notify,
            editor_lsp_poll,
            editor_lsp_stop,
            connect_project,
            reconnect_opencode,
            default_project_root,
            project_browser,
            conversation_request,
            conversation_view,
            conversation_image,
            approval_reply,
            project_view,
            project_events,
            cancel_exit,
            confirm_exit
        ])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. }
                if window.app_handle().webview_windows().len() == 1 =>
            {
                api.prevent_close();
                request_exit(window.app_handle());
            }
            tauri::WindowEvent::Destroyed => {
                window
                    .app_handle()
                    .state::<runtimes::Runtimes>()
                    .detach_window(window.label());
                window
                    .app_handle()
                    .state::<editor::Editor>()
                    .detach(window.label());
                window
                    .app_handle()
                    .state::<editor_lsp::Servers>()
                    .stop_window(window.label());
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("failed to build CorpsTex")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !app.state::<ExitState>().ready.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    request_exit(app);
                }
            }
        });
}
