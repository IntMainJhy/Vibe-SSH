import assert from "node:assert/strict";
import { test } from "node:test";
import { openText, saveText } from "./text-format.ts";

test("editing CRLF and BOM text preserves the original wire format", () => {
  const source = "\uFEFF部署=完成🚀\r\n空行\r\n";
  const {value,format} = openText(source);
  assert.equal(value,"部署=完成🚀\n空行\n");
  assert.equal(saveText(value,format),source);
  assert.equal(saveText(value+"最后一行\n",format),source+"最后一行\r\n");
});
test("UTF-8 and mixed line endings never silently change the remote file", () => {
  const source="中文🚀\n";const parsed=openText(source);
  assert.equal(saveText(parsed.value,parsed.format),source);
  for (const text of ["a\r\nb\n","a\rb"]) {
    const parsed=openText(text);assert.equal(parsed.format.mixed,true);
    assert.throws(()=>saveText(parsed.value,parsed.format));
  }
  assert.equal(Buffer.byteLength("部署🚀"),10);
});
