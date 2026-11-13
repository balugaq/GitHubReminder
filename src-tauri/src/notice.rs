// QQ 通知文案 + 多选复制共用的一套格式：按项目分组、时间升序、bot 通知下沉。
// QQ 推送从 github::Event 构造，多选复制从 store::Entity 构造，最终都走 format_notice。

use std::collections::BTreeMap;

use chrono::{DateTime, Local, Utc};

use crate::github::Event;
use crate::store::Entity;

pub const GR_TITLE: &str = "[GitHubReminder] 通知 · GitHub 动态";

#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NoticeKey {
    pub repo: String,
    pub number: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub number: Option<u64>,
    pub title: String,
    pub url: String,
    pub at: String,
    pub author: String,
    pub at_me: bool,
    pub action: String,
    pub is_release: bool,
}

fn event_ts(at: &str) -> i64 {
    DateTime::parse_from_rfc3339(at)
        .map(|d| d.timestamp())
        .unwrap_or_else(|_| Utc::now().timestamp())
}

fn short_time(at: &str) -> String {
    DateTime::parse_from_rfc3339(at)
        .map(|d| d.with_timezone(&Local).format("%y-%m %H:%M:%S").to_string())
        .unwrap_or_else(|_| Local::now().format("%y-%m %H:%M:%S").to_string())
}

// 「谁 动作 #编号 : 标题」句式里，状态/评论统一用中性的「更新了」：
// 评论事件的 author 是 issue 发起者而非评论者，写「评论了」会张冠李戴。
pub fn action_of(kind: &str) -> &'static str {
    match kind {
        "issue" | "pr" => "开启了",
        "closed_issue" | "closed_pr" => "关闭了",
        _ => "更新了",
    }
}

pub fn format_notice(n: &Notice) -> String {
    let who = n.author.trim();
    let prefix = if who.is_empty() {
        String::new()
    } else {
        format!("{who} ")
    };
    if n.is_release {
        return format!(
            "[{}] {}发布了 {} 版本\n点击跳转：{}",
            short_time(&n.at),
            prefix,
            n.title,
            n.url
        );
    }
    let mark = if n.at_me { "[@me] " } else { "" };
    let num = n
        .number
        .map(|v| format!("#{v}"))
        .unwrap_or_default();
    format!(
        "{mark}{prefix}{} {num} : {}\n点击跳转：{}",
        n.action, n.title, n.url
    )
}

fn notice_from_event(ev: &Event, me: Option<&str>) -> Notice {
    Notice {
        number: ev.number,
        title: ev.title.clone(),
        url: ev.url.clone(),
        at: ev.created_at.clone(),
        author: ev.author.clone(),
        at_me: me.is_some_and(|m| ev.assignees.iter().any(|a| a.trim().to_lowercase() == m)),
        action: action_of(&ev.kind).into(),
        is_release: ev.kind == "release",
    }
}

pub fn notice_from_entity(e: &Entity) -> Notice {
    Notice {
        number: e.number,
        title: e.title.clone(),
        url: e.url.clone(),
        at: e.updated_at.clone(),
        author: e.author.clone(),
        at_me: e.at_me,
        action: match e.last_event_kind.as_str() {
            "open" => "开启了",
            "closed" => "关闭了",
            _ => "更新了",
        }
        .into(),
        is_release: e.kind == "release",
    }
}

// bot 关键词按 contains 匹配 issue/pr 发起者；release 的发布者不参与折叠
pub fn is_bot(ev: &Event, keywords: &[String]) -> bool {
    if ev.kind == "release" {
        return false;
    }
    let author = ev.author.trim().to_lowercase();
    if author.is_empty() {
        return false;
    }
    keywords.iter().any(|k| {
        let key = k.trim().to_lowercase();
        !key.is_empty() && author.contains(&key)
    })
}

// 每个项目一条消息：固定标题 + 项目标题 + 一般通知（旧→新）+ bot 通知（旧→新），块间空行
pub fn build_messages(
    events: &[Event],
    my_username: Option<&str>,
    bot_keywords: &[String],
) -> Vec<String> {
    let me = my_username
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let mut groups: BTreeMap<String, Vec<&Event>> = BTreeMap::new();
    for ev in events {
        groups.entry(ev.repo.clone()).or_default().push(ev);
    }
    let mut out = Vec::new();
    for (repo, mut list) in groups {
        list.sort_by_key(|ev| event_ts(&ev.created_at));
        let (normal, bots): (Vec<&Event>, Vec<&Event>) =
            list.into_iter().partition(|ev| !is_bot(ev, bot_keywords));
        let blocks: Vec<String> = normal
            .iter()
            .chain(bots.iter())
            .map(|ev| format_notice(&notice_from_event(ev, me.as_deref())))
            .collect();
        out.push(
            [
                GR_TITLE.to_string(),
                format!("{repo} · {} 个通知", blocks.len()),
                blocks.join("\n\n"),
            ]
            .join("\n"),
        );
    }
    out
}

