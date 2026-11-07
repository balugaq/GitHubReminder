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
