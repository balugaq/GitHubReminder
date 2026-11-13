// 批量标记状态：把选中的卡片转成后端命令参数，同时留一份可撤销的状态快照。
// 纯逻辑、无 DOM 依赖，浏览器挂 window.GhrBatch，Node vm 单测复用同一份。
(function (root) {
  "use strict";

  function normalize(ev) {
    return {
      repo: String((ev && ev.repo) || ""),
      number: ev && ev.number != null ? ev.number : null,
      status: String((ev && ev.status) || "unread"),
    };
  }

  // targets 是当前选中的实体；status 是要批量设置的目标状态
  function buildBatch(targets, status) {
    const items = (Array.isArray(targets) ? targets : []).map(normalize);
    return {
      payload: {
        repos: items.map((i) => i.repo),
        numbers: items.map((i) => i.number),
        statuses: items.map(() => status),
      },
      snapshot: items,
    };
  }

  // 撤销要逐条写回编辑前的状态，不能像标记那样统一填一个值
  function buildUndo(snapshot) {
    const items = (Array.isArray(snapshot) ? snapshot : []).map(normalize);
    return {
      repos: items.map((i) => i.repo),
      numbers: items.map((i) => i.number),
      statuses: items.map((i) => i.status),
    };
  }

  // 撤销现场 = 操作快照 + 编辑时的选中集合。主人明确「选择状态回滚、操作回滚」两者都要，
  // 所以封存在同一个对象里，避免以后只留一边。两处都做拷贝，防止后续改动污染现场。
  function captureUndo(snapshot, selection) {
    return {
      snapshot: (Array.isArray(snapshot) ? snapshot : []).map(normalize),
      selection: Array.from(selection || []),
    };
  }

  // 严格回到当时的选中集合：先清空当前选择再按这份结果重建，不与期间新勾的卡片叠加。
  // 轮询/删除后已不存在的卡片要剪掉，否则选中数虚高、批量操作会带上幽灵目标。
  function restoreSelection(selection, validKeys) {
    const valid = new Set(Array.from(validKeys || []));
    const seen = new Set();
    const out = [];
    Array.from(selection || []).forEach(function (key) {
      if (valid.has(key) && !seen.has(key)) {
        seen.add(key);
        out.push(key);
      }
    });
    return out;
  }

  // 多选复制：把选择顺序（Set 插入顺序）还原成后端的 NoticeKey 列表。
  // keyOf 由调用方传入，避免在这里复制一份 entityKey 规则导致漂移。
  function orderedNoticeKeys(selection, events, keyOf) {
    const byKey = new Map();
    for (const ev of Array.isArray(events) ? events : []) {
      const key = keyOf(ev);
      if (!byKey.has(key)) byKey.set(key, ev);
    }
    const out = [];
    for (const key of Array.from(selection || [])) {
      const ev = byKey.get(key);
      if (!ev) continue; // 轮询/删除后失效的卡不该变成幽灵目标
      out.push({ repo: ev.repo, number: ev.number ?? null });
    }
    return out;
  }

  // 右键未选中的卡按主人拍板只操作该卡、选区原样保留，避免误伤别的已选卡
  function contextMenuScope(ev, selection, events, keyOf, multiSelect) {
    const chosen = new Set(Array.from(selection || []));
    const batch = !!multiSelect && chosen.has(keyOf(ev));
    const targets = batch
      ? (Array.isArray(events) ? events : []).filter((item) => chosen.has(keyOf(item)))
      : [ev];
    // 选区在轮询/删除后全失效时退回单卡，别让菜单显示批量却什么都不做
    return targets.length > 0 ? { batch, targets } : { batch: false, targets: [ev] };
  }

  // 选区字段混杂时返回 undefined，菜单的 ✓ 与置顶文案据此不冒充整批状态
  function uniformValue(targets, pick) {
    const items = Array.isArray(targets) ? targets : [];
    if (items.length === 0) return undefined;
    const first = pick(items[0]);
    return items.every((item) => pick(item) === first) ? first : undefined;
  }

  const api = {
    normalize,
    buildBatch,
    buildUndo,
    captureUndo,
    restoreSelection,
    orderedNoticeKeys,
    contextMenuScope,
    uniformValue,
  };
  root.GhrBatch = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
