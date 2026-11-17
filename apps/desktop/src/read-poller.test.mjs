import test from "node:test";
import assert from "node:assert/strict";
import { createReadPoller } from "./read-poller.ts";

const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
function setup(read, options = {}) {
  const values = [], errors = [], busy = [];
  const poller = createReadPoller({ read, onValue: v => values.push(v), onError: e => errors.push(e), onBusy: v => busy.push(v), ...options });
  return { ...poller, values, errors, busy };
}

test("refresh bursts queue one read and never overlap", async () => {
  const pending = [];
  const loop = setup(() => new Promise(resolve => pending.push(resolve)));
  const first = loop.refresh();
  await Promise.all(Array.from({ length: 20 }, () => loop.refresh()));
  assert.equal(pending.length, 1);
  pending[0]("first");
  await first;
  assert.equal(pending.length, 2);
  pending[1]("latest");
  await flush();
  assert.deepEqual(loop.values, ["first", "latest"]);
  loop.stop();
});

test("read failures retry with backoff and success restores the interval", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let calls = 0;
  const loop = setup(async () => { if (++calls <= 2) throw new Error("offline"); return calls; });
  await loop.refresh();
  assert.equal(loop.errors.length, 1);
  t.mock.timers.tick(5000); await flush();
  assert.equal(loop.errors.length, 2);
  t.mock.timers.tick(9999); await flush();
  assert.equal(calls, 2);
  t.mock.timers.tick(1); await flush();
  assert.deepEqual(loop.values, [3]);
  t.mock.timers.tick(5000); await flush();
  assert.deepEqual(loop.values, [3, 4]);
  loop.stop();
});

test("hidden pages make no scheduled or event-triggered reads; showing refreshes", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let visible = true, calls = 0;
  const loop = setup(async () => ++calls, { visible: () => visible });
  await loop.refresh();
  visible = false; loop.visibilityChanged();
  t.mock.timers.tick(60000); await loop.refresh();
  assert.equal(calls, 1);
  visible = true; loop.visibilityChanged(); await flush();
  assert.equal(calls, 2);
  loop.stop();
});

test("persistent failures cap retry spacing at thirty seconds", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let calls = 0;
  const loop = setup(async () => { calls++; throw new Error("offline"); });
  await loop.refresh();
  for (const delay of [5000, 10000, 20000, 30000, 30000, 30000]) {
    const before = calls;
    t.mock.timers.tick(delay - 1); await flush();
    assert.equal(calls, before);
    t.mock.timers.tick(1); await flush();
    assert.equal(calls, before + 1);
  }
  loop.stop();
});

test("stopping during an outstanding read drops late results and queued work", async () => {
  let resolve;
  const loop = setup(() => new Promise(done => { resolve = done; }));
  const first = loop.refresh();
  await loop.refresh();
  loop.stop();
  resolve("late"); await first;
  assert.deepEqual(loop.values, []);
  assert.deepEqual(loop.busy, [true]);
  await loop.refresh();
  assert.deepEqual(loop.values, []);
});

test("visibility changes cannot start concurrent reads", async () => {
  const pending = [];
  let visible = true;
  const loop = setup(() => new Promise(resolve => pending.push(resolve)), { visible: () => visible });
  const first = loop.refresh();
  visible = false; loop.visibilityChanged();
  visible = true; loop.visibilityChanged();
  assert.equal(pending.length, 1);
  pending[0](1); await first;
  assert.equal(pending.length, 2);
  loop.stop(); pending[1](2); await flush();
  assert.deepEqual(loop.values, [1]);
});
