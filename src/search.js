// 主界面通知搜索：纯逻辑、无 DOM 依赖。浏览器里挂到 window.GhrSearch（classic script），
// Node 单测用 vm 沙箱直接加载同一份源码，保证测的就是线上跑的那份。
//
// 查询语法：
//   "pr title"           → 标题或作者里同时含 pr / title（分词 AND，忽略大小写）
//   "@octocat pr title"  → 先限定「涉及 octocat」的通知，再在其上按关键词筛
//   "pr title @octocat"  → 同上，@用户名 可放在关键词任意位置
//   "@//octocat/"        → 旧写法，保留兼容
//
// 「涉及」= 该卡片的发起者或指派人（后端 Entity.people 累积，统一小写）；
// 普通关键词只匹配「标题 + author」，不含指派人——两者语义刻意区分，
// 否则 @用户名 就只是普通搜索的别名了。「涉及的人」按设计不含评论者。
(function (root) {
  "use strict";

  // 词首的 @用户名 才是「涉及某人」；@//用户名/ 是旧写法，仍要求结尾斜杠。
  // 前后都卡空白/首尾，避免把 foo@example.com 之类的文本误判成人名。
  var MENTION_RE = /(?:^|\s)@(?:\/\/([A-Za-z0-9][A-Za-z0-9-]*)\/|([A-Za-z0-9][A-Za-z0-9-]*))(?=\s|$)/g;

  function parseQuery(raw) {
    var text = String(raw == null ? "" : raw);
    var people = [];
    // 标记整体从关键词里摘掉，替换成空格，避免剩下的词粘在一起
    var rest = text.replace(MENTION_RE, function (_, legacy, modern) {
      var n = (legacy || modern).toLowerCase();
      if (people.indexOf(n) === -1) people.push(n);
      return " ";
    });
    var trimmed = rest.trim();
    var terms = trimmed === "" ? [] : trimmed.split(/\s+/).map(function (t) {
      return t.toLowerCase();
    });
    return {
      people: people,
      terms: terms,
      active: people.length > 0 || terms.length > 0,
      raw: text.trim()
    };
  }

  // 兼容旧存档：people 缺失时至少还有 author 能判定「涉及」
  function involvedPeople(ev) {
    var names = [];
    if (ev.author) names.push(ev.author);
    if (Array.isArray(ev.people)) names = names.concat(ev.people);
    var out = [];
    for (var i = 0; i < names.length; i++) {
      var n = String(names[i] || "").trim().toLowerCase();
      if (n && out.indexOf(n) === -1) out.push(n);
    }
    return out;
  }

  function matchesQuery(ev, parsed) {
    var q = parsed || parseQuery("");
    var involved = q.people.length ? involvedPeople(ev) : [];
    for (var i = 0; i < q.people.length; i++) {
      if (involved.indexOf(q.people[i]) === -1) return false;
    }
    if (q.terms.length === 0) return true;
    // 普通搜索范围严格限定为标题 + 作者
    var hay = ((ev.title || "") + " " + (ev.author || "")).toLowerCase();
    for (var j = 0; j < q.terms.length; j++) {
      if (hay.indexOf(q.terms[j]) === -1) return false;
    }
    return true;
  }

  function searchEvents(events, raw) {
    var list = Array.isArray(events) ? events : [];
    var parsed = parseQuery(raw);
    if (!parsed.active) return list; // 空查询直接返回原数组，避免无谓拷贝
    return list.filter(function (ev) {
      return matchesQuery(ev, parsed);
    });
  }

  var api = {
    parseQuery: parseQuery,
    involvedPeople: involvedPeople,
    matchesQuery: matchesQuery,
    searchEvents: searchEvents
  };
  root.GhrSearch = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
