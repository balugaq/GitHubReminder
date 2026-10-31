mod github;
mod store;
mod winembed;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use store::{Config, StoredEvent, Watch};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_notification::NotificationExt;

pub struct AppState {
    path: PathBuf,
    config: Mutex<Config>,
    // 上次轮询完成的 Unix 秒，供前端 hint 显示
    last_poll: Mutex<Option<u64>>,
    // 轮循互斥：一轮超过 60s 时定时器会并发再起一轮，两轮共用旧游标会重复通知
    polling: AtomicBool,
    // 已弹过 toast 且用户还没读掉的 issue/PR（"repo#n"），点击已读后由 mark_read 清除
    notified: Mutex<HashSet<String>>,
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
    closed_issues: bool,
    closed_prs: bool,
    comments: bool,
    state_change: bool,
) -> Result<Config, String> {
    let repo = repo.trim().trim_matches('/').to_string();
    validate_repo(&repo)?;
    let number = number.filter(|n| *n > 0);
    if number.is_some() && !comments && !state_change {
        return Err("单条监听至少要勾选一项（评论/状态变更）".into());
    }
    if number.is_none() && !issues && !prs && !releases && !closed_issues && !closed_prs {
        return Err("仓库监听至少要勾选一项（Issue/PR/Release/关Issue/关PR）".into());
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
            closed_issues,
            closed_prs,
            comments,
            state_change,
            title: None,
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        })
    });
    Ok(state.config.lock().unwrap().clone())
}

#[tauri::command]
fn update_watch(state: tauri::State<Arc<AppState>>, id: String, title: Option<String>) -> Config {
    // 空串 = 恢复默认显示名
    let title = title.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    state.with_config(|cfg| {
        if let Some(w) = cfg.watches.iter_mut().find(|w| w.id == id) {
            w.title = title;
        }
    });
    state.config.lock().unwrap().clone()
}

// 管理窗口在 tauri.conf.json 静态声明（visible:false），这里只负责唤起；
// 运行时建窗在 Windows 上有白屏/不渲染的毛病，故不走 WebviewWindowBuilder
#[tauri::command]
fn open_manage(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("manage") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    Err("管理窗口未初始化".into())
}

#[tauri::command]
fn remove_watch(state: tauri::State<Arc<AppState>>, id: String) -> Config {
    state.with_config(|cfg| {
        cfg.watches.retain(|w| w.id != id);
        cfg.cursor_issues.remove(&id);
        cfg.cursor_release.remove(&id);
        cfg.cursor_comment.remove(&id);
        cfg.cursor_state.remove(&id);
        cfg.cursor_closed.remove(&id);
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
    // 评论不在 toast 里展示内容，统一叫"有新消息"，内容去事件流里看
    let title = if first.kind == "comment" {
        "有新消息".to_string()
    } else {
        first.title.clone()
    };
    let head = format!(
        "{}{} {}",
        first.repo,
        first.number.map(|n| format!(" #{n}")).unwrap_or_default(),
        title
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
                // 这是主人主动藏，Win+D 守护协程见到 USER_HIDDEN 要放手
                winembed::USER_HIDDEN.store(true, Ordering::Relaxed);
                let _ = w.hide();
            }
            _ => {
                winembed::USER_HIDDEN.store(false, Ordering::Relaxed);
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
    }
}

const MAX_EVENTS: usize = 200;

// 新事件落库（防重启丢失）+ 按仓库/单条两个维度累计各类型次数
fn record_events(cfg: &mut Config, events: &[github::Event]) {
    for ev in events {
        cfg.events.push(StoredEvent {
            kind: ev.kind.clone(),
            repo: ev.repo.clone(),
            number: ev.number,
            title: ev.title.clone(),
            url: ev.url.clone(),
            created_at: ev.created_at.clone(),
            read: false,
        });
        let bump = |m: &mut std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>>, key: String| {
            *m.entry(key).or_default().entry(ev.kind.clone()).or_insert(0) += 1;
        };
        bump(&mut cfg.stats, ev.repo.clone());
        if let Some(n) = ev.number {
            bump(&mut cfg.stats, format!("{}#{n}", ev.repo));
        }
    }
    let len = cfg.events.len();
    if len > MAX_EVENTS {
        cfg.events.drain(..len - MAX_EVENTS);
    }
}

// 轮询 + 落盘 + 事件广播，轮循协程和手动刷新共用；返回完成时刻的 Unix 秒
async fn do_poll(app: &tauri::AppHandle, state: &AppState) -> u64 {
    // 上一轮还没跑完就跳过本轮，避免两轮共用旧游标重复通知
    if state
        .polling
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        println!("[poll {}] 上一轮仍在进行，跳过本轮", github::now_str());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return now;
    }
    let out = do_poll_inner(app, state).await;
    state.polling.store(false, Ordering::Release);
    out
}

async fn do_poll_inner(app: &tauri::AppHandle, state: &AppState) -> u64 {
    let (events, err) = {
        let mut cfg = state.config.lock().unwrap().clone();
        let (events, err) = github::poll_all(&mut cfg).await;
        if !events.is_empty() {
            record_events(&mut cfg, &events);
        }
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
        // 同一 issue/PR 已弹过且用户未读则不再弹 toast；release 无编号不参与去重
        let mut toasting: Vec<github::Event> = Vec::new();
        {
            let mut notified = state.notified.lock().unwrap();
            for ev in &events {
                match ev.number {
                    Some(n) => {
                        let key = format!("{}#{n}", ev.repo);
                        if notified.insert(key) {
                            toasting.push(ev.clone());
                        }
                    }
                    None => toasting.push(ev.clone()),
                }
            }
        }
        if !toasting.is_empty()
            && state.config.lock().unwrap().notify
        {
            let _ = app
                .notification()
                .builder()
                .title("GitHub 通知")
                .body(toast_summary(&toasting))
                .show();
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    *state.last_poll.lock().unwrap() = Some(now);
    let _ = app.emit("ghr-poll-done", now);
    now
}

// Win+D 会藏掉贴桌面的嵌入窗口且托盘唤不回（不在任务栏），定时检测自动唤回
fn spawn_visibility_guard(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if let Some(w) = app.get_webview_window("main") {
                winembed::restore_if_hidden_by_win_d(&w);
            }
        }
    });
}

