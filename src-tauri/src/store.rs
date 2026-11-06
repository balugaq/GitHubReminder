use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Watch {
    pub id: String,
    pub repo: String, // owner/name
    // Some(n) = 单条 Issue/PR 监听（M3）；None = 仓库级监听
    #[serde(default)]
    pub number: Option<u64>,
    pub issues: bool,
    pub prs: bool,
    pub releases: bool,
    // 仓库级关闭事件（closed_at 变化才报，评论不影响）
    #[serde(default)]
    pub closed_issues: bool,
    #[serde(default)]
    pub closed_prs: bool,
    // 单条颗粒度选项
    #[serde(default)]
    pub comments: bool,
    #[serde(default)]
    pub state_change: bool,
    // 用户自定义显示名；None 时前端回退到默认格式
    #[serde(default)]
    pub title: Option<String>,
    // 添加时间（unix 秒），用于管理窗口排序
    #[serde(default)]
    pub created: u64,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            id: String::new(),
            repo: String::new(),
            number: None,
            issues: true,
            prs: true,
            releases: true,
            closed_issues: true,
            closed_prs: true,
            comments: true,
            state_change: true,
            title: None,
            created: 0,
        }
    }
}

// 通知实体：以 issue/pr/release 为单位聚合后的卡片。同一 repo+number 的多条变化事件
// 合并成一张卡，updated_at 取最近一次变化，有新动态时 status 重置为未读。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Entity {
    pub kind: String,            // 实体类型: "issue" | "pr" | "release"
    pub repo: String,            // owner/name
    pub number: Option<u64>,     // issue/PR 编号；release 用其 id 保证唯一
    pub title: String,
    pub url: String,
    pub state: String,           // "open" | "closed" | "published"
    pub last_event_kind: String, // 最近一次变化类型: "open" | "comment" | "state" | "closed" | "release"
    pub updated_at: String,      // 最近变化时间 (ISO 8601)，排序键
    pub status: String,          // "unread" | "read" | "in_progress" | "done"（默认 unread）
    pub pinned: bool,            // 用户显式钉住（默认 false，最多 3 个）
    // 卡片发起者（issue/pr 作者、release 发布者）：实体首次写入后不再被后续事件覆盖，
    // 供主界面搜索「author」用。旧存档没有该字段，serde 默认空串。
    pub author: String,
    // 「涉及」过这张卡的人：历次事件的 author + assignees，统一小写、去重、累积。
    // 供搜索语法 @//用户名/ 判定「与谁有关」，比 author 更宽（含指派人）。
    // 评论事件的 author 仍是 issue/pr 发起者（见 github.rs::poll_item），故不含评论者。
    pub people: Vec<String>,
}

// 主窗口位置与尺寸（物理像素），移动/拉伸时记录，下次启动恢复
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    // 凭据管理器是唯一持久化位置；save 落盘前会剔除，仅 API 响应与内存中可见
    pub token: Option<String>,
    pub proxy: Option<String>, // 例如 http://127.0.0.1:7890；空则走系统代理
    #[serde(default = "default_true")]
    pub notify: bool, // 新事件时弹系统 Toast
    // 轮询间隔秒数；None/0 视为默认 60，运行时夹到 [10, 3600]
    #[serde(default)]
    pub poll_interval: Option<u64>,
    // 用户的 GitHub 用户名（自行填写）：用于判断 issue 的 assignee 里是否有本人
    #[serde(default)]
    pub github_username: Option<String>,
    // QQ 互联通知：内置中转服务（qqrelay.rs）推 QQ 私信，失败静默。
    // 全部无默认值，缺任一项则中转不启动、推送不发。
    #[serde(default)]
    pub qq_notify: bool,
    // 自动配置：开启 = 由本应用启动内置 QQ 中转服务；关闭 = 外部已有服务，只发推送
    #[serde(default)]
    pub qq_autoconfig: bool,
    #[serde(default)]
    pub qq_relay_port: Option<u16>,
    #[serde(default)]
    pub qq_app_id: Option<String>,
    // 与 token 一样：凭据管理器是唯一持久化位置，save 落盘前会剔除，
    // 仅 API 响应与内存中可见；旧 config.json 里的明文在启动时一次性迁移。
    #[serde(default)]
    pub qq_app_secret: Option<String>,
    #[serde(default)]
    pub qq_openid: Option<String>,
    // 主窗口上次的位置/尺寸；None = 从未记录过（走 tauri.conf 默认居中）
    #[serde(default)]
    pub window: Option<WindowRect>,
    pub watches: Vec<Watch>,
    // 游标：首次轮询只记游标不报事件
    pub cursor_issues: HashMap<String, String>,   // watch_id -> 最后一条 issue/PR created_at
    pub cursor_release: HashMap<String, String>,  // watch_id -> 最后 release id
    pub cursor_comment: HashMap<String, i64>,     // watch_id -> 最后一条评论 id
    pub cursor_state: HashMap<String, String>,    // watch_id -> 最后见到的 state (open/closed)
    pub cursor_closed: HashMap<String, String>,   // watch_id -> 最后见到的 closed_at
    // 通知实体（按 issue/pr/release 聚合，上限 200 条，超出丢最旧）；
    // status / pinned 由前端经对应命令置位。旧版 events 字段不兼容，serde 忽略即丢弃（不迁移）。
    #[serde(default)]
    pub entities: Vec<Entity>,
    // 统计：维度("repo" 或 "repo#n") -> 类型(kind) -> 次数，跨重启累计
    #[serde(default)]
    pub stats: BTreeMap<String, BTreeMap<String, u64>>,
}

