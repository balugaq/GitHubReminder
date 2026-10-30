use serde::Deserialize;
use std::net::ToSocketAddrs;
use crate::store::{Config, Watch};

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub kind: String, // "issue" | "pr" | "release"
    pub repo: String,
    pub number: Option<u64>,
    pub title: String,
    pub url: String,
    pub created_at: String,
}

#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    html_url: String,
    created_at: String,
    pull_request: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Release {
    id: u64,
    name: Option<String>,
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
}

fn client(cfg: &Config) -> Result<reqwest::Client, String> {    let mut b = reqwest::Client::builder()
        .user_agent("github-reminder/0.1")
        // API 请求超时别挂死整个轮询周期
        .timeout(std::time::Duration::from_secs(20));
    if let Some(p) = &cfg.proxy {
        let p = reqwest::Proxy::all(p).map_err(|e| format!("代理地址无效: {e}"))?;
        b = b.proxy(p);
    }
    if let Some(t) = &cfg.token {
        b = b.default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {t}")) {
                h.insert(reqwest::header::AUTHORIZATION, v);
            }
            h
        });
    }
    // 强制 IPv4：Node undici 有 Happy Eyeballs 能从 v6 自动回落 v4，reqwest 没有；
    // 国内环境 DNS 常返回 v6 但路由不通，v6 会卡死连接
    if let Ok(addrs) = ("api.github.com", 443).to_socket_addrs() {
        let v4: Vec<std::net::SocketAddr> = addrs.filter(|a| a.is_ipv4()).collect();
        if !v4.is_empty() {
            b = b.resolve_to_addrs("api.github.com", &v4);
        }
    }
    b.build().map_err(|e| format!("构建 HTTP 客户端失败: {e}"))
}

pub fn now_str() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

pub async fn poll_all(cfg: &mut Config) -> (Vec<Event>, Option<String>) {
    let t0 = std::time::Instant::now();
    let n_watches = cfg.watches.len();
    let (events, err) = poll_all_inner(cfg).await;
    match &err {
        Some(e) => println!(
            "[poll {}] 轮循完成: {n_watches} 个监听, {} 事件, 用时 {:?}, 失败: {e}",
            now_str(),
            events.len(),
            t0.elapsed()
        ),
        None => println!(
            "[poll {}] 轮循完成: {n_watches} 个监听, {} 事件, 用时 {:?}",
            now_str(),
            events.len(),
            t0.elapsed()
        ),
    }
    (events, err)
}

async fn poll_all_inner(cfg: &mut Config) -> (Vec<Event>, Option<String>) {
    let client = match client(cfg) {
        Ok(c) => c,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let mut events = Vec::new();
    let mut first_err: Option<String> = None;
    for w in cfg.watches.clone() {
        if w.number.is_some() {
            match poll_item(&client, &w, cfg).await {
                Ok(mut evs) => {
                    println!(
                        "[poll {}] {}#{:?} item: 拉取成功, 新事件 {} 条",
                        now_str(),
                        w.repo,
                        w.number,
                        evs.len()
                    );
                    events.append(&mut evs);
                }
                Err(e) => {
                    let msg = format!("{}#{:?}: {e}", w.repo, w.number);
                    eprintln!("[poll {}] {}#{:?} item 拉取失败", now_str(), w.repo, w.number);
                    if first_err.is_none() {
                        first_err = Some(msg);
                    }
                }
            }
            continue;
        }
        if w.issues || w.prs {
            match poll_issues(&client, &w, cfg).await {
                Ok(mut evs) => {
                    println!(
                        "[poll {}] {} issues/prs: 拉取成功, 新事件 {} 条",
                        now_str(),
                        w.repo,
                        evs.len()
                    );
                    events.append(&mut evs);
                }
                Err(e) => {
                    let msg = format!("{}: {e}", w.repo);
                    eprintln!("[poll {}] {} 拉取失败", now_str(), w.repo);
                    if first_err.is_none() {
                        first_err = Some(msg);
                    }
                }
            }
        }
        if w.releases {
            match poll_release(&client, &w, cfg).await {
                Ok(Some(ev)) => {
                    println!("[poll {}] {} release: 新版本 {}", now_str(), w.repo, ev.title);
                    events.push(ev);
                }
                Ok(None) => println!("[poll {}] {} release: 无变化", now_str(), w.repo),
                Err(e) => {
                    let msg = format!("{} (release): {e}", w.repo);
                    eprintln!("[poll {}] {} release 拉取失败", now_str(), w.repo);
                    if first_err.is_none() {
                        first_err = Some(msg);
                    }
                }
            }
        }
    }
    (events, first_err)
}

#[derive(Deserialize)]
struct Comment {
    id: i64,
    body: Option<String>,
    html_url: String,
    created_at: String,
}

#[derive(Deserialize)]
struct IssueDetail {
    state: String,
    title: String,
    html_url: String,
}

fn first_line(text: &Option<String>, max: usize) -> String {
    text.as_deref()
        .unwrap_or("")
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|l| {
            if l.chars().count() > max {
                let s: String = l.chars().take(max).collect();
                format!("{s}…")
            } else {
                l.to_string()
            }
        })
        .unwrap_or_else(|| "(空评论)".into())
}

