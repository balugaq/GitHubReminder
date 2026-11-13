import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

// chart.js 是 classic script，用 vm 加载线上同一份源码，不依赖 DOM/SVG。
const sandbox = {};
vm.createContext(sandbox);
vm.runInContext(readFileSync(new URL("../src/chart.js", import.meta.url), "utf8"), sandbox);
const C = sandbox.GhrChart;

// vm 沙箱里的对象原型与主 realm 不同，deepEqual 前先做一次 JSON 归一化
const plain = (v) => JSON.parse(JSON.stringify(Array.from(v)));

// fixture 一律通用示例
test("summarize 算总数与占比，按条数降序", () => {
  const r = C.summarize({ "o/a": 3, "o/b": 1 }, { "o/a": { color: "#f00" } });
  assert.equal(r.total, 4);
  assert.deepEqual(plain(r.slices.map((s) => s.repo)), ["o/a", "o/b"]);
  assert.equal(r.slices[0].share, 0.75);
  assert.equal(r.slices[0].color, "#f00");
  assert.equal(r.slices[1].color, "#26c281", "缺样式走默认色");
  assert.equal(r.slices.reduce((s, x) => s + x.share, 0), 1);
});

test("summarize 过滤 0 条并容忍空输入", () => {
  assert.equal(C.summarize({ "o/a": 0 }, {}).total, 0);
  assert.equal(C.summarize(null, null).total, 0);
  assert.equal(C.summarize(undefined, undefined).slices.length, 0);
});

test("slicePath 整圆拆两段弧，普通扇形按大小弧", () => {
  const one = C.summarize({ "o/a": 5 }, {}).slices[0];
  const d = C.slicePath(one, 260, 160, 100);
  assert.equal((d.match(/ A /g) || []).length, 2, "整圆必须拆两段，否则 A 画不出 360°");
  const two = C.summarize({ "o/a": 3, "o/b": 1 }, {}).slices;
  assert.ok(C.slicePath(two[0], 260, 160, 100).includes(" 0 1 1 "), "75% 是大弧");
  assert.ok(C.slicePath(two[1], 260, 160, 100).includes(" 0 0 1 "), "25% 是小弧");
});

test("polar 角度与半径一致", () => {
  const p = C.polar(100, 100, 50, 0);
  assert.equal(p.x, 150);
  assert.equal(p.y, 100);
});

test("leaderLayout 分左右且同侧标签不叠字", () => {
  const { slices } = C.summarize({ a: 5, b: 4, c: 3, d: 2, e: 1 }, {});
  const items = plain(C.leaderLayout(slices, 260, 160, 100, 520, 320));
  for (const side of ["left", "right"]) {
    const ys = items
      .filter((i) => i.side === side)
      .map((i) => i.labelY)
      .sort((a, b) => a - b);
    for (let i = 1; i < ys.length; i++) {
      assert.ok(ys[i] - ys[i - 1] >= 12.9, `${side} 侧标签间距太小`);
    }
  }
  assert.ok(items.every((i) => i.line.startsWith("M ")));
  assert.ok(
    items.some((i) => i.anchor === "start") && items.some((i) => i.anchor === "end"),
    "左右两侧都应有标签"
  );
});

test("barModel 给出最高柱与比例", () => {
  const { slices } = C.summarize({ a: 6, b: 3, c: 0 }, {});
  const m = C.barModel(slices, 10);
  assert.equal(m.max, 6);
  assert.deepEqual(plain(m.bars.map((b) => b.ratio)), [1, 0.5]);
});

test("barModel 按 maxBars 截断，比例仍以最高柱为基准", () => {
  const { slices } = C.summarize({ a: 9, b: 6, c: 3, d: 1 }, {});
  const m = C.barModel(slices, 2);
  assert.deepEqual(plain(m.bars.map((b) => b.repo)), ["a", "b"]);
  assert.equal(m.max, 9);
  assert.equal(m.bars[1].ratio, 6 / 9);
});
