mod github;
mod qqrelay;
mod stats;
mod store;
mod winembed;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use store::{Config, Entity, Watch};
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
    // 内置 QQ 中转服务的运行句柄（自动配置开启且配置齐全时在跑）
    relay: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    // 上次窗口位置落盘时刻（毫秒），拖动/拉伸高频触发靠它节流
    last_win_save: Mutex<u64>,
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
fn set_token(state: tauri::State<Arc<AppState>>, token: Option<String>) -> Result<Config, String> {
    let t = token.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    store::save_token(t.as_deref())?;
    state.with_config(|cfg| cfg.token = t);
    Ok(state.config.lock().unwrap().clone())
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

// 编辑监听：repo/number 不可改，其余字段 None = 不动；保存后校验至少勾选一项
// tauri 命令参数需平铺供前端 invoke，不收拢为结构体
#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn update_watch(
    state: tauri::State<Arc<AppState>>,
    id: String,
    title: Option<String>,
    issues: Option<bool>,
    prs: Option<bool>,
    releases: Option<bool>,
    closed_issues: Option<bool>,
    closed_prs: Option<bool>,
    comments: Option<bool>,
    state_change: Option<bool>,
) -> Result<Config, String> {
    // 传了 title：非空设为自定义名，空串恢复默认（清掉）；没传 = 不动
    let raw_title = title;
    let title = raw_title
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut found = false;
    let mut is_repo = false;
    state.with_config(|cfg| {
        let Some(w) = cfg.watches.iter_mut().find(|w| w.id == id) else {
            return;
        };
        found = true;
        is_repo = w.number.is_none();
        if let Some(t) = title.clone() {
            w.title = Some(t);
        } else if raw_title.is_some() {
            w.title = None;
        }
        if let Some(v) = issues {
            w.issues = v;
        }
        if let Some(v) = prs {
            w.prs = v;
        }
        if let Some(v) = releases {
            w.releases = v;
        }
        if let Some(v) = closed_issues {
            w.closed_issues = v;
        }
        if let Some(v) = closed_prs {
            w.closed_prs = v;
        }
        if let Some(v) = comments {
            w.comments = v;
        }
        if let Some(v) = state_change {
            w.state_change = v;
        }
    });
    // 至少一项：仓库级五选一，单条级二选一
    if !found {
        return Err("监听不存在".into());
    }
    if is_repo {
        let ok = state
            .config
            .lock()
            .unwrap()
            .watches
            .iter()
            .find(|w| w.id == id)
            .is_some_and(|w| w.issues || w.prs || w.releases || w.closed_issues || w.closed_prs);
        if !ok {
            return Err("仓库监听至少要勾选一项（Issue/PR/Release/关Issue/关PR）".into());
        }
    } else {
        let ok = state
            .config
            .lock()
            .unwrap()
            .watches
            .iter()
            .find(|w| w.id == id)
            .is_some_and(|w| w.comments || w.state_change);
        if !ok {
            return Err("单条监听至少要勾选一项（评论/状态变更）".into());
        }
    }
    Ok(state.config.lock().unwrap().clone())
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
fn set_poll_interval(state: tauri::State<Arc<AppState>>, secs: Option<u64>) -> Config {
    state.with_config(|cfg| cfg.poll_interval = secs.filter(|s| *s > 0));
    state.config.lock().unwrap().clone()
}

// 用户的 GitHub 用户名：用于判断 issue 的 assignee 里是否有本人
#[tauri::command]
fn set_github_username(state: tauri::State<Arc<AppState>>, username: Option<String>) -> Config {
    let u = username.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    state.with_config(|cfg| cfg.github_username = u);
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_qq_notify(state: tauri::State<Arc<AppState>>, notify: bool) -> Config {
    state.with_config(|cfg| cfg.qq_notify = notify);
    state.config.lock().unwrap().clone()
}

// 自动配置 = 是否由本应用启动内置 QQ 中转服务（关着表示外部已有服务在跑）
#[tauri::command]
fn set_qq_autoconfig(state: tauri::State<Arc<AppState>>, enable: bool) -> Config {
    state.with_config(|cfg| cfg.qq_autoconfig = enable);
    restart_relay(&state);
    state.config.lock().unwrap().clone()
}

// 端口 + 三凭证一并保存，全部无默认值；改完即按需重启内置中转
#[tauri::command]
fn set_qq_settings(
    state: tauri::State<Arc<AppState>>,
    port: Option<u16>,
    app_id: Option<String>,
    app_secret: Option<String>,
    openid: Option<String>,
) -> Result<Config, String> {
    let trim = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    // AppSecret 存凭据管理器：留空 = 不改已存的密钥，填了新值才覆盖。
    // 写失败时整包设置不落盘，避免前端显示已保存但实际没生效。
    let new_secret = trim(app_secret);
    if let Some(s) = new_secret.as_deref() {
        store::save_qq_app_secret(Some(s))?;
    }
    state.with_config(|cfg| {
        cfg.qq_relay_port = port;
        cfg.qq_app_id = trim(app_id);
        if let Some(s) = new_secret.clone() {
            cfg.qq_app_secret = Some(s);
        }
        cfg.qq_openid = trim(openid);
    });
    restart_relay(&state);
    Ok(state.config.lock().unwrap().clone())
}

fn relay_cfg_of(cfg: &Config) -> Option<Arc<qqrelay::RelayCfg>> {
    qqrelay::RelayCfg::from_parts(
        cfg.qq_relay_port,
        cfg.qq_app_id.as_deref(),
        cfg.qq_app_secret.as_deref(),
        cfg.qq_openid.as_deref(),
    )
    .map(Arc::new)
}

// 按当前配置起停内置中转：自动配置开且配置齐全才跑，否则停掉旧的
fn restart_relay(state: &AppState) {
    if let Some(h) = state.relay.lock().unwrap().take() {
        h.abort();
    }
    let cfg = state.config.lock().unwrap().clone();
    if !cfg.qq_autoconfig {
        println!("[qq] 自动配置未开启，内置中转不启动");
        return;
    }
    let Some(rc) = relay_cfg_of(&cfg) else {
        println!("[qq] 自动配置已开但端口/凭证不全，内置中转不启动");
        return;
    };
    let handle = tauri::async_runtime::spawn(qqrelay::run_server(rc));
    *state.relay.lock().unwrap() = Some(handle);
}

// QQ 推送正文：每条事件一块三行卡片；全部发出不折叠，
// 超长交给 qqrelay::split_text 按行分段（1900 字上限，链接不会被截断）
// 格式（头部一行由 spawn_qq_push 拼上）：
//   [GitHubReminder] 通知 · GitHub 动态
//   新 PR #459 · owner/repo
//   [@me] [author] · 标题（评论统一显示「有新消息」）
//   https://github.com/owner/repo/pull/459
//
// [@me] 仅当该事件的 assignee 列表里有本人（设置里填的 GitHub 用户名）时出现；
// [author] 是提 issue / pr 的人。两者都没有时标题行保持纯标题。
// 头一行带动作（新/已关闭/状态变更/新评论），主人 2026-11-07 要求补事件标注。
fn event_kind_label(ev: &github::Event) -> &'static str {
    match ev.kind.as_str() {
        "release" => "新 Release",
        "pr" => "新 PR",
        "closed_pr" => "已关闭 PR",
        "issue" => "新 Issue",
        "closed_issue" => "已关闭 Issue",
        // state/comment 按 URL 区分挂在 Issue 还是 PR 上
        "state" if ev.url.contains("/pull/") => "PR 状态变更",
        "state" => "Issue 状态变更",
        _ if ev.url.contains("/pull/") => "PR 新评论",
        _ => "Issue 新评论",
    }
}

fn qq_summary(events: &[github::Event], my_username: Option<&str>) -> String {
    let me = my_username
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let mut lines: Vec<String> = Vec::new();
    for ev in events {
        let head = match ev.number {
            Some(n) => format!("{} #{n} · {}", event_kind_label(ev), ev.repo),
            None => format!("{} · {}", event_kind_label(ev), ev.repo),
        };
        let title = if ev.kind == "comment" {
            "有新消息".to_string()
        } else {
            ev.title.clone()
        };
        let at_me = me.as_ref().is_some_and(|m| {
            ev.assignees.iter().any(|a| a.trim().to_lowercase() == *m)
        });
        let mut marks = String::new();
        if at_me {
            marks.push_str("[@me] ");
        }
        if !ev.author.is_empty() {
            marks.push_str(&format!("[{}] ", ev.author));
        }
        let title_line = if marks.is_empty() {
            title
        } else {
            format!("{marks}· {title}")
        };
        lines.push(head);
        lines.push(title_line);
        lines.push(ev.url.clone());
    }
    lines.join("\n")
}

// 有新事件时推送：自动配置开 = 直调内置中转（需端口+凭证齐全）；
// 自动配置关 = POST 到外部已有中转服务（只需端口，凭证由外部服务持有）
fn spawn_qq_push(cfg: &Config, events: &[github::Event]) {
    if !cfg.qq_notify || events.is_empty() {
        return;
    }
    let text = format!(
        "[GitHubReminder] 通知 · GitHub 动态\n{}",
        qq_summary(events, cfg.github_username.as_deref())
    );
    if cfg.qq_autoconfig {
        let Some(rc) = relay_cfg_of(cfg) else {
            println!("[qq] 自动配置开但端口/凭证未配全，跳过 QQ 推送");
            return;
        };
        tauri::async_runtime::spawn(async move {
            match qqrelay::send_to_qq(&rc, &text).await {
                Ok(n) => println!("[qq] 已推送 {n} 段"),
                Err(e) => println!("[qq] 推送失败（静默）: {e}"),
            }
        });
    } else {
        let Some(port) = cfg.qq_relay_port else {
            println!("[qq] 外部中转端口未配置，跳过 QQ 推送");
            return;
        };
        tauri::async_runtime::spawn(async move {
            match qqrelay::post_to_relay(port, &text).await {
                Ok(n) => println!("[qq] 经外部中转已推送 {n} 段"),
                Err(e) => println!("[qq] 推送失败（静默）: {e}"),
            }
        });
    }
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

// 记录主窗口位置/尺寸：内存即时更新，落盘按 1s 节流（拖动过程高频触发）；
// 主窗口最小尺寸（逻辑像素），与 tauri.conf 的 minWidth/minHeight 保持一致
const MIN_WINDOW_SIZE: f64 = 240.0;
// Windows 最小化会把窗口移到 (-32000,-32000)，这类坐标不是用户真实位置
const OFFSCREEN_XY_LIMIT: f64 = -10000.0;

fn min_window_px(window: &tauri::Window) -> f64 {
    MIN_WINDOW_SIZE * window.scale_factor().unwrap_or(1.0)
}

// 最小化/隐藏时的 Moved/Resized 会给出离屏坐标和极小尺寸，存下来会让下次启动恢复到屏幕外
fn rect_is_plausible(x: f64, y: f64, width: f64, height: f64, min_px: f64) -> bool {
    x > OFFSCREEN_XY_LIMIT
        && y > OFFSCREEN_XY_LIMIT
        && width >= min_px
        && height >= min_px
}

// 关闭隐藏前把节流计时清零再调一次，保证最终位置一定落盘
// 最大化时全屏矩形不入库（否则下次启动恢复成铺满屏的伪全屏），只记住「上次是最大化」；
// 没有历史矩形时什么都不存，启动按默认尺寸直接最大化
fn next_window_rect(
    prev: Option<store::WindowRect>,
    measured: store::WindowRect,
    maximized: bool,
) -> Option<store::WindowRect> {
    if maximized {
        prev.map(|mut r| {
            r.maximized = true;
            r
        })
    } else {
        Some(store::WindowRect {
            maximized: false,
            ..measured
        })
    }
}

fn remember_window(state: &AppState, window: &tauri::Window) {
    if window.is_minimized().unwrap_or(false) || !window.is_visible().unwrap_or(true) {
        return;
    }
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return;
    };
    let min_px = min_window_px(window);
    let maxed = window.is_maximized().unwrap_or(false);
    let rect = store::WindowRect {
        x: pos.x as f64,
        y: pos.y as f64,
        width: size.width as f64,
        height: size.height as f64,
        maximized: false,
    };
    // 最大化时全屏矩形本来就不入库，离屏/过小过滤只对普通矩形有意义
    if !maxed && !rect_is_plausible(rect.x, rect.y, rect.width, rect.height, min_px) {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let due = {
        let mut last = state.last_win_save.lock().unwrap();
        let due = now.saturating_sub(*last) >= 1000;
        if due {
            *last = now;
        }
        due
    };
    let mut cfg = state.config.lock().unwrap();
    cfg.window = next_window_rect(cfg.window, rect, maxed);
    if due {
        store::save(&state.path, &cfg);
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

// 托盘切换时顺带体检：离屏/过小的窗口会被拉回屏幕中央，光 show 唤不回来
fn main_window_geometry_ok(window: &tauri::WebviewWindow) -> bool {
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return false;
    };
    let min_px = MIN_WINDOW_SIZE * window.scale_factor().unwrap_or(1.0);
    rect_is_plausible(
        pos.x as f64,
        pos.y as f64,
        size.width as f64,
        size.height as f64,
        min_px,
    )
}

fn toggle_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let broken = !main_window_geometry_ok(&w);
        // 最小化的窗口 is_visible 仍为 true，不排除会把「托盘唤回」误判成「主人要隐藏」
        let shown = w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
        match shown {
            true if !broken => {
                // 这是主人主动藏，Win+D 守护协程见到 USER_HIDDEN 要放手
                winembed::USER_HIDDEN.store(true, Ordering::Relaxed);
                let _ = w.hide();
            }
            _ => {
                if broken {
                    let min_px = MIN_WINDOW_SIZE * w.scale_factor().unwrap_or(1.0);
                    let _ = w.set_min_size(Some(tauri::LogicalSize::new(
                        MIN_WINDOW_SIZE,
                        MIN_WINDOW_SIZE,
                    )));
                    let _ = w.set_size(tauri::PhysicalSize::new(
                        min_px.round() as u32,
                        min_px.round() as u32,
                    ));
                    let _ = w.center();
                }
                winembed::USER_HIDDEN.store(false, Ordering::Relaxed);
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
    }
}

const MAX_ENTITIES: usize = 200;

// 把某条事件里出现的人并进实体的 people（统一小写、去重）。GitHub login 大小写不敏感，
// 统一小写既能避免 "Octocat"/"octocat" 重复，也让前端 @用户名 直接做字符串比较。
fn merge_people(target: &mut Vec<String>, ev: &github::Event) {
    for p in std::iter::once(&ev.author).chain(ev.assignees.iter()) {
        let name = p.trim().to_lowercase();
        if !name.is_empty() && !target.contains(&name) {
            target.push(name);
        }
    }
}

// ---- 自动归类规则 ----------------------------------------------------------
// 规则：dependabot 发起的 PR 一旦 close / merge，直接归入「已解决」，不占未读。
// 关闭事件有两条来源：仓库级 poll_closed 的 closed_pr、单条监听的 state 变更
// （title 含「→ closed」），classify 都给出 state = "closed"；GitHub 里 merge
// 同样表现为 closed，故一条判定即可覆盖 close 与 merge。
// 规则按「批次」生效：只有本批次真出现关闭事件时才置 done，之后用户手动改的
// 状态不会被后续轮询反复覆盖。
fn is_dependabot_author(author: &str) -> bool {
    let a = author.trim().to_lowercase();
    matches!(
        a.as_str(),
        "dependabot"
            | "dependabot[bot]"
            | "dependabot-preview"
            | "dependabot-preview[bot]"
            | "dependabot-bot"
    )
}

fn auto_done_on_close(ev: &github::Event, state: &str) -> bool {
    state == "closed"
        && (ev.url.contains("/pull/") || ev.kind == "closed_pr")
        && is_dependabot_author(&ev.author)
}

// 把变化事件 upsert 进实体表：同一 repo+number 聚合为一卡，updated_at 取最新；
// 有新动态（时间更新或状态/类型变化）则 status 重置为未读。同时累计 stats。
fn record_entities(cfg: &mut Config, events: &[github::Event]) {
    // 本批次触发自动规则的实体；循环结束后统一应用，避免同批次后续事件把 done 冲掉
    let mut auto_done: Vec<(String, Option<u64>)> = Vec::new();
    for ev in events {
        let (kind, last_kind, state) = classify(ev);
        if auto_done_on_close(ev, &state) {
            auto_done.push((ev.repo.clone(), ev.number));
        }
        if let Some(e) = cfg.entities.iter_mut().find(|e| {
            e.repo.eq_ignore_ascii_case(&ev.repo) && e.number == ev.number
        }) {
            // 旧存档没有人物字段：后续有新事件时顺手补全。卡片作者取首个非空值，
            // 不被评论等后续事件覆盖；「涉及的人」则持续累积。
            if e.author.is_empty() && !ev.author.is_empty() {
                e.author = ev.author.clone();
            }
            merge_people(&mut e.people, ev);
            // 评论不改 state，不视为变化；其余按时间/状态/类型判断
            let changed = ev.created_at > e.updated_at
                || e.last_event_kind != last_kind
                || (!state.is_empty() && e.state != state);
            e.last_event_kind = last_kind.clone();
            e.updated_at = ev.created_at.clone();
            if !ev.title.is_empty() {
                e.title = ev.title.clone();
            }
            e.url = ev.url.clone();
            if !state.is_empty() {
                e.state = state.clone();
            }
            if changed {
                e.status = "unread".into();
            }
        } else {
            let mut people = Vec::new();
            merge_people(&mut people, ev);
            cfg.entities.push(Entity {
                kind,
                repo: ev.repo.clone(),
                number: ev.number,
                title: ev.title.clone(),
                url: ev.url.clone(),
                state,
                last_event_kind: last_kind,
                updated_at: ev.created_at.clone(),
                status: "unread".into(),
                pinned: false,
                author: ev.author.clone(),
                people,
            });
        }
        // 统计流水只记「哪个项目在什么时候来了一条通知」，占比由 stats::snapshot 现算
        let at = chrono::DateTime::parse_from_rfc3339(&ev.created_at)
            .map(|d| d.timestamp())
            .unwrap_or_else(|_| chrono::Utc::now().timestamp());
        cfg.stat_events.push(store::StatEvent {
            repo: ev.repo.clone(),
            at,
        });
    }
    stats::trim_log(&mut cfg.stat_events);
    // 自动规则最后统一应用：本批次里即使还夹着评论等事件把 status 重置成 unread，
    // 也以「已解决」为准。
    for e in cfg.entities.iter_mut() {
        if auto_done
            .iter()
            .any(|(r, n)| e.repo.eq_ignore_ascii_case(r) && *n == e.number)
        {
            e.status = "done".into();
        }
    }
    let len = cfg.entities.len();
    if len > MAX_ENTITIES {
        // 丢最旧的，但跳过置顶卡（钉住的不该因为久远被挤掉）
        cfg.entities.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
        let mut excess = len - MAX_ENTITIES;
        let mut i = 0;
        while excess > 0 && i < cfg.entities.len() {
            if cfg.entities[i].pinned {
                i += 1;
            } else {
                cfg.entities.remove(i);
                excess -= 1;
            }
        }
    }
}

// 从变化事件推断：实体类型 / 最近变更类型 / 当前状态
fn classify(ev: &github::Event) -> (String, String, String) {
    let is_pr = ev.url.contains("/pull/");
    match ev.kind.as_str() {
        "release" => ("release".into(), "release".into(), "published".into()),
        "state" => {
            let state = if ev.title.contains("→ closed") {
                "closed"
            } else {
                "open"
            };
            (
                (if is_pr { "pr" } else { "issue" }).into(),
                "state".into(),
                state.into(),
            )
        }
        "comment" => (
            (if is_pr { "pr" } else { "issue" }).into(),
            "comment".into(),
            String::new(),
        ),
        "closed_issue" | "closed_pr" => (
            (if is_pr { "pr" } else { "issue" }).into(),
            "closed".into(),
            "closed".into(),
        ),
        "issue" => ("issue".into(), "open".into(), "open".into()),
        "pr" => ("pr".into(), "open".into(), "open".into()),
        other => (other.to_string(), other.to_string(), String::new()),
    }
}

// 回补最近 14 天内创建、当前仍 open 的 Issue/PR：落库 + 前端事件流 + QQ/系统通知（不走未读去重）
#[tauri::command]
async fn backfill_14d(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>) -> Result<u64, String> {
    let st = state.inner().clone();
    let cfg = st.config.lock().unwrap().clone();
    let events = github::backfill_open_14d(&cfg).await?;
    let count = events.len() as u64;
    if count > 0 {
        {
            let mut c = st.config.lock().unwrap();
            record_entities(&mut c, &events);
            store::save(&st.path, &c);
        }
        let _ = app.emit("ghr-events", &events);
        let cur = st.config.lock().unwrap().clone();
        spawn_qq_push(&cur, &events);
        if cur.notify {
            let _ = app
                .notification()
                .builder()
                .title("GitHub 通知（14 天回补）")
                .body(format!("[回补] {}", toast_summary(&events)))
                .show();
        }
    }
    Ok(count)
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
        // 网络请求可能耗时十几秒，全程只改工作副本、不持锁；
        // 拿到结果后只把「游标 + 本轮新实体」合并回内存配置，
        // 不能整体覆盖——否则轮询期间用户改的状态/置顶/监听会被静默回滚
        let mut work = state.config.lock().unwrap().clone();
        let (events, err) = github::poll_all(&mut work).await;
        {
            let mut cfg = state.config.lock().unwrap();
            cfg.cursor_issues = work.cursor_issues;
            cfg.cursor_release = work.cursor_release;
            cfg.cursor_comment = work.cursor_comment;
            cfg.cursor_state = work.cursor_state;
            cfg.cursor_closed = work.cursor_closed;
            if !events.is_empty() {
                record_entities(&mut cfg, &events);
            }
            // 无事件时游标仍可能变化，照常落盘（增量小，可接受）
            store::save(&state.path, &cfg);
        }
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
        if !toasting.is_empty() {
            spawn_qq_push(&state.config.lock().unwrap().clone(), &toasting);
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

// 免疫 Win+D 的兜底：200ms 体检唤回，另每 1s 无条件把窗口提回桌面层 Z 序顶部
// （Win+D 未必改 WS_VISIBLE，只看可见性会漏判）。唤回无效则指数退避到 2s，
// 避免与 Shell 拉锯——高频 SetWindowPos 会打断鼠标按下/抬起配对。
fn spawn_visibility_guard(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut delay = 200u64;
        let mut last_raise = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            let Some(w) = app.get_webview_window("main") else {
                continue;
            };
            let raise_z = last_raise.elapsed() >= std::time::Duration::from_secs(1);
            if raise_z {
                last_raise = std::time::Instant::now();
            }
            let healthy = winembed::guard_tick(&w, raise_z);
            delay = if healthy { 200 } else { (delay * 2).min(2000) };
        }
    });
}

fn spawn_poller(app: tauri::AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        println!("[poll] 轮循协程已启动");
        loop {
            // 每轮重新读设置，改频率无需重启应用；下限 10s 防 API 限流
            let secs = state
                .config
                .lock()
                .unwrap()
                .poll_interval
                .unwrap_or(60)
                .clamp(10, 3600);
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
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
            for e in cfg.entities.iter_mut() {
                if e.repo.eq_ignore_ascii_case(&repo) && e.number == Some(n) {
                    e.status = "read".into();
                }
            }
        });
        state.notified.lock().unwrap().remove(&format!("{repo}#{n}"));
    }
}