// 多选复制：按传入顺序取实体，只留小通知块与块间空行，不带大标题与项目标题
pub fn format_selected(entities: &[Entity], keys: &[NoticeKey]) -> String {
    let mut blocks = Vec::new();
    for key in keys {
        let Some(e) = entities.iter().find(|e| {
            e.repo.eq_ignore_ascii_case(key.repo.trim()) && e.number == key.number
        }) else {
            continue;
        };
        blocks.push(format_notice(&notice_from_entity(e)));
    }
    blocks.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(repo: &str, kind: &str, num: u64, at: &str, author: &str) -> Event {
        Event {
            kind: kind.into(),
            repo: repo.into(),
            number: Some(num),
            title: "示例标题".into(),
            url: format!("https://github.com/{repo}/issues/{num}"),
            created_at: at.into(),
            author: author.into(),
            assignees: vec!["octocat".into()],
        }
    }

    fn entity(repo: &str, num: u64) -> Entity {
        Entity {
            kind: "issue".into(),
            repo: repo.into(),
            number: Some(num),
            title: "示例标题".into(),
            url: format!("https://github.com/{repo}/issues/{num}"),
            last_event_kind: "open".into(),
            updated_at: "2026-11-01T00:00:00+08:00".into(),
            author: "octocat".into(),
            ..Entity::default()
        }
    }

    #[test]
    fn one_message_per_repo_with_project_header() {
        let events = vec![
            ev("o/alpha", "issue", 1, "2026-11-01T00:00:00Z", "octocat"),
            ev("o/alpha", "issue", 2, "2026-11-01T00:01:00Z", "octocat"),
            ev("o/beta", "pr", 3, "2026-11-01T00:02:00Z", "octocat"),
        ];
        let msgs = build_messages(&events, None, &[]);
        assert_eq!(msgs.len(), 2, "两个项目应分成两条消息");
        let lines: Vec<&str> = msgs[0].lines().collect();
        assert_eq!(lines[0], GR_TITLE);
        assert_eq!(lines[1], "o/alpha · 2 个通知");
        assert!(msgs[1].contains("o/beta · 1 个通知"));
    }

    #[test]
    fn notices_are_oldest_first_with_blank_line_between() {
        let events = vec![
            ev("o/alpha", "issue", 9, "2026-11-01T03:00:00Z", "octocat"),
            ev("o/alpha", "issue", 1, "2026-11-01T01:00:00Z", "octocat"),
        ];
        let msg = build_messages(&events, None, &[]).remove(0);
        assert!(msg.find("#1").unwrap() < msg.find("#9").unwrap());
        // 两个块之间恰好一个空行
        assert!(msg.contains("\n\n"), "块之间应留空行");
        assert!(!msg.contains("\n\n\n"), "不应出现连续两个空行");
    }

    #[test]
    fn at_me_only_when_assignee_matches() {
        let mut e = ev("o/alpha", "issue", 1, "2026-11-01T00:00:00Z", "octocat");
        let with = build_messages(&[e.clone()], Some("octocat"), &[]).remove(0);
        assert!(with.contains("[@me] octocat 开启了 #1 : 示例标题"));
        e.assignees = vec!["bob".into()];
        let without = build_messages(&[e], Some("octocat"), &[]).remove(0);
        assert!(!without.contains("[@me]"));
        assert!(without.contains("octocat 开启了 #1"));
    }

    #[test]
    fn release_uses_timestamp_and_version_template() {
        let mut e = ev("o/alpha", "release", 1, "2026-11-01T00:00:00Z", "octocat");
        e.number = None;
        e.title = "v1.0".into();
        e.url = "https://github.com/o/alpha/releases/tag/v1.0".into();
        let msg = build_messages(&[e], None, &[]).remove(0);
        assert!(msg.contains("] octocat 发布了 v1.0 版本"), "release 文案: {msg}");
        assert!(msg.contains("点击跳转：https://github.com/o/alpha/releases/tag/v1.0"));
        assert!(msg.lines().nth(2).unwrap().starts_with("["));
    }

    #[test]
    fn bot_notices_sink_to_the_end_of_their_repo() {
        // bot 的时间故意比一般通知更早，纯按时间排会混在前面，能真正验出「下沉」
        let events = vec![
            ev("o/alpha", "issue", 3, "2026-11-01T01:00:00Z", "dependabot[bot]"),
            ev("o/alpha", "issue", 1, "2026-11-01T02:00:00Z", "octocat"),
            ev("o/alpha", "pr", 4, "2026-11-01T03:00:00Z", "renovate[bot]"),
            ev("o/alpha", "issue", 2, "2026-11-01T04:00:00Z", "octocat"),
        ];
        let keywords = vec!["dependabot[bot]".into(), "renovate[bot]".into()];
        let msg = build_messages(&events, None, &keywords).remove(0);
        let p1 = msg.find("#1").unwrap();
        let p2 = msg.find("#2").unwrap();
        let p3 = msg.find("#3").unwrap();
        let p4 = msg.find("#4").unwrap();
        assert!(p1 < p2 && p2 < p3 && p3 < p4, "一般通知按时间在前，bot 按时间在后: {msg}");
        assert_eq!(msg.lines().next(), Some(GR_TITLE));
        assert!(msg.contains("o/alpha · 4 个通知"));
    }

    #[test]
    fn bot_keywords_match_by_contains_and_skip_release() {
        let mut rel = ev("o/alpha", "release", 1, "2026-11-01T00:00:00Z", "dependabot[bot]");
        rel.number = None;
        assert!(!is_bot(&rel, &["dependabot[bot]".into()]));
        let e = ev("o/alpha", "pr", 2, "2026-11-01T00:00:00Z", "my-dependabot[bot]-fork");
        assert!(is_bot(&e, &["dependabot[bot]".into()]));
    }

    #[test]
    fn selected_text_keeps_order_and_drops_headers() {
        let entities = vec![entity("o/alpha", 1), entity("o/alpha", 2)];
        let keys = vec![
            NoticeKey { repo: "o/alpha".into(), number: Some(2) },
            NoticeKey { repo: "o/alpha".into(), number: Some(1) },
        ];
        let text = format_selected(&entities, &keys);
        assert!(text.find("#2").unwrap() < text.find("#1").unwrap(), "按选择顺序");
        assert!(!text.contains(GR_TITLE));
        assert!(!text.contains("个通知"));
        assert!(text.contains("\n\n"), "块之间保留空行");
    }
}
