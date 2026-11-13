// 统计纯逻辑：按本机自然日 / 自然周 / 自然月 / 全部四个窗口，把通知流水聚合成项目占比。
// 不碰 AppState，方便单测直接喂时间戳。

use std::collections::BTreeMap;

use chrono::{Datelike, Local, TimeZone};

use crate::store::{Entity, StatEvent};

pub const MAX_STAT_EVENTS: usize = 5000;

#[derive(serde::Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RangeCount {
    pub today: BTreeMap<String, u64>,
    pub week: BTreeMap<String, u64>,
    pub month: BTreeMap<String, u64>,
    pub history: BTreeMap<String, u64>,
}

// 返回 [今日 00:00, 本周一 00:00, 本月 1 日 00:00] 的 unix 秒
pub fn window_starts(now: i64) -> [i64; 3] {
    let local = Local
        .timestamp_opt(now, 0)
        .single()
        .unwrap_or_else(Local::now);
    let day0 = Local
        .with_ymd_and_hms(local.year(), local.month(), local.day(), 0, 0, 0)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(now);
    let month0 = Local
        .with_ymd_and_hms(local.year(), local.month(), 1, 0, 0, 0)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(day0);
    let week0 = day0 - i64::from(local.weekday().num_days_from_monday()) * 86_400;
    [day0, week0, month0]
}

pub fn count_since(events: &[StatEvent], since: i64) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for e in events.iter().filter(|e| e.at >= since) {
        *out.entry(e.repo.clone()).or_insert(0) += 1;
    }
    out
}

// 升级后流水为空时拿现有实体按项目兜一次，避免历史视图一片空白
pub fn history_counts(events: &[StatEvent], entities: &[Entity]) -> BTreeMap<String, u64> {
    let all = count_since(events, i64::MIN);
    if !all.is_empty() {
        return all;
    }
    let mut out = BTreeMap::new();
    for e in entities {
        *out.entry(e.repo.clone()).or_insert(0) += 1;
    }
    out
}

pub fn snapshot(events: &[StatEvent], entities: &[Entity], now: i64) -> RangeCount {
    let [today, week, month] = window_starts(now);
    RangeCount {
        today: count_since(events, today),
        week: count_since(events, week),
        month: count_since(events, month),
        history: history_counts(events, entities),
    }
}

// 流水超上限时丢最旧的，排序只在触顶那一次做
pub fn trim_log(events: &mut Vec<StatEvent>) {
    if events.len() <= MAX_STAT_EVENTS {
        return;
    }
    events.sort_by_key(|e| e.at);
    let drop = events.len() - MAX_STAT_EVENTS;
    events.drain(..drop);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn ev(repo: &str, at: i64) -> StatEvent {
        StatEvent {
            repo: repo.into(),
            at,
        }
    }

    #[test]
    fn windows_start_at_local_boundaries() {
        let now = Local::now().timestamp();
        let [today, week, month] = window_starts(now);
        let d = Local.timestamp_opt(today, 0).single().unwrap();
        assert_eq!((d.hour(), d.minute(), d.second()), (0, 0, 0));
        let w = Local.timestamp_opt(week, 0).single().unwrap();
        assert_eq!(w.weekday(), chrono::Weekday::Mon);
        let m = Local.timestamp_opt(month, 0).single().unwrap();
        assert_eq!(m.day(), 1);
        assert!(week <= today && month <= today && today <= now);
    }

    #[test]
    fn count_since_includes_the_boundary() {
        let events = vec![ev("o/a", 100), ev("o/a", 200), ev("o/b", 50)];
        let counts = count_since(&events, 100);
        assert_eq!(counts.get("o/a"), Some(&2));
        assert_eq!(counts.get("o/b"), None);
    }

    #[test]
    fn history_falls_back_to_entities_only_when_log_is_empty() {
        let entities = vec![
            Entity {
                repo: "o/a".into(),
                ..Entity::default()
            },
            Entity {
                repo: "o/b".into(),
                ..Entity::default()
            },
        ];
        let fb = history_counts(&[], &entities);
        assert_eq!(fb.get("o/a"), Some(&1));
        assert_eq!(fb.get("o/b"), Some(&1));
        let logged = history_counts(&[ev("o/a", 1)], &entities);
        assert_eq!(logged.get("o/b"), None, "有流水时不再用实体兜底");
    }

    #[test]
    fn trim_keeps_newest_entries() {
        let mut log: Vec<StatEvent> = (0..MAX_STAT_EVENTS + 7)
            .map(|i| ev("o/r", MAX_STAT_EVENTS as i64 - i as i64))
            .collect();
        trim_log(&mut log);
        assert_eq!(log.len(), MAX_STAT_EVENTS);
        assert_eq!(log.first().map(|e| e.at), Some(1));
        assert_eq!(log.last().map(|e| e.at), Some(MAX_STAT_EVENTS as i64));
    }
}
