// 板视图纯逻辑：按状态分列 + 拖放目标判定。无 DOM 依赖，浏览器挂 window.GhrBoard，
// Node 单测用 vm 沙箱加载同一份源码。
(function (root) {
  "use strict";

  var COLUMNS = [
    ["unread", "🔔 未读"],
    ["read", "📖 已读"],
    ["in_progress", "⏳ 处理中"],
    ["done", "✅ 已解决"]
  ];

  // 旧存档可能缺 status，统一按未读归列
  function normalizeStatus(status) {
    for (var i = 0; i < COLUMNS.length; i++) {
      if (COLUMNS[i][0] === status) return status;
    }
    return "unread";
  }

  function groupByStatus(events) {
    var groups = {};
    for (var i = 0; i < COLUMNS.length; i++) groups[COLUMNS[i][0]] = [];
    var list = Array.isArray(events) ? events : [];
    for (var j = 0; j < list.length; j++) {
      var ev = list[j];
      groups[normalizeStatus(ev && ev.status)].push(ev);
    }
    return groups;
  }

  // 拖到非法列或拖回原列都不发请求；返回 null 表示这次拖放该被忽略
  function dropStatus(current, target) {
    if (normalizeStatus(target) !== target) return null;
    return normalizeStatus(current) === target ? null : target;
  }

  var api = {
    COLUMNS: COLUMNS,
    normalizeStatus: normalizeStatus,
    groupByStatus: groupByStatus,
    dropStatus: dropStatus
  };
  root.GhrBoard = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
