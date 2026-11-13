import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

// batch.js 是 classic script，用 vm 加载线上同一份源码，不依赖 DOM/TAURI。
const sandbox = {};
vm.createContext(sandbox);
vm.runInContext(readFileSync(new URL("../src/batch.js", import.meta.url), "utf8"), sandbox);
const B = sandbox.GhrBatch;

// vm 沙箱内建数组原型与主 realm 不同，断言前拷回主 realm。
const plain = (v) => Array.from(v);

// fixture 一律通用示例
const card = (over = {}) => ({
  repo: "octocat/hello-world",
  number: 1,
  status: "unread",
  ...over,
});

test("批量 payload 三个数组等长，状态统一覆盖", () => {
  const { payload, snapshot } = B.buildBatch(
    [card(), card({ number: 2, status: "done" })],
    "in_progress"
  );
  assert.deepEqual(plain(payload.repos), ["octocat/hello-world", "octocat/hello-world"]);
  assert.deepEqual(plain(payload.numbers), [1, 2]);
  assert.deepEqual(plain(payload.statuses), ["in_progress", "in_progress"]);
  assert.equal(payload.repos.length, payload.numbers.length);
  assert.equal(payload.repos.length, payload.statuses.length);
  assert.deepEqual(plain(snapshot.map((s) => s.status)), ["unread", "done"]);
});

test("撤销 payload 逐条恢复快照里不同的状态", () => {
  const { snapshot } = B.buildBatch(
    [card(), card({ number: 2, status: "done" })],
    "read"
  );
  const undo = B.buildUndo(snapshot);
  assert.deepEqual(plain(undo.statuses), ["unread", "done"]);
  assert.deepEqual(plain(undo.numbers), [1, 2]);
  assert.equal(undo.repos.length, undo.statuses.length);
});

test("release 卡片 number 为 null 时保留 null", () => {
  const { payload } = B.buildBatch([card({ number: null })], "done");
  assert.deepEqual(plain(payload.numbers), [null]);
  assert.equal(payload.numbers[0], null);
});

test("缺 status 的旧实体按未读处理", () => {
  const { snapshot } = B.buildBatch([{ repo: "owner/repo", number: 123 }], "read");
  assert.equal(snapshot[0].status, "unread");
  assert.equal(snapshot[0].repo, "owner/repo");
});

test("空选择产出空数组，撤销空快照也不炸", () => {
  const { payload, snapshot } = B.buildBatch([], "done");
  assert.deepEqual(plain(payload.repos), []);
  assert.deepEqual(plain(payload.numbers), []);
  assert.deepEqual(plain(payload.statuses), []);
  assert.deepEqual(plain(snapshot), []);
  assert.deepEqual(plain(B.buildUndo(null).statuses), []);
});

// 主人 2026-11-07 定的撤销语义：选择状态回滚 + 操作回滚，两者都要留现场。
test("captureUndo 同时封存逐条旧状态与当时的选中集合，且是拷贝", () => {
  const { snapshot } = B.buildBatch(
    [card(), card({ number: 2, status: "done" })],
    "read"
  );
  const selection = new Set(["octocat/hello-world#1", "octocat/hello-world#2"]);
  const state = B.captureUndo(snapshot, selection);
  assert.deepEqual(plain(state.snapshot.map((s) => s.status)), ["unread", "done"]);
  assert.deepEqual(plain(state.selection), [
    "octocat/hello-world#1",
    "octocat/hello-world#2",
  ]);
  assert.deepEqual(plain(B.buildUndo(state.snapshot).statuses), ["unread", "done"]);
  // 现场不该被后续改动污染
  snapshot[0].status = "in_progress";
  selection.add("octocat/hello-world#3");
  assert.equal(state.snapshot[0].status, "unread");
  assert.equal(state.selection.length, 2);
});

test("restoreSelection 精确回到编辑时的选集：剪掉失效卡片、去重、不叠加新选的", () => {
  const valid = new Set(["owner/repo#1", "owner/repo#2", "owner/repo#3"]);
  const restored = B.restoreSelection(
    ["owner/repo#2", "owner/repo#1", "owner/repo#2", "owner/repo#99"],
    valid
  );
  assert.deepEqual(plain(restored), ["owner/repo#2", "owner/repo#1"]);
  // 撤销期间新勾的 owner/repo#3 不在快照里，不该被带回来
  assert.equal(restored.includes("owner/repo#3"), false);
});

test("restoreSelection 空现场 / 空有效集合返回空数组", () => {
  assert.deepEqual(plain(B.restoreSelection(null, ["owner/repo#1"])), []);
  assert.deepEqual(plain(B.restoreSelection(["owner/repo#1"], null)), []);
  assert.deepEqual(plain(B.captureUndo(null, null).selection), []);
});