// 卡片状态机：未读 / 已读 / 处理中 / 已完成（默认未读）
const ENTITY_STATUSES: [&str; 4] = ["unread", "read", "in_progress", "done"];

// 实体列表统一按最近更新时间降序，主界面直接消费
fn sorted_entities(state: &AppState) -> Vec<Entity> {
    let mut v = state.config.lock().unwrap().entities.clone();
    v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)); // 最近变更在上
    v
}

// 启动竞态：首轮轮询、窗口位置落盘都可能短暂持有 config 锁。
// 原先 try_lock 一撞就返回 Err，前端会立刻把整张列表抹空，而 ghr-events 又不一定会来；
// config 的临界区都很短（内存操作或一次小文件写入），阻塞等几毫秒不会死锁。
#[tauri::command]
fn get_events(state: tauri::State<Arc<AppState>>) -> Vec<Entity> {
    sorted_entities(state.inner().as_ref())
}

// 右键菜单改状态：按 repo+number 定位聚合实体（同一 issue/PR 只有一张卡）
#[tauri::command]
fn set_status(
    state: tauri::State<Arc<AppState>>,
    repo: String,
    number: Option<u64>,
    status: String,
) -> Result<Vec<Entity>, String> {
    if !ENTITY_STATUSES.contains(&status.as_str()) {
        return Err(format!("未知状态: {status}"));
    }
    state.with_config(|cfg| {
        for e in cfg.entities.iter_mut() {
            if e.repo.eq_ignore_ascii_case(&repo) && e.number == number {
                e.status = status.clone();
            }
        }
    });
    // 人工改过状态即允许该条目再次弹通知，与 mark_read 的去重口径一致
    if let Some(n) = number {
        state.notified.lock().unwrap().remove(&format!("{repo}#{n}"));
    }
    Ok(sorted_entities(&state))
}

