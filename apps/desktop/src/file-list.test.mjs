import test from "node:test";
import assert from "node:assert/strict";
import { arrangeFiles, fileModified, filePage, FILE_PAGE_SIZE } from "./file-list.ts";

const entry = (name, extra = {}) => ({ name, path: `/目录/${name}`, kind: "file", size: 1, modified: 0, permissions: 0o644, ...extra });

test("untrusted timestamps cannot crash the file browser, and epoch zero is valid", () => {
  assert.notEqual(fileModified(0), "—");
  for (const value of [null, Infinity, NaN, 1e20]) assert.equal(fileModified(value), "—");
});

test("natural name sorting keeps directories first in both directions without mutating input", () => {
  const source = [entry("file-10"), entry("file-2"), entry("文档", { kind: "directory" })];
  assert.deepEqual(arrangeFiles(source, "", false, "name-asc").map(e => e.name), ["文档", "file-2", "file-10"]);
  assert.deepEqual(arrangeFiles(source, "", false, "name-desc").map(e => e.name), ["文档", "file-10", "file-2"]);
  assert.deepEqual(source.map(e => e.name), ["file-10", "file-2", "文档"]);
});

test("size/time sorting treats zero as known and puts unknown metadata last", () => {
  const source = [entry("unknown", { size: null, modified: null }), entry("zero", { size: 0 }), entry("larger", { size: 100, modified: 10 })];
  assert.deepEqual(arrangeFiles(source, "", false, "size-desc").map(e => e.name), ["larger", "zero", "unknown"]);
  assert.deepEqual(arrangeFiles(source, "", false, "modified-asc").map(e => e.name), ["zero", "larger", "unknown"]);
});

test("filtering covers hidden names, Unicode and case without reading content", () => {
  const source = [entry(".配置"), entry("配置.TXT"), entry("other")];
  assert.deepEqual(arrangeFiles(source, "配置", false, "name-asc").map(e => e.name), ["配置.TXT"]);
  assert.equal(arrangeFiles(source, "配置", true, "name-asc").length, 2);
  assert.equal(arrangeFiles(source, ".txt", false, "name-asc")[0].name, "配置.TXT");
});

test("5000 entries remain reachable exactly once in bounded pages; shrinking clamps the page", () => {
  const entries = Array.from({ length: 5000 }, (_, i) => entry(String(i)));
  const seen = [];
  for (let page = 0; page < 50; page++) {
    const current = filePage(entries, page);
    assert.ok(current.entries.length <= FILE_PAGE_SIZE);
    seen.push(...current.entries.map(e => e.path));
  }
  assert.equal(new Set(seen).size, 5000);
  assert.equal(seen.length, 5000);
  assert.equal(filePage(entries.slice(0, 101), 49).page, 1);
  assert.equal(filePage([], 49).page, 0);
  assert.equal(filePage(entries, -1).page, 0);
});
