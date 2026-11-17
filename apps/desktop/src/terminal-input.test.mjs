import assert from "node:assert/strict";
import { test } from "node:test";
import { inputFrames, inputQueue } from "./terminal-input.ts";

test("a large multilingual paste fits every write budget without changing its content", () => {
  const original = "部署完成🚀\r\n".repeat(10_000);
  const frames = inputFrames(original);
  assert.equal(frames.join(""), original);
  assert.ok(frames.length > 1);
  for (const frame of frames) {
    assert.ok(Buffer.byteLength(frame) <= 16_384);
    assert.ok(!frame.includes("\uFFFD"));
  }
  assert.deepEqual(inputFrames(""), []);
});

test("queued writes stop when control is lost, including after it comes back", async () => {
  let allowed = true,
    unlock;
  const sent = [];
  const queue = inputQueue(() => allowed, assert.fail);
  const first = queue(async () => {
    sent.push("first");
    await new Promise((done) => {
      unlock = done;
    });
  });
  const later = queue(async () => {
    sent.push("queued");
  });
  await Promise.resolve();
  allowed = false;
  unlock();
  await Promise.all([first, later]);
  allowed = true;
  assert.deepEqual(
    sent,
    ["first"],
    "returning control must not replay a skipped write",
  );
});

test("a partially delivered write is never retried and stops everything behind it", async () => {
  const sent = [],
    failures = [];
  const queue = inputQueue(
    () => true,
    (error) => failures.push(error),
  );
  await Promise.all([
    queue(async () => {
      sent.push("first");
      throw new Error("delivery unknown");
    }),
    queue(async () => {
      sent.push("second");
    }),
  ]);
  assert.deepEqual(sent, ["first"]);
  assert.equal(failures.length, 1);
});

test("writes from an old control period stay discarded when ownership changes twice", async () => {
  let generation = 0,
    unlock;
  const sent = [];
  const queue = inputQueue(
    () => true,
    assert.fail,
    () => generation,
  );
  const first = queue(async () => {
    sent.push("first");
    await new Promise((done) => {
      unlock = done;
    });
  });
  const stale = queue(async () => {
    sent.push("stale");
  });
  await Promise.resolve();
  generation += 2; // release then take over again, before the old write can drain
  unlock();
  await Promise.all([first, stale]);
  await queue(async () => {
    sent.push("fresh");
  });
  assert.deepEqual(sent, ["first", "fresh"]);
});