// 批量改状态：标记时 statuses 全填同一个值，撤销时逐条填快照里的原状态。
// 抽成纯函数是为了不依赖 AppState 也能单测参数校验与定位逻辑。
fn apply_statuses(
    cfg: &mut Config,
    repos: &[String],
    numbers: &[Option<u64>],
    statuses: &[String],
) -> Result<(), String> {
    if repos.len() != numbers.len() || repos.len() != statuses.len() {
        return Err("批量参数长度不一致".into());
    }
    if statuses.iter().any(|s| !ENTITY_STATUSES.contains(&s.as_str())) {
        return Err("批量状态里有未知状态".into());
    }
    for i in 0..repos.len() {
        for e in cfg.entities.iter_mut() {
            if e.repo.eq_ignore_ascii_case(&repos[i]) && e.number == numbers[i] {
                e.status = statuses[i].clone();
            }
        }
    }
    Ok(())
}

// 一次落盘改多张卡，避免前端循环 invoke 导致 N 次重绘和 N 次写盘
#[tauri::command]
fn set_status_batch(
    state: tauri::State<Arc<AppState>>,
    repos: Vec<String>,
    numbers: Vec<Option<u64>>,
    statuses: Vec<String>,
) -> Result<Vec<Entity>, String> {
    state.with_config(|cfg| apply_statuses(cfg, &repos, &numbers, &statuses))?;
    for (repo, number) in repos.iter().zip(numbers.iter()) {
        if let Some(n) = number {
            state.notified.lock().unwrap().remove(&format!("{repo}#{n}"));
        }
    }
    Ok(sorted_entities(&state))
}

