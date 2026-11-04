#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod media;
mod opencode_credentials;
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
async fn opencode_save_key(
    app: tauri::AppHandle,
    window: tauri::Window,
    provider: String,
    api_key: String,
) -> Result<(), runtime::Error> {
    if app
        .state::<runtimes::Runtimes>()
        .view(window.label())?
        .exit_requested
    {
        return Err(runtime::Error::ExitPending);
    }
    let guard = opencode_credentials::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        opencode_credentials::save(provider, api_key, guard)
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
    if opencode_credentials::saving() {
        return Err(runtime::Error::OpenCodeKeyBusy);
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
        .manage(runtimes::Runtimes::default())
        .manage(ExitState::default())
        .invoke_handler(tauri::generate_handler![
            connect_project,
            opencode_save_key,
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
            tauri::WindowEvent::Destroyed => window
                .app_handle()
                .state::<runtimes::Runtimes>()
                .detach_window(window.label()),
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