// ---- 多选复制：按选择顺序还原后端 NoticeKey ----
// vm 沙箱里的对象原型与主 realm 不同，deepEqual 前先做一次 JSON 归一化
const normKeys = (v) => JSON.parse(JSON.stringify(Array.from(v)));

test("orderedNoticeKeys 保持 Set 的选择顺序", () => {
  const keyOf = (ev) => `o/r:${ev.number}`;
  const events = [
    { repo: "o/r", number: 1 },
    { repo: "o/r", number: 2 },
    { repo: "o/r", number: 3 },
  ];
  const selection = new Set(["o/r:3", "o/r:1", "o/r:2"]);
  assert.deepEqual(normKeys(B.orderedNoticeKeys(selection, events, keyOf)), [
    { repo: "o/r", number: 3 },
    { repo: "o/r", number: 1 },
    { repo: "o/r", number: 2 },
  ]);
});

test("orderedNoticeKeys 跳过失效选中项并保留空编号", () => {
  const keyOf = (ev) => `o/r:${ev.number ?? ""}`;
  const events = [{ repo: "o/r", number: 2 }, { repo: "o/r", number: null }];
  const selection = new Set(["o/r:9", "o/r:2", "o/r:"]);
  assert.deepEqual(normKeys(B.orderedNoticeKeys(selection, events, keyOf)), [
    { repo: "o/r", number: 2 },
    { repo: "o/r", number: null },
  ]);
});

test("orderedNoticeKeys 空输入返回空数组", () => {
  const keyOf = (ev) => String(ev.number);
  assert.deepEqual(normKeys(B.orderedNoticeKeys(new Set(), [], keyOf)), []);
  assert.deepEqual(normKeys(B.orderedNoticeKeys(null, null, keyOf)), []);
});

// ---- 右键菜单作用域 ----
const scopeCard = (over = {}) => ({
  repo: "owner/repo",
  number: 123,
  status: "unread",
  pinned: false,
  ...over,
});
const scopeKey = (ev) => `${(ev.repo || "").toLowerCase()}:${ev.number ?? ""}`;
const scopeEvents = [
  scopeCard({ number: 123, status: "unread" }),
  scopeCard({ number: 124, status: "done" }),
  scopeCard({ number: 125, status: "read" }),
];

test("contextMenuScope 右键已选中的卡 → 操作扩展到整个选区", () => {
  const selection = new Set(["owner/repo:123", "owner/repo:124", "owner/repo:99"]);
  const scope = B.contextMenuScope(scopeEvents[0], selection, scopeEvents, scopeKey, true);
  assert.equal(scope.batch, true);
  assert.deepEqual(plain(scope.targets.map((t) => t.number)), [123, 124]);
  // 失效的 99 只是被剪出 targets，选区本身不被改动
  assert.deepEqual(plain(selection), ["owner/repo:123", "owner/repo:124", "owner/repo:99"]);
});

test("contextMenuScope 右键未选中的卡 → 只操作该卡，选区原样保留", () => {
  const selection = new Set(["owner/repo:124"]);
  const scope = B.contextMenuScope(scopeEvents[0], selection, scopeEvents, scopeKey, true);
  assert.equal(scope.batch, false);
  assert.deepEqual(plain(scope.targets.map((t) => t.number)), [123]);
  assert.deepEqual(plain(selection), ["owner/repo:124"]);
});

test("contextMenuScope 非多选状态即使卡在选区里也只操作单卡", () => {
  const selection = new Set(["owner/repo:123", "owner/repo:124"]);
  const scope = B.contextMenuScope(scopeEvents[0], selection, scopeEvents, scopeKey, false);
  assert.equal(scope.batch, false);
  assert.equal(scope.targets.length, 1);
  assert.equal(scope.targets[0].number, 123);
});

test("contextMenuScope 选区全失效时退回单卡，不出现空批量菜单", () => {
  const selection = new Set(["owner/repo:123", "owner/repo:999"]);
  const scope = B.contextMenuScope(scopeEvents[0], selection, [scopeEvents[1]], scopeKey, true);
  assert.equal(scope.batch, false);
  assert.equal(scope.targets.length, 1);
  assert.equal(scope.targets[0].number, 123);
});

test("uniformValue 只在整批一致时返回值，混杂与空输入都不打勾", () => {
  const statusOf = (t) => t.status || "unread";
  assert.equal(B.uniformValue([scopeEvents[0]], statusOf), "unread");
  assert.equal(B.uniformValue(scopeEvents, statusOf), undefined);
  assert.equal(
    B.uniformValue([scopeCard({ status: "done" }), scopeCard({ status: "done" })], statusOf),
    "done"
  );
  assert.equal(B.uniformValue([], statusOf), undefined);
  assert.equal(B.uniformValue(scopeEvents, (t) => !!t.pinned), false);
});
