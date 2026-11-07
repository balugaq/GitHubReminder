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

  const api = { normalize, buildBatch, buildUndo, captureUndo, restoreSelection };
  root.GhrBatch = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