// 删除卡片：只删通知记录，不动监听配置；被删条目不会再出现（除非有新动态）
#[tauri::command]
fn delete_event(
    state: tauri::State<Arc<AppState>>,
    repo: String,
    number: Option<u64>,
) -> Vec<Entity> {
    state.with_config(|cfg| {
        cfg.entities
            .retain(|e| !(e.repo.eq_ignore_ascii_case(&repo) && e.number == number));
    });
    if let Some(n) = number {
        state.notified.lock().unwrap().remove(&format!("{repo}#{n}"));
    }
    sorted_entities(&state)
}

// 置顶（钉住）上限：主界面顶部最多横排 3 张
const MAX_PINNED: usize = 3;

// 右键菜单置顶/取消置顶；置顶数已满时拒绝，由前端提示
#[tauri::command]
fn set_pin(
    state: tauri::State<Arc<AppState>>,
    repo: String,
    number: Option<u64>,
    pinned: bool,
) -> Result<Vec<Entity>, String> {
    let mut err: Option<String> = None;
    state.with_config(|cfg| {
        let Some(idx) = cfg
            .entities
            .iter()
            .position(|e| e.repo.eq_ignore_ascii_case(&repo) && e.number == number)
        else {
            err = Some("通知不存在".into());
            return;
        };
        if pinned && !cfg.entities[idx].pinned {
            let count = cfg.entities.iter().filter(|e| e.pinned).count();
            if count >= MAX_PINNED {
                err = Some(format!("最多钉 {MAX_PINNED} 个，请先取消一个"));
                return;
            }
        }
        cfg.entities[idx].pinned = pinned;
    });
    match err {
        Some(e) => Err(e),
        None => Ok(sorted_entities(&state)),
    }
}

