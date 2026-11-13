import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

// project.js 是 classic script，用 vm 加载线上同一份源码，不依赖 DOM。
const sandbox = {};
vm.createContext(sandbox);
vm.runInContext(readFileSync(new URL("../src/project.js", import.meta.url), "utf8"), sandbox);
const P = sandbox.GhrProject;

// vm 沙箱里的数组原型与主 realm 不同，deepEqual 前先做一次 JSON 归一化
const plain = (v) => JSON.parse(JSON.stringify(v));

// fixture 一律通用示例
test("normalizeKeywords 去空白/空项并按大小写去重，保留顺序", () => {
  const r = P.normalizeKeywords([
    "  dependabot[bot] ",
    "",
    "  ",
    "renovate[bot]",
    "DEPENDABOT[BOT]",
    null,
    "renovate[bot] extra",
  ]);
  assert.deepEqual(plain(r), ["dependabot[bot]", "renovate[bot]", "renovate[bot] extra"]);
  assert.deepEqual(plain(P.normalizeKeywords(null)), []);
  assert.deepEqual(plain(P.normalizeKeywords(undefined)), []);
});

test("pickStyle 精确命中优先，大小写不一致时回退", () => {
  const styles = { "owner/repo": { color: "#111111", textColor: "#eeeeee" } };
  assert.equal(P.pickStyle(styles, "owner/repo").textColor, "#eeeeee");
  assert.equal(P.pickStyle(styles, "OWNER/REPO").textColor, "#eeeeee", "旧存档大小写漂移可回退");
  assert.equal(P.pickStyle(styles, "owner/other"), null);
  assert.equal(P.pickStyle(null, "owner/repo"), null);
  assert.equal(P.pickStyle(styles, ""), null);
});

test("textColorOf 只放行合法 hex，脏值返回 null", () => {
  const styles = {
    "owner/a": { color: "#26c281", textColor: "#ffcc00" },
    "owner/b": { color: "#26c281", textColor: "not-a-color" },
    "owner/c": { color: "#26c281" },
  };
  assert.equal(P.textColorOf(styles, "owner/a"), "#ffcc00");
  assert.equal(P.textColorOf(styles, "owner/b"), null);
  assert.equal(P.textColorOf(styles, "owner/c"), null);
  assert.equal(P.textColorOf(styles, "owner/missing"), null);
});

test("projectColorOf 脏值/缺样式回退默认色，sortedRepos 稳定排序", () => {
  const styles = {
    "owner/b": { color: "#123456" },
    "owner/a": { color: "bad" },
    "owner/C": { color: "#abcdef" },
  };
  assert.equal(P.projectColorOf(styles, "owner/b"), "#123456");
  assert.equal(P.projectColorOf(styles, "owner/a"), "#26c281");
  assert.equal(P.projectColorOf(styles, "owner/missing"), "#26c281");
  assert.deepEqual(plain(P.sortedRepos(styles)), ["owner/a", "owner/b", "owner/C"]);
  assert.deepEqual(plain(P.sortedRepos(null)), []);
});

test("isHexColor 只接受 6 位十六进制", () => {
  assert.equal(P.isHexColor("#AABBCC"), true);
  assert.equal(P.isHexColor("#abc"), false);
  assert.equal(P.isHexColor("rgb(1,2,3)"), false);
  assert.equal(P.isHexColor(null), false);
});
