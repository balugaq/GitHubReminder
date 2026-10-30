mod github;
mod store;
mod winembed;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use store::{Config, Watch};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_notification::NotificationExt;

pub struct AppState {
    path: PathBuf,
    config: Mutex<Config>,
}

impl AppState {
    fn with_config<T>(&self, f: impl FnOnce(&mut Config) -> T) -> T {
        let mut cfg = self.config.lock().unwrap();
        let out = f(&mut cfg);
        store::save(&self.path, &cfg);
        out
    }
}

fn new_watch_id() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("w{n}")
}

fn validate_repo(repo: &str) -> Result<(), String> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
        return Err("仓库格式应为 owner/name".into());
    }
    Ok(())
}

#[tauri::command]
fn get_config(state: tauri::State<Arc<AppState>>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_token(state: tauri::State<Arc<AppState>>, token: Option<String>) -> Config {
    let t = token.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    state.with_config(|cfg| cfg.token = t);
    state.config.lock().unwrap().clone()
}

// tauri 命令参数需平铺供前端 invoke，不收拢为结构体
#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn add_watch(
    state: tauri::State<Arc<AppState>>,
    repo: String,
    number: Option<u64>,
    issues: bool,
    prs: bool,
    releases: bool,
    comments: bool,
    state_change: bool,
) -> Result<Config, String> {
    let repo = repo.trim().trim_matches('/').to_string();
    validate_repo(&repo)?;
    let number = number.filter(|n| *n > 0);
    if number.is_some() && !comments && !state_change {
        return Err("单条监听至少要勾选一项（评论/状态变更）".into());
    }
    if number.is_none() && !issues && !prs && !releases {
        return Err("仓库监听至少要勾选一项（Issue/PR/Release）".into());
    }
    {
        let cfg = state.config.lock().unwrap();
        if cfg.watches.iter().any(|w| {
            w.repo.eq_ignore_ascii_case(&repo) && w.number == number
        }) {
            return Err("该监听对象已在列表中".into());
        }
    }
    state.with_config(|cfg| {
        cfg.watches.push(Watch {
            id: new_watch_id(),
            repo,
            number,
            issues,
            prs,
            releases,
            comments,
            state_change,
        })
    });
    Ok(state.config.lock().unwrap().clone())
}

#[tauri::command]
fn remove_watch(state: tauri::State<Arc<AppState>>, id: String) -> Config {
    state.with_config(|cfg| {
        cfg.watches.retain(|w| w.id != id);
        cfg.cursor_issues.remove(&id);
        cfg.cursor_release.remove(&id);
        cfg.cursor_comment.remove(&id);
        cfg.cursor_state.remove(&id);
    });
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_proxy(state: tauri::State<Arc<AppState>>, proxy: Option<String>) -> Config {
    let p = proxy.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    state.with_config(|cfg| cfg.proxy = p);
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_notify(state: tauri::State<Arc<AppState>>, notify: bool) -> Config {
    state.with_config(|cfg| cfg.notify = notify);
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enable: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    if enable {
        autolaunch.enable().map_err(|e| e.to_string())?;
    } else {
        autolaunch.disable().map_err(|e| e.to_string())?;
    }
    autolaunch.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

fn toast_summary(events: &[github::Event]) -> String {
    let first = &events[0];
    let head = format!(
        "{}{} {}",
        first.repo,
        first.number.map(|n| format!(" #{n}")).unwrap_or_default(),
        first.title
    );
    if events.len() == 1 {
        head
    } else {
        format!("{head}\n…以及另外 {} 条新动态", events.len() - 1)
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };

    let toggle = MenuItem::with_id(app, "toggle", "显示 / 隐藏", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("GitHub 通知")
        .menu(&menu)
        .on_menu_event(|app, ev| match ev.id.as_ref() {
            "toggle" => toggle_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|_tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                toggle_main_window(_tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn toggle_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        match w.is_visible() {
            Ok(true) => {
                let _ = w.hide();
            }
            _ => {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
    }
}

fn spawn_poller(app: tauri::AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        println!("[poll] 轮循协程已启动, 间隔 60 秒");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let (events, err) = {
                let mut cfg = state.config.lock().unwrap().clone();
                let (events, err) = github::poll_all(&mut cfg).await;
                // 只有游标可能变化，无事件时也照常落盘（增量小，可接受）
                *state.config.lock().unwrap() = cfg;
                store::save(&state.path, &state.config.lock().unwrap());
                (events, err)
            };
            if let Some(e) = err {
                let _ = app.emit("ghr-poll-error", e);
            }
            if !events.is_empty() {
                let _ = app.emit("ghr-events", &events);
                if state.config.lock().unwrap().notify {
                    let _ = app
                        .notification()
                        .builder()
                        .title("GitHub 通知")
                        .body(toast_summary(&events))
                        .show();
                }
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let path = app
                .path()
                .app_data_dir()
                .map(|d| d.join("config.json"))
                .unwrap_or_else(|_| PathBuf::from("config.json"));
            let cfg = store::load(&path);
            println!(
                "[init] 配置: {} ({} 个监听, token: {})",
                path.display(),
                cfg.watches.len(),
                if cfg.token.is_some() { "已设置" } else { "未设置" }
            );
            let state = Arc::new(AppState {
                path,
                config: Mutex::new(cfg),
            });
            spawn_poller(app.handle().clone(), state.clone());
            setup_tray(app)?;
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            // 点关闭 = 隐藏到托盘，退出走托盘菜单
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            winembed::set_window_mode,
            get_config,
            set_token,
            set_proxy,
            set_notify,
            set_autostart,
            get_autostart,
            add_watch,
            remove_watch
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