#[tauri::command]
fn get_stats(
    state: tauri::State<Arc<AppState>>,
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>> {
    state.config.lock().unwrap().stats.clone()
}

// ---- 项目外观 / bot 关键词 / 统计窗口 -------------------------------------
// 已知项目 = 监听过的仓库 + 现有通知 + 统计流水；缺样式的在读取时补齐并落盘，
// 保证饼图颜色和设置页列表永远对得上。
fn known_repos(cfg: &Config) -> Vec<String> {
    let mut repos: Vec<String> = cfg.watches.iter().map(|w| w.repo.clone()).collect();
    repos.extend(cfg.entities.iter().map(|e| e.repo.clone()));
    repos.extend(cfg.stat_events.iter().map(|s| s.repo.clone()));
    repos
}

#[tauri::command]
fn get_project_styles(
    state: tauri::State<Arc<AppState>>,
) -> std::collections::BTreeMap<String, store::ProjectStyle> {
    state.with_config(|cfg| {
        let repos = known_repos(cfg);
        store::ensure_project_styles(cfg, &repos);
    });
    state.config.lock().unwrap().project_styles.clone()
}

#[tauri::command]
fn set_project_style(
    state: tauri::State<Arc<AppState>>,
    repo: String,
    color: Option<String>,
    text_color: Option<String>,
) -> std::collections::BTreeMap<String, store::ProjectStyle> {
    let repo = repo.trim().to_string();
    if repo.is_empty() {
        return state.config.lock().unwrap().project_styles.clone();
    }
    state.with_config(|cfg| {
        store::ensure_project_styles(cfg, std::slice::from_ref(&repo));
        if let Some(entry) = cfg.project_styles.get_mut(&repo) {
            if let Some(c) = color.filter(|v| !v.trim().is_empty()) {
                entry.color = c;
            }
            if let Some(t) = text_color.filter(|v| !v.trim().is_empty()) {
                entry.text_color = t;
            }
        }
    });
    state.config.lock().unwrap().project_styles.clone()
}

#[tauri::command]
fn get_bot_keywords(state: tauri::State<Arc<AppState>>) -> Vec<String> {
    state.config.lock().unwrap().bot_keywords.clone()
}

#[tauri::command]
fn set_bot_keywords(state: tauri::State<Arc<AppState>>, keywords: Vec<String>) -> Vec<String> {
    state.with_config(|cfg| {
        let mut seen = HashSet::new();
        cfg.bot_keywords = keywords
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && seen.insert(s.to_lowercase()))
            .collect();
    });
    state.config.lock().unwrap().bot_keywords.clone()
}

#[tauri::command]
fn get_stats_ranges(state: tauri::State<Arc<AppState>>) -> stats::RangeCount {
    let cfg = state.config.lock().unwrap().clone();
    stats::snapshot(&cfg.stat_events, &cfg.entities, chrono::Utc::now().timestamp())
}

