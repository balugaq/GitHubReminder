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
            releases: false,
            comments: true,
            state_change: true,
            title: None,
            created: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct StoredEvent {
    pub kind: String, // "issue" | "pr" | "release" | "comment" | "state"
    pub repo: String,
    pub number: Option<u64>,
    pub title: String,
    pub url: String,
    pub created_at: String,
    pub read: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub token: Option<String>,
    pub proxy: Option<String>, // 例如 http://127.0.0.1:7890；空则走系统代理
    #[serde(default = "default_true")]
    pub notify: bool, // 新事件时弹系统 Toast
    pub watches: Vec<Watch>,
    // 游标：首次轮询只记游标不报事件
    pub cursor_issues: HashMap<String, String>,   // watch_id -> 最后一条 issue/PR created_at
    pub cursor_release: HashMap<String, String>,  // watch_id -> 最后 release id
    pub cursor_comment: HashMap<String, i64>,     // watch_id -> 最后一条评论 id
    pub cursor_state: HashMap<String, String>,    // watch_id -> 最后见到的 state (open/closed)
    // 通知持久化（上限 200 条，超出丢最旧），read 由前端点卡时经 mark_read 置位
    #[serde(default)]
    pub events: Vec<StoredEvent>,
    // 统计：维度("repo" 或 "repo#n") -> 类型(kind) -> 次数，跨重启累计
    #[serde(default)]
    pub stats: BTreeMap<String, BTreeMap<String, u64>>,
}

pub fn load(path: &PathBuf) -> Config {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &PathBuf, cfg: &Config) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let json = serde_json::to_string_pretty(cfg).unwrap_or_default();
    let _ = fs::write(path, json);
}
