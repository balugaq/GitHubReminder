// 项目外观 / bot 关键词纯逻辑：设置页与主窗口共用，无 DOM 依赖。
// 浏览器挂 window.GhrProject，Node 单测用 vm 加载同一份源码。
(function (root) {
  "use strict";

  var HEX = /^#[0-9a-fA-F]{6}$/;
  var DEFAULT_COLOR = "#26c281";

  function isHexColor(value) {
    return typeof value === "string" && HEX.test(value.trim());
  }

  // 关键词列表：去首尾空白、丢空项、忽略大小写去重，保留用户写的原始大小写与顺序。
  // 与 Rust set_bot_keywords 的清洗口径保持一致，避免界面回显和落盘结果不一致。
  function normalizeKeywords(list) {
    var seen = new Set();
    var out = [];
    (Array.isArray(list) ? list : []).forEach(function (item) {
      var word = String(item == null ? "" : item).trim();
      var low = word.toLowerCase();
      if (!word || seen.has(low)) return;
      seen.add(low);
      out.push(word);
    });
    return out;
  }

  // 旧存档里项目名大小写可能和事件 repo 不一致：先精确命中，再忽略大小写兜底。
  function pickStyle(styles, repo) {
    if (!styles || !repo) return null;
    if (Object.prototype.hasOwnProperty.call(styles, repo)) return styles[repo];
    var low = String(repo).toLowerCase();
    var keys = Object.keys(styles);
    for (var i = 0; i < keys.length; i++) {
      if (keys[i].toLowerCase() === low) return styles[keys[i]];
    }
    return null;
  }

  // 脏值不能进 CSS 变量，否则整张卡会被浏览器按继承色处理成默认黑。
  function textColorOf(styles, repo) {
    var style = pickStyle(styles, repo);
    var value = style && style.textColor;
    return isHexColor(value) ? value.trim() : null;
  }

  function projectColorOf(styles, repo) {
    var style = pickStyle(styles, repo);
    var value = style && style.color;
    return isHexColor(value) ? value.trim() : DEFAULT_COLOR;
  }

  function sortedRepos(styles) {
    return Object.keys(styles || {}).sort(function (a, b) {
      return a.localeCompare(b);
    });
  }

  var api = {
    isHexColor: isHexColor,
    normalizeKeywords: normalizeKeywords,
    pickStyle: pickStyle,
    textColorOf: textColorOf,
    projectColorOf: projectColorOf,
    sortedRepos: sortedRepos,
  };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  root.GhrProject = api;
})(typeof window !== "undefined" ? window : globalThis);
