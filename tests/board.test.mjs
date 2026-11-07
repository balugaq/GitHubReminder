import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

// board.js 是 classic script，用 vm 加载线上同一份源码，不依赖 DOM/TAURI。
const sandbox = {};
vm.createContext(sandbox);
vm.runInContext(readFileSync(new URL("../src/board.js", import.meta.url), "utf8"), sandbox);
const B = sandbox.GhrBoard;

// vm 沙箱内建数组原型与主 realm 不同，断言前拷回主 realm。
const plain = (v) => Array.from(v);

// fixture 一律通用示例
const card = (over = {}) => ({
  repo: "octocat/hello-world",
  number: 1,
  title: "示例标题",
  status: "unread",
  ...over,
});

test("四列都建出来，卡片按状态归到对应列", () => {
  const groups = B.groupByStatus([
    card({ number: 1, status: "unread" }),
    card({ number: 2, status: "done" }),
    card({ number: 3, status: "in_progress" }),
    card({ number: 4, status: "read" }),
  ]);
  assert.deepEqual(Object.keys(groups), plain(B.COLUMNS.map((c) => c[0])));
  assert.deepEqual(plain(groups.unread.map((e) => e.number)), [1]);
  assert.deepEqual(plain(groups.read.map((e) => e.number)), [4]);
  assert.deepEqual(plain(groups.in_progress.map((e) => e.number)), [3]);
  assert.deepEqual(plain(groups.done.map((e) => e.number)), [2]);
});

test("缺 status 与未知状态都归未读", () => {
  const groups = B.groupByStatus([
    card({ number: 1, status: undefined }),
    card({ number: 2, status: "archived" }),
    card({ number: 3, status: "done" }),
  ]);
  assert.deepEqual(plain(groups.unread.map((e) => e.number)), [1, 2]);
  assert.deepEqual(plain(groups.done.map((e) => e.number)), [3]);
});

test("空输入也返回四个空列", () => {
  for (const input of [null, undefined, []]) {
    const groups = B.groupByStatus(input);
    assert.equal(Object.keys(groups).length, 4);
    for (const [key] of B.COLUMNS) assert.deepEqual(plain(groups[key]), []);
  }
});

test("拖到其它列给出目标状态，拖回原列和非法列都忽略", () => {
  assert.equal(B.dropStatus("unread", "done"), "done");
  assert.equal(B.dropStatus("done", "unread"), "unread");
  assert.equal(B.dropStatus(undefined, "read"), "read");
  assert.equal(B.dropStatus("read", "read"), null);
  assert.equal(B.dropStatus("unread", "archived"), null);
  assert.equal(B.dropStatus("unread", undefined), null);
});

test("四列顺序与状态常量一致", () => {
  assert.deepEqual(plain(B.COLUMNS.map((c) => c[0])), [
    "unread",
    "read",
    "in_progress",
    "done",
  ]);
  for (const c of B.COLUMNS) assert.equal(typeof c[1], "string");
});