pub fn load(path: &PathBuf) -> Config {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        // 文件不存在 = 首次使用，静默走默认值
        Err(_) => return Config::default(),
    };
    match serde_json::from_str(&raw) {
        Ok(cfg) => cfg,
        Err(e) => {
            // 解析失败绝不能静默清空（会连带丢监听和通知）：先备份原文件再回退默认值
            let backup = path.with_file_name("config.broken.json");
            let _ = fs::copy(path, &backup);
            eprintln!(
                "[store] 配置解析失败，已备份到 {} 并回退默认值: {e}",
                backup.display()
            );
            Config::default()
        }
    }
}

pub fn save(path: &PathBuf, cfg: &Config) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    // 密钥类字段不落盘（存 Windows 凭据管理器），剔除后再序列化
    let mut disk = cfg.clone();
    disk.token = None;
    disk.qq_app_secret = None;
    let json = match serde_json::to_string_pretty(&disk) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[store] 配置序列化失败，跳过本次保存: {e}");
            return;
        }
    };
    // 原子写：先写临时文件再改名覆盖，避免进程被强杀时留下半截 JSON
    // （半截 JSON 下次启动解析失败 → 整体回退默认值 → 通知全丢）
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = fs::write(&tmp, json) {
        eprintln!("[store] 配置临时文件写入失败，保留原文件: {e}");
        return;
    }
    if let Err(e) = fs::rename(&tmp, path) {
        eprintln!("[store] 配置改名失败，保留原文件: {e}");
        let _ = fs::remove_file(&tmp);
    }
}

const CRED_SERVICE: &str = "GitHubReminder";
const CRED_USER_PAT: &str = "pat";
const CRED_USER_QQ_SECRET: &str = "qq_app_secret";

fn load_credential(user: &str) -> Option<String> {
    let entry = keyring::Entry::new(CRED_SERVICE, user).ok()?;
    entry.get_password().ok().filter(|s| !s.is_empty())
}

fn save_credential(user: &str, value: Option<&str>) -> Result<(), String> {
    let entry = keyring::Entry::new(CRED_SERVICE, user).map_err(|e| e.to_string())?;
    match value {
        Some(s) => entry.set_password(s).map_err(|e| e.to_string()),
        None => match entry.delete_credential() {
            // 本来就没存过也算删除成功
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        },
    }
}

pub fn load_token() -> Option<String> {
    load_credential(CRED_USER_PAT)
}

pub fn save_token(token: Option<&str>) -> Result<(), String> {
    save_credential(CRED_USER_PAT, token)
}

pub fn load_qq_app_secret() -> Option<String> {
    load_credential(CRED_USER_QQ_SECRET)
}

pub fn save_qq_app_secret(secret: Option<&str>) -> Result<(), String> {
    save_credential(CRED_USER_QQ_SECRET, secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_overwrites_existing_file_on_windows() {
        let dir = std::env::temp_dir().join(format!("ghr-store-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("config.json");

        let mut cfg = Config::default();
        cfg.watches.push(Watch {
            id: "w1".into(),
            repo: "o/r".into(),
            ..Watch::default()
        });
        save(&path, &cfg);
        assert!(path.exists(), "首次保存应生成文件");

        // 二次保存必须能覆盖已存在文件（rename 覆盖语义），且不能留下半截内容
        cfg.watches.push(Watch {
            id: "w2".into(),
            repo: "o/r2".into(),
            ..Watch::default()
        });
        save(&path, &cfg);
        let raw = fs::read_to_string(&path).expect("覆盖后应仍可读");
        let back: Config = serde_json::from_str(&raw).expect("覆盖后内容应仍是合法 JSON");
        assert_eq!(back.watches.len(), 2, "应读到两份监听");
        assert!(
            !dir.join("config.json.tmp").exists(),
            "临时文件不应残留"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_strips_credentials_from_disk() {
        let dir = std::env::temp_dir().join(format!("ghr-cred-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("config.json");

        let cfg = Config {
            token: Some("pat-example-value".into()),
            qq_app_secret: Some("qq-example-secret".into()),
            ..Config::default()
        };
        save(&path, &cfg);

        let raw = fs::read_to_string(&path).expect("配置应可读");
        assert!(!raw.contains("pat-example-value"), "PAT 不应落盘");
        assert!(
            !raw.contains("qq-example-secret"),
            "QQ AppSecret 不应落盘"
        );
        let back: Config = serde_json::from_str(&raw).expect("落盘内容应仍是合法 JSON");
        assert!(back.token.is_none(), "读回的 PAT 应为空");
        assert!(back.qq_app_secret.is_none(), "读回的 QQ AppSecret 应为空");
        assert_eq!(
            cfg.qq_app_secret.as_deref(),
            Some("qq-example-secret"),
            "内存里的值不应被改动"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
