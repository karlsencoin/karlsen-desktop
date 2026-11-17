//! Karlsen Desktop v3 - Tauri backend.

mod child;
mod emission;
mod job;
mod miner;
mod miner_settings;
mod node;
mod settings;

use miner::{MinerManager, MinerStatus};
use miner_settings::MinerSettings;
use node::{NodeManager, NodeStatus};
use settings::Settings;
use std::path::PathBuf;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, RunEvent, State, WindowEvent};
use tokio::sync::Mutex;

struct AppState {
    node: NodeManager,
    settings: Mutex<Settings>,
    miner: MinerManager,
    miner_settings: Mutex<MinerSettings>,
    config_dir: PathBuf,
    default_data_dir: PathBuf,
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

async fn start_node_from_settings(app: &AppHandle, state: &AppState) -> anyhow::Result<()> {
    let s = state.settings.lock().await.clone();
    let data_dir = s.resolved_data_dir(&state.default_data_dir);
    // node_args resolves bootstrap peer host names (blocking DNS), keep it off the async runtime.
    let args = {
        let (s, dir) = (s.clone(), data_dir.clone());
        tokio::task::spawn_blocking(move || s.node_args(&dir)).await?
    };
    state.node.start(app, data_dir, args, s.auto_restart_node).await
}

/// karlsend's own default application directory (rusty-karlsen `get_app_dir()`):
/// `%LOCALAPPDATA%\\rusty-karlsen` on Windows, `~/.rusty-karlsen` elsewhere.
/// Desktop uses the same folder by default, so an existing node's chain data is reused
/// and the database never lands in a roaming profile.
fn karlsend_default_app_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    #[cfg(windows)]
    return Ok(app.path().local_data_dir()?.join("rusty-karlsen"));
    #[cfg(not(windows))]
    return Ok(app.path().home_dir()?.join(".rusty-karlsen"));
}

#[tauri::command]
async fn node_status(state: State<'_, AppState>) -> CmdResult<NodeStatus> {
    Ok(state.node.status().await)
}

#[tauri::command]
fn node_logs(state: State<'_, AppState>) -> Vec<String> {
    state.node.logs()
}

#[tauri::command]
async fn node_start(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    start_node_from_settings(&app, &state).await.map_err(err)
}

#[tauri::command]
async fn node_stop(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.node.stop(&app).await.map_err(err)
}

#[tauri::command]
async fn node_restart(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.node.stop(&app).await.map_err(err)?;
    start_node_from_settings(&app, &state).await.map_err(err)
}

#[tauri::command]
async fn node_delete_and_resync(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.node.delete_and_resync(&app).await.map_err(err)
}

/// Mainnet block reward schedule at a DAA score (pure computation, no node call).
#[tauri::command]
fn block_reward_info(daa_score: u64) -> emission::BlockRewardInfo {
    emission::block_reward_info(daa_score)
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(state.settings.lock().await.clone())
}

/// Save settings. Node-affecting changes take effect after a node restart,
/// which the UI offers explicitly.
#[tauri::command]
async fn save_settings(state: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    settings.save(&state.config_dir).map_err(err)?;
    *state.settings.lock().await = settings;
    Ok(())
}

/// Start solo mining on the local node. The miner waits by itself until the node is synced.
async fn start_miner_from_settings(app: &AppHandle, state: &AppState) -> anyhow::Result<()> {
    let s = state.miner_settings.lock().await.clone();
    let node_rpc_listen = state.settings.lock().await.rpc_listen.clone();
    let work_dir = state.miner.work_dir();
    let launch = miner::Launch { miner_args: s.miner_args(&node_rpc_listen, work_dir).map_err(anyhow::Error::msg)? };
    state.miner.start(app, launch).await
}

#[tauri::command]
async fn miner_status(state: State<'_, AppState>) -> CmdResult<MinerStatus> {
    Ok(state.miner.status().await)
}

#[tauri::command]
fn miner_logs(state: State<'_, AppState>) -> Vec<String> {
    state.miner.logs()
}

#[tauri::command]
async fn miner_start(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    start_miner_from_settings(&app, &state).await.map_err(err)
}

#[tauri::command]
async fn miner_stop(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.miner.stop(&app).await.map_err(err)
}

#[tauri::command]
async fn get_miner_settings(state: State<'_, AppState>) -> CmdResult<MinerSettings> {
    Ok(state.miner_settings.lock().await.clone())
}

/// Save miner settings. Invalid values are refused here, not only in the UI.
/// Changes apply the next time the miner is started.
#[tauri::command]
async fn save_miner_settings(state: State<'_, AppState>, settings: MinerSettings) -> CmdResult<()> {
    settings.validate()?;
    settings.save(&state.config_dir).map_err(err)?;
    *state.miner_settings.lock().await = settings;
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch focuses the running instance instead of
            // starting another karlsend on the same ports.
            show_main_window(app);
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let default_data_dir = karlsend_default_app_dir(app.handle())?;
            let settings = Settings::load(&config_dir);
            let auto_start = settings.auto_start_node;
            let miner_settings = MinerSettings::load(&config_dir);
            let auto_start_miner = miner_settings.auto_start;
            // Working folder of the miner (local, non-roaming data).
            let miner_root = app.path().app_local_data_dir()?.join("miner");

            app.manage(AppState {
                node: NodeManager::new(),
                settings: Mutex::new(settings),
                miner: MinerManager::new(miner_root),
                miner_settings: Mutex::new(miner_settings),
                config_dir,
                default_data_dir,
            });

            // Tray menu: show window / quit.
            let show = MenuItem::with_id(app, "show", "Show Karlsen Desktop", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Karlsen Desktop")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            if auto_start {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    if let Err(e) = start_node_from_settings(&handle, &state).await {
                        log::error!("auto-start failed: {e}");
                    }
                });
            }
            if auto_start_miner {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    if let Err(e) = start_miner_from_settings(&handle, &state).await {
                        log::error!("miner auto-start failed: {e}");
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let background = tauri::async_runtime::block_on(async {
                    app.state::<AppState>().settings.lock().await.run_in_background
                });
                if background {
                    // Hide to tray; the node (and the miner, if started) keep running.
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            node_status,
            node_logs,
            node_start,
            node_stop,
            node_restart,
            node_delete_and_resync,
            get_settings,
            save_settings,
            block_reward_info,
            miner_status,
            miner_logs,
            miner_start,
            miner_stop,
            get_miner_settings,
            save_miner_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Karlsen Desktop");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { .. } = event {
            // Always stop karlsend before the process exits so RocksDB closes cleanly.
            let app = app.clone();
            tauri::async_runtime::block_on(async move {
                let state = app.state::<AppState>();
                // Mining first: the miner must not lose its node mid-submit, and karlsend then
                // gets its full shutdown window.
                let _ = state.miner.stop(&app).await;
                let _ = state.node.stop(&app).await;
            });
        }
    });
}
