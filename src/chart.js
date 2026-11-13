// 统计图表纯逻辑：把 {repo: count} 变成饼图/柱状图需要的几何数据。
// 无 DOM 依赖，浏览器挂 window.GhrChart，Node 单测用 vm 加载同一份源码。
(function (root) {
  "use strict";

  var TAU = Math.PI * 2;

  function summarize(counts, styles) {
    var entries = Object.entries(counts || {}).filter(function (pair) {
      return Number(pair[1]) > 0;
    });
    var total = entries.reduce(function (sum, pair) {
      return sum + Number(pair[1]);
    }, 0);
    var slices = entries
      .map(function (pair) {
        var repo = pair[0];
        return {
          repo: repo,
          name: repo.indexOf("/") >= 0 ? repo.split("/").pop() : repo,
          count: Number(pair[1]),
          share: total > 0 ? Number(pair[1]) / total : 0,
          color: (styles && styles[repo] && styles[repo].color) || "#26c281",
        };
      })
      .sort(function (a, b) {
        return b.count - a.count || a.repo.localeCompare(b.repo);
      });
    var acc = 0;
    for (var i = 0; i < slices.length; i++) {
      var s = slices[i];
      s.start = acc;
      acc += s.share;
      s.end = acc;
      s.angle = ((s.start + s.end) / 2) * TAU - Math.PI / 2;
    }
    return { total: total, slices: slices };
  }

  function polar(cx, cy, r, angle) {
    return {
      x: cx + r * Math.cos(angle),
      y: cy + r * Math.sin(angle),
    };
  }

  // 12 点方向起笔顺时针；整圆要拆两段弧，单段 A 画不出 360°
  function slicePath(slice, cx, cy, r) {
    var a0 = slice.start * TAU - Math.PI / 2;
    var a1 = slice.end * TAU - Math.PI / 2;
    var p0 = polar(cx, cy, r, a0);
    var p1 = polar(cx, cy, r, a1);
    var fmt = function (p) {
      return p.x.toFixed(2) + " " + p.y.toFixed(2);
    };
    if (slice.share >= 1 - 1e-9) {
      var half = polar(cx, cy, r, a0 + Math.PI);
      return (
        "M " + cx + " " + cy + " L " + fmt(p0) +
        " A " + r + " " + r + " 0 1 1 " + fmt(half) +
        " A " + r + " " + r + " 0 1 1 " + fmt(p0) + " Z"
      );
    }
    var large = slice.end - slice.start > 0.5 ? 1 : 0;
    return (
      "M " + cx + " " + cy + " L " + fmt(p0) +
      " A " + r + " " + r + " 0 " + large + " 1 " + fmt(p1) + " Z"
    );
  }

  // 引出线折线 + 标签锚点；同一侧的标签按 y 排序后做最小间距推挤，避免叠字
  function leaderLayout(slices, cx, cy, r, width, height) {
    var LEFT = 9;
    var RIGHT = width - 9;
    var GAP = 13;
    var TOP = 12;
    var BOTTOM = height - 8;
    var groups = { left: [], right: [] };
    var list = Array.isArray(slices) ? slices : [];
    for (var i = 0; i < list.length; i++) {
      var slice = list[i];
      var side = Math.cos(slice.angle) >= 0 ? "right" : "left";
      groups[side].push({
        slice: slice,
        side: side,
        start: polar(cx, cy, r, slice.angle),
        elbow: polar(cx, cy, r + 14, slice.angle),
      });
    }
    var out = [];
    ["left", "right"].forEach(function (side) {
      var items = groups[side].sort(function (a, b) {
        return a.elbow.y - b.elbow.y;
      });
      var last = TOP - GAP;
      for (var j = 0; j < items.length; j++) {
        items[j].labelY = Math.max(items[j].elbow.y, last + GAP);
        last = items[j].labelY;
      }
      var overflow = last - BOTTOM;
      if (overflow > 0) {
        for (var k = 0; k < items.length; k++) items[k].labelY -= overflow;
      }
      for (var m = 0; m < items.length; m++) {
        var item = items[m];
        var endX = side === "right" ? RIGHT : LEFT;
        item.endX = endX;
        item.anchor = side === "right" ? "end" : "start";
        item.line =
          "M " + item.start.x.toFixed(2) + " " + item.start.y.toFixed(2) +
          " L " + item.elbow.x.toFixed(2) + " " + item.elbow.y.toFixed(2) +
          " L " + endX + " " + item.labelY.toFixed(2);
      }
      out.push.apply(out, items);
    });
    return out;
  }

  // 柱状图沿用饼图的排序与配色，返回每根柱子相对最高柱的比例
  function barModel(slices, maxBars) {
    var list = Array.isArray(slices) ? slices : [];
    var shown = list.slice(0, maxBars > 0 ? maxBars : list.length);
    var max = shown.reduce(function (m, s) {
      return Math.max(m, s.count);
    }, 0);
    return {
      max: max,
      bars: shown.map(function (s) {
        return {
          repo: s.repo,
          name: s.name,
          color: s.color,
          count: s.count,
          ratio: max > 0 ? s.count / max : 0,
        };
      }),
    };
  }

  var api = {
    summarize: summarize,
    polar: polar,
    slicePath: slicePath,
    leaderLayout: leaderLayout,
    barModel: barModel,
  };
  root.GhrChart = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
