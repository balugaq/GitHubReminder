import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

// search.js 是浏览器 classic script（无 import/export），用 vm 沙箱加载同一份源码，
// 保证测的就是主界面实际运行的那份逻辑，不需要额外打包或 DOM 环境。
const sandbox = {};
vm.createContext(sandbox);
vm.runInContext(readFileSync(new URL("../src/search.js", import.meta.url), "utf8"), sandbox);
const S = sandbox.GhrSearch;

// vm 沙箱内建的数组原型与主 realm 不同，deepStrictEqual 会误报「结构相同但非引用相等」。
// 断言前统一拷回主 realm。
const plain = (v) => Array.from(v);

// fixture 一律通用示例：octocat / hubot / example.com / 示例标题
const card = (over = {}) => ({
  repo: "octocat/hello-world",
  number: 1,
  title: "fix: 示例标题 PR",
  author: "octocat",
  people: ["octocat", "hubot"],
  url: "https://example.com/octocat/hello-world/pull/1",
  ...over,
});

test("空查询不过滤，返回原数组", () => {
  const list = [card(), card({ number: 2 })];
  assert.equal(S.searchEvents(list, ""), list);
  assert.equal(S.parseQuery("   ").active, false);
  assert.equal(S.parseQuery(null).active, false);
});

test("普通关键词搜标题与作者，大小写不敏感", () => {
  const list = [
    card({ number: 1, title: "fix: 示例标题 PR", author: "octocat" }),
    card({ number: 2, title: "无关内容", author: "hubot" }),
    card({ number: 3, title: "完全不相干", author: "alice" }),
  ];
  assert.deepEqual(plain(S.searchEvents(list, "示例").map((e) => e.number)), [1]);
  assert.deepEqual(plain(S.searchEvents(list, "OCTOCAT").map((e) => e.number)), [1]);
  assert.deepEqual(plain(S.searchEvents(list, "HUBOT").map((e) => e.number)), [2]);
  assert.deepEqual(plain(S.searchEvents(list, "pr").map((e) => e.number)), [1]);
});

test("普通关键词多词是 AND，且只在标题+作者里搜", () => {
  const list = [
    card({ number: 1, title: "pr title" }),
    card({ number: 2, title: "pr only" }),
  ];
  assert.deepEqual(plain(S.searchEvents(list, "pr title").map((e) => e.number)), [1]);
  // hubot 只是 assignee（在 people 里），普通关键词不该命中
  assert.deepEqual(plain(S.searchEvents(list, "hubot")), []);
});

test("@//用户名/ 解析为人名，剩余文本作关键词", () => {
  const q = S.parseQuery(" @//octocat/ pr title ");
  assert.deepEqual(plain(q.people), ["octocat"]);
  assert.deepEqual(plain(q.terms), ["pr", "title"]);
  assert.equal(q.active, true);
});

test("涉及的人包含指派人，人名大小写不敏感", () => {
  const list = [
    card({ number: 1, title: "pr title" }),
    card({
      number: 2,
      title: "pr title",
      author: "someone-else",
      people: ["someone-else"],
    }),
  ];
  assert.deepEqual(plain(S.searchEvents(list, "@//hubot/ pr title").map((e) => e.number)), [1]);
  assert.deepEqual(plain(S.searchEvents(list, "@//OCTOCAT/ pr").map((e) => e.number)), [1]);
  assert.deepEqual(plain(S.searchEvents(list, "@//nobody/ pr title")), []);
});

test("人名与关键词叠加；只有人名时仅按人筛", () => {
  const list = [
    card({ number: 1, title: "pr title" }),
    card({ number: 2, title: "无关内容" }),
  ];
  assert.deepEqual(plain(S.searchEvents(list, "@//octocat/").map((e) => e.number)), [1, 2]);
  assert.deepEqual(plain(S.searchEvents(list, "@//octocat/ title").map((e) => e.number)), [1]);
});

test("多个人名是 AND", () => {
  const list = [
    card({ number: 1, people: ["octocat", "hubot"] }),
    card({ number: 2, people: ["octocat"], author: "octocat" }),
  ];
  assert.deepEqual(plain(S.searchEvents(list, "@//octocat/ @//hubot/").map((e) => e.number)), [1]);
});

test("未闭合的标记当普通文本，不误判为人名", () => {
  const q = S.parseQuery("@//octocat pr title");
  assert.deepEqual(plain(q.people), []);
  assert.ok(plain(q.terms).includes("@//octocat"));
  assert.deepEqual(plain(S.searchEvents([card()], "@//octocat pr title")), []);
});

test("兼容旧存档：people 缺失时 author 仍能判「涉及」", () => {
  const legacy = { title: "示例标题", author: "octocat" };
  assert.deepEqual(plain(S.involvedPeople(legacy)), ["octocat"]);
  assert.equal(S.matchesQuery(legacy, S.parseQuery("@//octocat/ 示例")), true);
});

test("空 author / 空 people 的旧卡片不崩", () => {
  const legacy = { title: "示例标题" };
  assert.equal(S.searchEvents([legacy], "示例").length, 1);
  assert.deepEqual(plain(S.searchEvents([legacy], "@//octocat/")), []);
});
