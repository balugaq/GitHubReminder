// QQ 互联中转服务（由 qq_relay.js 改写内置）：本地 HTTP 端点收文本，经 QQ 机器人
// 主动推到私信（C2C）。全部配置来自设置页，无默认值；缺任一项则服务不启动。
// 端点：POST /send {"text":...}  |  GET /send?text=...
// 注意：QQ 机器人 C2C 主动消息有每日额度，调用方自行节制。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub struct RelayCfg {
    pub port: u16,
    pub app_id: String,
    pub app_secret: String,
    pub openid: String,
}

impl RelayCfg {
    pub fn from_parts(
        port: Option<u16>,
        app_id: Option<&str>,
        app_secret: Option<&str>,
        openid: Option<&str>,
    ) -> Option<Self> {
        let build = |p: u16, a: &str, s: &str, o: &str| {
            (!a.is_empty() && !s.is_empty() && !o.is_empty()).then(|| Self {
                port: p,
                app_id: a.into(),
                app_secret: s.into(),
                openid: o.into(),
            })
        };
        port.and_then(|p| {
            app_id
                .zip(app_secret)
                .zip(openid)
                .and_then(|((a, s), o)| build(p, a, s, o))
        })
    }
}

const MAX_LEN: usize = 1900; // 单条文本上限，超出自动分段
const TOKEN_URL: &str = "https://bots.qq.com/app/getAppAccessToken";

struct TokenCache {
    token: String,
    expire_at: Instant,
}

// token 过期前 1 分钟刷新，避免边界失败
async fn get_token(cfg: &RelayCfg, cache: &Mutex<TokenCache>) -> Result<String, String> {
    {
        let c = cache.lock().unwrap();
        if !c.token.is_empty() && Instant::now() < c.expire_at {
            return Ok(c.token.clone());
        }
    } // 锁不跨 await（future 需 Send）
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "appId": cfg.app_id,
            "clientSecret": cfg.app_secret
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let data: Value = resp.json().await.map_err(|e| e.to_string())?;
    let token = data["access_token"]
        .as_str()
        .ok_or_else(|| format!("getToken failed: {data}"))?
        .to_string();
    let expires = data["expires_in"].as_u64().unwrap_or(7200);
    let mut c = cache.lock().unwrap();
    c.token = token.clone();
    c.expire_at = Instant::now() + Duration::from_secs(expires.saturating_sub(60));
    Ok(token)
}

// QQ 文本里 @ 会触发 mention，转义掉避免误触
fn escape_at(text: &str) -> String {
    text.replace('@', "\\u0040")
}

fn split_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    text.chars()
        .collect::<Vec<_>>()
        .chunks(MAX_LEN)
        .map(|c| c.iter().collect())
        .collect()
}

// 推送到 QQ 私信；返回成功发送的分段数
pub async fn send_to_qq(cfg: &RelayCfg, text: &str) -> Result<usize, String> {
    let cache = Mutex::new(TokenCache {
        token: String::new(),
        expire_at: Instant::now() - Duration::from_secs(1),
    });
    let token = get_token(cfg, &cache).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("https://api.sgroup.qq.com/v2/users/{}/messages", cfg.openid);
    let mut sent = 0;
    for p in split_text(text) {
        let resp = client
            .post(&url)
            .header("Authorization", format!("QQBot {token}"))
            .header("Content-Type", "application/json")
            .header("X-Im-Biz-Type", "1") // 1 = 私信 C2C
            .body(serde_json::json!({ "content": escape_at(&p), "msg_type": 0 }).to_string())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("send failed: HTTP {status} {body}"));
        }
        sent += 1;
    }
    Ok(sent)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn cors_headers() -> String {
    "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Content-Type\r\n".into()
}

// 最小 HTTP/1.1 解析：只服务本机 localhost 简单调用，够用即可
async fn handle_conn(mut stream: TcpStream, cfg: Arc<RelayCfg>) {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        match stream.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 65536 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let first = head.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let content_length = head
        .lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let body_start = header_end + 4;
    while buf.len() < body_start + content_length {
        match stream.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }

    if method == "OPTIONS" {
        let resp = format!("HTTP/1.1 204 No Content\r\n{}\r\n", cors_headers());
        let _ = stream.write_all(resp.as_bytes()).await;
        return;
    }

    let text = match method.as_str() {
        "POST" => {
            let body = String::from_utf8_lossy(&buf[body_start..]);
            serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|j| {
                    j["text"]
                        .as_str()
                        .or_else(|| j["content"].as_str())
                        .or_else(|| j["message"].as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body.trim().to_string())
        }
        "GET" => path
            .split_once('?')
            .and_then(|(_, q)| {
                q.split('&').find_map(|kv| {
                    kv.split_once('=')
                        .filter(|(k, _)| *k == "text")
                        .map(|(_, v)| percent_decode(v))
                })
            })
            .unwrap_or_default(),
        _ => String::new(),
    };

    if text.trim().is_empty() {
        let resp = format!("HTTP/1.1 400 Bad Request\r\n{}\r\nmissing text", cors_headers());
        let _ = stream.write_all(resp.as_bytes()).await;
        return;
    }

    let (status, body) = match send_to_qq(&cfg, &text).await {
        Ok(n) => (
            200,
            serde_json::json!({ "ok": true, "sent": n }).to_string(),
        ),
        Err(e) => (
            500,
            serde_json::json!({ "ok": false, "error": e }).to_string(),
        ),
    };
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{}\r\n{body}",
        cors_headers()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
}

// 常驻监听循环；绑定失败（端口占用等）只打日志不重试
pub async fn run_server(cfg: Arc<RelayCfg>) {
    let listener = match TcpListener::bind(("127.0.0.1", cfg.port)).await {
        Ok(l) => l,
        Err(e) => {
            println!("[qq] 中转服务绑定 127.0.0.1:{} 失败: {e}", cfg.port);
            return;
        }
    };
    println!("[qq] 中转服务已监听 http://localhost:{}", cfg.port);
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let cfg = cfg.clone();
                tokio::spawn(handle_conn(stream, cfg));
            }
            Err(e) => println!("[qq] accept 失败: {e}"),
        }
    }
}