fn spawn_poller(app: tauri::AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        println!("[poll] 轮循协程已启动, 间隔 60 秒");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            do_poll(&app, &state).await;
        }
    });
}

// 前端点 🔄 立即刷新；返回完成时刻供 hint 显示
#[tauri::command]
async fn poll_now(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<u64, String> {
    Ok(do_poll(&app, &state).await)
}

// 用户点开事件卡即视为该 issue/PR 已读，之后它的新闻动才允许再弹 toast
#[tauri::command]
fn mark_read(state: tauri::State<Arc<AppState>>, repo: String, number: Option<u64>) {
    if let Some(n) = number {
        state.with_config(|cfg| {
            for ev in cfg.events.iter_mut() {
                if ev.repo.eq_ignore_ascii_case(&repo) && ev.number == Some(n) {
                    ev.read = true;
                }
            }
        });
        state.notified.lock().unwrap().remove(&format!("{repo}#{n}"));
    }
}

#[tauri::command]
fn get_events(state: tauri::State<Arc<AppState>>) -> Vec<StoredEvent> {
    state.config.lock().unwrap().events.clone()
}

#[tauri::command]
fn get_stats(
    state: tauri::State<Arc<AppState>>,
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>> {
    state.config.lock().unwrap().stats.clone()
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
            // 首次使用（config.json 不存在）时自带一条 microsoft/vscode 监听，开箱即用
            let first_run = !path.exists();
            let mut cfg = store::load(&path);
            if first_run {
                cfg.watches.push(Watch {
                    id: new_watch_id(),
                    repo: "microsoft/vscode".into(),
                    number: None,
                    issues: true,
                    prs: true,
                    releases: false,
                    closed_issues: false,
                    closed_prs: false,
                    comments: true,
                    state_change: true,
                    title: None,
                    created: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                });
                store::save(&path, &cfg);
                println!("[init] 首次使用，已预置监听 microsoft/vscode (issues+prs)");
            }
            println!(
                "[init] 配置: {} ({} 个监听, token: {})",
                path.display(),
                cfg.watches.len(),
                if cfg.token.is_some() { "已设置" } else { "未设置" }
            );
            let state = Arc::new(AppState {
                path,
                config: Mutex::new(cfg),
                last_poll: Mutex::new(None),
                polling: AtomicBool::new(false),
                notified: Mutex::new(HashSet::new()),
            });
            spawn_poller(app.handle().clone(), state.clone());
            spawn_visibility_guard(app.handle().clone());
            setup_tray(app)?;
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            // 两个窗口点关闭都只隐藏：主窗口藏到托盘，管理窗口藏起来以便再次唤起
            if matches!(window.label(), "main" | "manage")
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                if window.label() == "main" {
                    winembed::USER_HIDDEN.store(true, Ordering::Relaxed);
                }
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
            update_watch,
            remove_watch,
            open_manage,
            poll_now,
            mark_read,
            get_events,
            get_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