/// 单条 Issue/PR 监听：新评论 + 状态开关变更
async fn poll_item(client: &reqwest::Client, w: &Watch, cfg: &mut Config) -> Result<Vec<Event>, String> {
    let n = w.number.ok_or("item 监听缺少 number")?;
    let mut events = Vec::new();

    if w.state_change {
        let url = format!("https://api.github.com/repos/{}/issues/{}", w.repo, n);
        let issue: IssueDetail = client
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("{e:?}"))?
            .json()
            .await
            .map_err(|e| format!("{e:?}"))?;
        match cfg.cursor_state.get(&w.id) {
            None => {
                cfg.cursor_state.insert(w.id.clone(), issue.state.clone());
            }
            Some(s) if *s != issue.state => {
                events.push(Event {
                    kind: "state".into(),
                    repo: w.repo.clone(),
                    number: Some(n),
                    title: format!("「{}」状态 {} → {}", issue.title, s, issue.state),
                    url: issue.html_url.clone(),
                    created_at: chrono::Local::now().to_rfc3339(),
                });
                cfg.cursor_state.insert(w.id.clone(), issue.state.clone());
            }
            _ => {}
        }
    }

    if w.comments {
        let url = format!(
            "https://api.github.com/repos/{}/issues/{}/comments?per_page=15&sort=created&direction=desc",
            w.repo, n
        );
        let comments: Vec<Comment> = client
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("{e:?}"))?
            .json()
            .await
            .map_err(|e| format!("{e:?}"))?;
        // 返回是降序（最新在前）
        let latest_id = comments.first().map(|c| c.id);
        let cursor = cfg.cursor_comment.get(&w.id).copied();
        let first_run = cursor.is_none();
        // 升序遍历新于游标的评论，事件按时间正序浮现
        for c in comments.iter().rev() {
            if let Some(last) = cursor
                && c.id <= last
            {
                continue;
            }
            events.push(Event {
                kind: "comment".into(),
                repo: w.repo.clone(),
                number: Some(n),
                title: first_line(&c.body, 60),
                url: c.html_url.clone(),
                created_at: c.created_at.clone(),
            });
        }
        if first_run {
            events.clear(); // 首次只记游标
        }
        if let Some(id) = latest_id {
            cfg.cursor_comment.insert(w.id.clone(), id);
        }
    }

    Ok(events)
}

async fn poll_issues(client: &reqwest::Client, w: &Watch, cfg: &mut Config) -> Result<Vec<Event>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/issues?state=all&sort=created&direction=desc&per_page=15",
        w.repo
    );
    let items: Vec<Issue> = client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        // Debug 格式带完整 source 链，能看出是 DNS/connect/TLS 哪层挂了
        .map_err(|e| format!("{e:?}"))?
        .json()
        .await
        .map_err(|e| format!("{e:?}"))?;

    let latest = items.first().map(|i| i.created_at.clone());
    let cursor = cfg.cursor_issues.get(&w.id).cloned();
    let first_run = cursor.is_none();
    let mut events = Vec::new();

    for i in items {
        let is_pr = i.pull_request.is_some();
        if is_pr && !w.prs {
            continue;
        }
        if !is_pr && !w.issues {
            continue;
        }
        if let Some(c) = &cursor
            && i.created_at <= *c
        {
            continue;
        }
        events.push(Event {
            kind: if is_pr { "pr" } else { "issue" }.into(),
            repo: w.repo.clone(),
            number: Some(i.number),
            title: i.title,
            url: i.html_url,
            created_at: i.created_at,
        });
    }
    // 首次只记游标，避免把存量 issue 全报出来
    if first_run {
        events.clear();
    }
    if let Some(c) = latest {
        cfg.cursor_issues.insert(w.id.clone(), c);
    }
    Ok(events)
}

async fn poll_release(client: &reqwest::Client, w: &Watch, cfg: &mut Config) -> Result<Option<Event>, String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", w.repo);
    let rel: Release = match client.get(&url).send().await {
        Ok(r) => match r.error_for_status() {
            Ok(r) => r.json().await.map_err(|e| e.to_string())?,
            // 404 = 该仓库还没有 release，不算错误
            Err(_) => {
                return Ok(None);
            }
        },
        Err(e) => return Err(format!("{e:?}")),
    };
    let id_str = rel.id.to_string();
    if cfg.cursor_release.get(&w.id) == Some(&id_str) {
        return Ok(None);
    }
    let first_run = !cfg.cursor_release.contains_key(&w.id);
    cfg.cursor_release.insert(w.id.clone(), id_str);
    if first_run {
        return Ok(None);
    }
    Ok(Some(Event {
        kind: "release".into(),
        repo: w.repo.clone(),
        number: None,
        title: rel.name.unwrap_or(rel.tag_name),
        url: rel.html_url,
        created_at: rel.published_at.unwrap_or_default(),
    }))
}
