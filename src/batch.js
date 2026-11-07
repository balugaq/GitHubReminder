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

  const api = { normalize, buildBatch, buildUndo };
  root.GhrBatch = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