// 添加监听时的浏览弹窗：列出仓库下的 Issue/PR 供勾选（单条监听默认开评论+状态变更）
#[tauri::command]
async fn list_repo_items(
    state: tauri::State<'_, Arc<AppState>>,
    repo: String,
    item_state: String,
) -> Result<Vec<github::RepoItem>, String> {
    let repo = repo.trim().to_string();
    validate_repo(&repo)?;
    if !matches!(item_state.as_str(), "open" | "closed") {
        return Err(format!("未知状态筛选: {item_state}"));
    }
    let cfg = state.inner().config.lock().unwrap().clone();
    github::list_repo_items(&cfg, &repo, &item_state).await
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
            // token 唯一来源是凭据管理器；不迁移 config.json 里的旧明文（主人明确不要）
            cfg.token = store::load_token();
            // QQ AppSecret 同样改存凭据管理器：旧 config.json 里的明文一次性迁入并剔除，
            // 避免升级后内置中转因缺密钥静默不启动。写失败时本次仅内存保留、不删明文。
            let mut stored_secret = store::load_qq_app_secret();
            if let Some(plain) = cfg.qq_app_secret.clone() {
                if stored_secret.is_none() {
                    match store::save_qq_app_secret(Some(&plain)) {
                        Ok(()) => stored_secret = Some(plain),
                        Err(e) => eprintln!("[init] QQ AppSecret 迁移失败，本次仅内存保留: {e}"),
                    }
                }
                if stored_secret.is_some() {
                    store::save(&path, &cfg); // save 落盘前会剔除明文
                    println!("[init] QQ AppSecret 已存凭据管理器，config.json 明文已剔除");
                }
            }
            if let Some(s) = stored_secret {
                cfg.qq_app_secret = Some(s);
            }
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
            // 持久化可观测点：重启后这里应报出重启前的实体条数
            let cnt = |s: &str| cfg.entities.iter().filter(|e| e.status == s).count();
            println!(
                "[init] 通知实体: {} 条（未读 {} / 已读 {} / 处理中 {} / 已完成 {}，置顶 {}）",
                cfg.entities.len(),
                cnt("unread"),
                cnt("read"),
                cnt("in_progress"),
                cnt("done"),
                cfg.entities.iter().filter(|e| e.pinned).count()
            );
            // 恢复上次的位置/尺寸（物理像素，覆盖 tauri.conf 的 center）；坏存档忽略并自愈清掉
            if let Some(w) = app.get_webview_window("main") {
                // 配置里的 minWidth/minHeight 只约束用户拖拽，程序化 set_size 不走那条限制
                let _ = w.set_min_size(Some(tauri::LogicalSize::new(
                    MIN_WINDOW_SIZE,
                    MIN_WINDOW_SIZE,
                )));
                if let Some(rect) = cfg.window {
                    let min_px = MIN_WINDOW_SIZE * w.scale_factor().unwrap_or(1.0);
                    if rect_is_plausible(rect.x, rect.y, rect.width, rect.height, min_px) {
                        let _ = w.set_size(tauri::PhysicalSize::new(
                            rect.width.round() as u32,
                            rect.height.round() as u32,
                        ));
                        let _ = w.set_position(tauri::PhysicalPosition::new(
                            rect.x.round() as i32,
                            rect.y.round() as i32,
                        ));
                    } else {
                        println!("[init] 窗口矩形离屏或过小，已忽略并按默认居中显示");
                        cfg.window = None;
                        store::save(&path, &cfg);
                    }
                }
                // 先恢复普通矩形再翻最大化，保证取消最大化后能回到上次的位置
                if cfg.window.map(|r| r.maximized).unwrap_or(false) {
                    let _ = w.maximize();
                }
            }
            let state = Arc::new(AppState {
                path,
                config: Mutex::new(cfg.clone()),
                last_poll: Mutex::new(None),
                polling: AtomicBool::new(false),
                notified: Mutex::new(HashSet::new()),
                relay: Mutex::new(None),
                last_win_save: Mutex::new(0),
            });
            // 自动配置开着且配置齐全的话，启动内置 QQ 中转服务
            restart_relay(&state);
            spawn_poller(app.handle().clone(), state.clone());
            // 打开 app 先自动轮询一次，不必等第一个轮询周期到点；
            // 稍等 1s 让主窗口挂好事件监听，首轮结果才不会漏给前端
            {
                let handle = app.handle().clone();
                let st = state.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    do_poll(&handle, &st).await;
                });
            }
            spawn_visibility_guard(app.handle().clone());
            setup_tray(app)?;
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            // 主窗口移动/拉伸即记录位置，下次启动恢复
            if window.label() == "main"
                && matches!(
                    event,
                    tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
                )
            {
                let state = window.app_handle().state::<Arc<AppState>>();
                remember_window(&state, window);
            }
            // 两个窗口点关闭都只隐藏：主窗口藏到托盘，管理窗口藏起来以便再次唤起
            if matches!(window.label(), "main" | "manage")
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                if window.label() == "main" {
                    let state = window.app_handle().state::<Arc<AppState>>();
                    // 清零节流计时，强制把最终位置落盘
                    *state.last_win_save.lock().unwrap() = 0;
                    remember_window(&state, window);
                    winembed::USER_HIDDEN.store(true, Ordering::Relaxed);
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            winembed::set_window_mode,
            winembed::user_minimize,
            get_config,
            set_token,
            set_proxy,
            set_notify,
            backfill_14d,
            set_poll_interval,
            set_github_username,
            set_qq_notify,
            set_qq_settings,
            set_qq_autoconfig,
            set_autostart,
            get_autostart,
            add_watch,
            update_watch,
            remove_watch,
            open_manage,
            poll_now,
            mark_read,
            get_events,
            set_status,
            set_status_batch,
            delete_event,
            set_pin,
            get_stats,
            get_stats_ranges,
            get_project_styles,
            set_project_style,
            get_bot_keywords,
            set_bot_keywords,
            list_repo_items
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn rejects_minimized_geometry() {
        assert!(!rect_is_plausible(-32000.0, -32000.0, 199.0, 34.0, 240.0));
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> store::WindowRect {
        store::WindowRect { x, y, width: w, height: h, maximized: false }
    }

    #[test]
    fn maximized_keeps_prev_rect_and_sets_flag() {
        let prev = rect(100.0, 80.0, 480.0, 320.0);
        let out = next_window_rect(Some(prev), rect(0.0, 0.0, 1920.0, 1080.0), true).unwrap();
        assert_eq!(out.x, 100.0);
        assert_eq!(out.width, 480.0);
        assert!(out.maximized);
    }

    #[test]
    fn maximized_without_prev_stores_nothing() {
        assert!(next_window_rect(None, rect(0.0, 0.0, 1920.0, 1080.0), true).is_none());
    }

    #[test]
    fn normal_rect_clears_flag() {
        let mut prev = rect(100.0, 80.0, 480.0, 320.0);
        prev.maximized = true;
        let out = next_window_rect(Some(prev), rect(120.0, 90.0, 500.0, 340.0), false).unwrap();
        assert!(!out.maximized);
        assert_eq!(out.x, 120.0);
    }

    #[test]
    fn rejects_window_smaller_than_min() {
        assert!(!rect_is_plausible(100.0, 100.0, 239.0, 320.0, 240.0));
        assert!(!rect_is_plausible(100.0, 100.0, 480.0, 200.0, 240.0));
    }

    #[test]
    fn accepts_normal_and_negative_monitor_coords() {
        assert!(rect_is_plausible(100.0, 100.0, 480.0, 320.0, 240.0));
        assert!(rect_is_plausible(-1920.0, 0.0, 480.0, 320.0, 240.0));
    }
}

#[cfg(test)]
mod qq_tests {
    use super::*;

    fn ev(kind: &str, num: Option<u64>, path: &str) -> github::Event {
        github::Event {
            kind: kind.into(),
            repo: "octocat/hello-world".into(),
            number: num,
            title: "fix: 示例提交标题".into(),
            url: format!("https://github.com/octocat/hello-world/{path}"),
            created_at: "2026-11-01T00:00:00+08:00".into(),
            author: "octocat".into(),
            assignees: vec!["octocat".into()],
        }
    }

    #[test]
    fn qq_summary_uses_card_style_four_line_block() {
        let events = vec![ev("pr", Some(459), "pull/459")];
        let text = format!(
            "[GitHubReminder] 通知 · GitHub 动态\n{}",
            qq_summary(&events, Some("octocat"))
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "[GitHubReminder] 通知 · GitHub 动态");
        assert_eq!(lines[1], "新 PR #459 · octocat/hello-world");
        assert_eq!(lines[2], "[@me] [octocat] · fix: 示例提交标题");
        assert_eq!(lines[3], "https://github.com/octocat/hello-world/pull/459");
    }

    #[test]
    fn qq_summary_hides_at_me_when_not_assignee() {
        let mut e = ev("issue", Some(461), "issues/461");
        e.assignees = vec!["other".into()];
        let text = qq_summary(&[e], Some("octocat"));
        assert!(!text.contains("[@me]"), "不在 assignee 列表里就不该出现 @me：{text}");
        assert!(text.contains("[octocat] · "), "提 issue 的人仍在：{text}");
    }

    #[test]
    fn qq_summary_no_at_me_when_username_not_configured() {
        let events = vec![ev("issue", Some(461), "issues/461")];
        let text = qq_summary(&events, None);
        assert!(!text.contains("[@me]"), "未填用户名时不应判断：{text}");
        assert!(text.contains("[octocat] · "), "但用户名标记仍应显示：{text}");
    }

    #[test]
    fn qq_summary_comment_shows_generic_message() {
        let events = vec![ev("comment", Some(461), "issues/461")];
        let text = qq_summary(&events, None);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Issue 新评论 #461 · octocat/hello-world");
        assert!(lines[1].ends_with("· 有新消息"), "评论标题统一「有新消息」：{}", lines[1]);
    }

    #[test]
    fn qq_summary_release_has_no_number() {
        let mut rel = ev("release", None, "releases/tag/v1");
        rel.title = "v1.0".into();
        let text = qq_summary(&[rel], None);
        assert!(text.starts_with("新 Release · octocat/hello-world\n"));
        assert!(text.contains("[octocat] · v1.0"), "发布者应显示在标题行：{text}");
    }

    #[test]
    fn qq_summary_emits_all_events_without_folding() {
        let events: Vec<github::Event> = (1..=12)
            .map(|n| ev("issue", Some(n), &format!("issues/{n}")))
            .collect();
        let text = qq_summary(&events, None);
        for n in 1..=12 {
            assert!(
                text.contains(&format!("新 Issue #{n} · ")),
                "第 {n} 条不应被折叠掉"
            );
        }
        assert!(
            !text.contains("以及另外"),
            "不应再有折叠提示：{text}"
        );
    }

    #[test]
    fn qq_summary_head_line_marks_event_action() {
        // 主人 2026-11-07 要求：头一行除类型外还要标事件动作
        let cases: Vec<(&str, &str, &str)> = vec![
            ("closed_pr", "pull/9", "已关闭 PR #9 · octocat/hello-world"),
            ("closed_issue", "issues/9", "已关闭 Issue #9 · octocat/hello-world"),
            ("state", "pull/9", "PR 状态变更 #9 · octocat/hello-world"),
            ("state", "issues/9", "Issue 状态变更 #9 · octocat/hello-world"),
            ("comment", "pull/9", "PR 新评论 #9 · octocat/hello-world"),
        ];
        for (kind, path, expected) in cases {
            let text = qq_summary(&[ev(kind, Some(9), path)], None);
            assert!(
                text.lines().next() == Some(expected),
                "kind={kind} 头一行应为「{expected}」：{text}"
            );
        }
    }
}

#[cfg(test)]
mod auto_done_tests {
    use super::*;

    // fixture 一律通用示例：octocat/hello-world、#123、示例标题
    fn ev(kind: &str, author: &str, path: &str, title: &str) -> github::Event {
        github::Event {
            kind: kind.into(),
            repo: "octocat/hello-world".into(),
            number: Some(123),
            title: title.into(),
            url: format!("https://github.com/octocat/hello-world/{path}"),
            created_at: "2026-11-07T00:00:00+08:00".into(),
            author: author.into(),
            assignees: Vec::new(),
        }
    }

    fn closed_pr(author: &str) -> github::Event {
        ev(
            "closed_pr",
            author,
            "pull/123",
            "chore(deps): bump example from 1.0 to 1.1",
        )
    }

    #[test]
    fn recognizes_dependabot_bot_accounts_only() {
        for a in [
            "dependabot[bot]",
            "DEPENDABOT[BOT]",
            " dependabot[bot] ",
            "dependabot",
            "dependabot-preview[bot]",
            "dependabot-bot",
        ] {
            assert!(is_dependabot_author(a), "{a} 应识别为 dependabot");
        }
        for a in ["octocat", "dependabot-fan", "my-dependabot", "dependabotish", ""] {
            assert!(!is_dependabot_author(a), "{a} 不该误判为 dependabot");
        }
    }

    #[test]
    fn rule_fires_only_for_closed_pr_by_dependabot() {
        assert!(!auto_done_on_close(
            &ev("pr", "dependabot[bot]", "pull/123", "示例标题"),
            "open"
        ));
        assert!(!auto_done_on_close(
            &ev("closed_pr", "octocat", "pull/123", "示例标题"),
            "closed"
        ));
        assert!(!auto_done_on_close(
            &ev("closed_issue", "dependabot[bot]", "issues/123", "示例标题"),
            "closed"
        ));
        assert!(auto_done_on_close(&closed_pr("dependabot[bot]"), "closed"));
    }

    #[test]
    fn closed_dependabot_pr_lands_in_done() {
        let mut cfg = Config::default();
        record_entities(&mut cfg, &[closed_pr("dependabot[bot]")]);
        assert_eq!(cfg.entities.len(), 1);
        assert_eq!(cfg.entities[0].state, "closed");
        assert_eq!(cfg.entities[0].status, "done");
    }

    #[test]
    fn state_change_to_closed_also_lands_in_done() {
        // 单条监听走 state 事件，且实体可能已被主人手动标成处理中
        let mut cfg = Config::default();
        record_entities(&mut cfg, &[ev("pr", "dependabot[bot]", "pull/123", "示例标题")]);
        cfg.entities[0].status = "in_progress".into();
        let mut state_ev = ev(
            "state",
            "dependabot[bot]",
            "pull/123",
            "「示例标题」状态 open → closed",
        );
        state_ev.created_at = "2026-11-07T00:05:00+08:00".into();
        record_entities(&mut cfg, &[state_ev]);
        assert_eq!(cfg.entities.len(), 1);
        assert_eq!(cfg.entities[0].state, "closed");
        assert_eq!(cfg.entities[0].status, "done");
    }

    #[test]
    fn other_authors_closed_pr_stays_unread() {
        let mut cfg = Config::default();
        record_entities(&mut cfg, &[closed_pr("octocat")]);
        assert_eq!(cfg.entities[0].status, "unread");
    }

    #[test]
    fn dependabot_issue_close_is_not_affected() {
        let mut cfg = Config::default();
        record_entities(
            &mut cfg,
            &[ev("closed_issue", "dependabot[bot]", "issues/123", "示例标题")],
        );
        assert_eq!(cfg.entities[0].status, "unread");
    }

    #[test]
    fn comment_in_same_batch_cannot_undo_auto_done() {
        let mut cfg = Config::default();
        let mut comment = ev("comment", "dependabot[bot]", "pull/123", "示例评论");
        comment.created_at = "2026-11-07T00:06:00+08:00".into();
        record_entities(&mut cfg, &[closed_pr("dependabot[bot]"), comment]);
        assert_eq!(cfg.entities[0].status, "done");
    }
}

#[cfg(test)]
mod search_field_tests {
    use super::*;

    // fixture 一律通用示例：octocat / hubot / example.com / #123 / 示例标题
    fn ev(author: &str, assignees: &[&str]) -> github::Event {
        github::Event {
            kind: "issue".into(),
            repo: "octocat/hello-world".into(),
            number: Some(123),
            title: "示例标题".into(),
            url: "https://example.com/octocat/hello-world/issues/123".into(),
            created_at: "2026-11-07T00:00:00+08:00".into(),
            author: author.into(),
            assignees: assignees.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn new_entity_keeps_author_and_involved_people() {
        let mut cfg = Config::default();
        record_entities(&mut cfg, &[ev("Octocat", &["Hubot", "octocat"])]);
        let e = &cfg.entities[0];
        assert_eq!(e.author, "Octocat");
        assert_eq!(e.people, vec!["octocat", "hubot"]); // 统一小写、去重
    }

    #[test]
    fn later_events_keep_original_author_but_accumulate_people() {
        let mut cfg = Config::default();
        record_entities(&mut cfg, &[ev("octocat", &[])]);
        let mut later = ev("hubot", &["alice"]);
        later.created_at = "2026-11-08T00:00:00+08:00".into();
        record_entities(&mut cfg, &[later]);
        assert_eq!(cfg.entities.len(), 1);
        assert_eq!(cfg.entities[0].author, "octocat", "卡片作者不该被后续事件覆盖");
        assert_eq!(cfg.entities[0].people, vec!["octocat", "hubot", "alice"]);
    }

    #[test]
    fn legacy_entity_without_people_is_backfilled_on_next_event() {
        // 旧版存档没有 author / people，serde 的 default 必须给空值而不是整体解析失败
        let json = r#"{"kind":"issue","repo":"octocat/hello-world","number":123,"title":"示例标题","url":"https://example.com/o/r/issues/123","state":"open","lastEventKind":"open","updatedAt":"2026-11-01T00:00:00Z","status":"read","pinned":false}"#;
        let old: Entity = serde_json::from_str(json).expect("旧存档应能解析");
        assert!(old.author.is_empty() && old.people.is_empty());
        let mut cfg = Config::default();
        cfg.entities.push(old);
        let mut later = ev("octocat", &["hubot"]);
        later.created_at = "2026-11-08T00:00:00+08:00".into();
        record_entities(&mut cfg, &[later]);
        assert_eq!(cfg.entities.len(), 1);
        assert_eq!(cfg.entities[0].author, "octocat");
        assert_eq!(cfg.entities[0].people, vec!["octocat", "hubot"]);
    }
}

#[cfg(test)]
mod batch_status_tests {
    use super::*;

    fn two_entities() -> Config {
        let mut cfg = Config::default();
        cfg.entities.push(Entity {
            kind: "pr".into(),
            repo: "owner/repo".into(),
            number: Some(1),
            status: "unread".into(),
            ..Entity::default()
        });
        cfg.entities.push(Entity {
            kind: "issue".into(),
            repo: "owner/repo".into(),
            number: Some(2),
            status: "done".into(),
            ..Entity::default()
        });
        cfg
    }

    #[test]
    fn batch_sets_selected_to_one_status() {
        let mut cfg = two_entities();
        let repos = vec!["owner/repo".to_string(), "owner/repo".to_string()];
        let numbers = vec![Some(1), Some(2)];
        let statuses = vec!["in_progress".to_string(), "in_progress".to_string()];
        apply_statuses(&mut cfg, &repos, &numbers, &statuses).unwrap();
        assert_eq!(cfg.entities[0].status, "in_progress");
        assert_eq!(cfg.entities[1].status, "in_progress");
    }

    #[test]
    fn batch_restores_mixed_statuses_for_undo() {
        let mut cfg = two_entities();
        let repos = vec!["owner/repo".to_string(), "owner/repo".to_string()];
        let numbers = vec![Some(1), Some(2)];
        let snapshot = vec!["unread".to_string(), "done".to_string()];
        apply_statuses(&mut cfg, &repos, &numbers, &snapshot).unwrap();
        assert_eq!(cfg.entities[0].status, "unread");
        assert_eq!(cfg.entities[1].status, "done");
    }

    #[test]
    fn batch_rejects_mismatched_lengths_and_unknown_status() {
        let mut cfg = two_entities();
        let repos = vec!["owner/repo".to_string()];
        assert!(apply_statuses(&mut cfg, &repos, &[], &[]).is_err());
        assert!(
            apply_statuses(&mut cfg, &repos, &[Some(1)], &["bogus".to_string()]).is_err()
        );
        assert_eq!(cfg.entities[0].status, "unread", "校验失败不该改到任何卡片");
    }

    #[test]
    fn batch_matches_repo_case_insensitively() {
        let mut cfg = two_entities();
        apply_statuses(
            &mut cfg,
            &["OWNER/REPO".to_string()],
            &[Some(2)],
            &["read".to_string()],
        )
        .unwrap();
        assert_eq!(cfg.entities[0].status, "unread");
        assert_eq!(cfg.entities[1].status, "read");
    }
}
