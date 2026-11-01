mod github;
mod qqrelay;
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
) -> Config {
    let trim = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    state.with_config(|cfg| {
        cfg.qq_relay_port = port;
        cfg.qq_app_id = trim(app_id);
        cfg.qq_app_secret = trim(app_secret);
        cfg.qq_openid = trim(openid);
    });
    restart_relay(&state);
    state.config.lock().unwrap().clone()
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
//   PR #459 · owner/repo
//   标题（评论统一显示「有新消息」）
//   https://github.com/owner/repo/pull/459
fn event_kind_label(ev: &github::Event) -> &'static str {
    match ev.kind.as_str() {
        "release" => "Release",
        "pr" | "closed_pr" => "PR",
        "issue" | "closed_issue" => "Issue",
        // comment/state 按 URL 区分挂在 Issue 还是 PR 上
        _ if ev.url.contains("/pull/") => "PR",
        _ => "Issue",
    }
}

fn qq_summary(events: &[github::Event]) -> String {
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
        lines.push(head);
        lines.push(title);
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
        qq_summary(events)
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
// 关闭隐藏前把节流计时清零再调一次，保证最终位置一定落盘
fn remember_window(state: &AppState, window: &tauri::Window) {
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return;
    };
    let rect = store::WindowRect {
        x: pos.x as f64,
        y: pos.y as f64,
        width: size.width as f64,
        height: size.height as f64,
    };
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
    cfg.window = Some(rect);
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

const MAX_ENTITIES: usize = 200;

// 把变化事件 upsert 进实体表：同一 repo+number 聚合为一卡，updated_at 取最新；
// 有新动态（时间更新或状态/类型变化）则 status 重置为未读。同时累计 stats。
fn record_entities(cfg: &mut Config, events: &[github::Event]) {
    for ev in events {
        let (kind, last_kind, state) = classify(ev);
        if let Some(e) = cfg.entities.iter_mut().find(|e| {
            e.repo.eq_ignore_ascii_case(&ev.repo) && e.number == ev.number
        }) {
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
            });
        }
        let bump = |m: &mut std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>>, key: String| {
            *m.entry(key).or_default().entry(ev.kind.clone()).or_insert(0) += 1;
        };
        bump(&mut cfg.stats, ev.repo.clone());
        if let Some(n) = ev.number {
            bump(&mut cfg.stats, format!("{}#{n}", ev.repo));
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

// Result 而非裸 Vec：失败要落到后端日志——前端 flash 报错而后台毫无痕迹时没法排查
#[tauri::command]
fn get_events(state: tauri::State<Arc<AppState>>) -> Result<Vec<Entity>, String> {
    let mut v = match state.config.try_lock() {
        Ok(cfg) => cfg.entities.clone(),
        Err(e) => {
            // 中毒/被持有都要在后端留下痕迹，前端只能拿到这串字符串
            let msg = format!("实体列表不可读（锁状态: {e:?}）");
            eprintln!("[events] get_events 失败: {msg}");
            return Err(msg);
        }
    };
    v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)); // 最近变更在上
    Ok(v)
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
            let state = Arc::new(AppState {
                path,
                config: Mutex::new(cfg.clone()),
                last_poll: Mutex::new(None),
                polling: AtomicBool::new(false),
                notified: Mutex::new(HashSet::new()),
                relay: Mutex::new(None),
                last_win_save: Mutex::new(0),
            });
            // 恢复上次的位置/尺寸（物理像素，直接设回；覆盖 tauri.conf 的 center）
            if let (Some(rect), Some(w)) = (cfg.window, app.get_webview_window("main")) {
                let _ = w.set_size(tauri::PhysicalSize::new(
                    rect.width.round() as u32,
                    rect.height.round() as u32,
                ));
                let _ = w.set_position(tauri::PhysicalPosition::new(
                    rect.x.round() as i32,
                    rect.y.round() as i32,
                ));
            }
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
            delete_event,
            set_pin,
            get_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod qq_tests {
    use super::*;

    fn ev(kind: &str, num: Option<u64>, path: &str) -> github::Event {
        github::Event {
            kind: kind.into(),
            repo: "ytd9527/NetworksExpansion".into(),
            number: num,
            title: "fix: 修改元件转运台逻辑".into(),
            url: format!("https://github.com/ytd9527/NetworksExpansion/{path}"),
            created_at: "2026-11-01T00:00:00+08:00".into(),
            author: "ytd9527".into(),
            assignees: vec!["ytd9527".into()],
        }
    }

    #[test]
    fn qq_summary_uses_card_style_four_line_block() {
        let events = vec![ev("pr", Some(459), "pull/459")];
        let text = format!("[GitHubReminder] 通知 · GitHub 动态\n{}", qq_summary(&events));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "[GitHubReminder] 通知 · GitHub 动态");
        assert_eq!(lines[1], "PR #459 · ytd9527/NetworksExpansion");
        assert_eq!(lines[2], "fix: 修改元件转运台逻辑");
        assert_eq!(lines[3], "https://github.com/ytd9527/NetworksExpansion/pull/459");
    }

    #[test]
    fn qq_summary_comment_shows_generic_message() {
        let events = vec![ev("comment", Some(461), "issues/461")];
        let text = qq_summary(&events);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Issue #461 · ytd9527/NetworksExpansion");
        assert_eq!(lines[1], "有新消息");
    }

    #[test]
    fn qq_summary_release_has_no_number() {
        let mut rel = ev("release", None, "releases/tag/v1");
        rel.title = "v1.0".into();
        let text = qq_summary(&[rel]);
        assert!(text.starts_with("Release · ytd9527/NetworksExpansion\nv1.0\n"));
    }

    #[test]
    fn qq_summary_emits_all_events_without_folding() {
        let events: Vec<github::Event> = (1..=12)
            .map(|n| ev("issue", Some(n), &format!("issues/{n}")))
            .collect();
        let text = qq_summary(&events);
        for n in 1..=12 {
            assert!(
                text.contains(&format!("Issue #{n} · ")),
                "第 {n} 条不应被折叠掉"
            );
        }
        assert!(
            !text.contains("以及另外"),
            "不应再有折叠提示：{text}"
        );
    }
}
