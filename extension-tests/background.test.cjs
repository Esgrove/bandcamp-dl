const { test } = require("bun:test");
const assert = require("node:assert/strict");
const vm = require("node:vm");
const fs = require("node:fs");
const path = require("node:path");
const { webcrypto } = require("node:crypto");

const source = (name) => fs.readFileSync(path.join(__dirname, "../extension", name), "utf8");
const settle = () => new Promise((resolve) => setImmediate(resolve));
const requestId = "b".repeat(64);

async function background(tabs, responses = []) {
  const calls = [];
  const messages = [];
  const storage = {
    pairing: { port: 47653, token: "a".repeat(64) },
    profileId: "test-profile",
  };
  let tick;
  let click;
  const context = vm.createContext({
    URL, AbortSignal, TypeError, crypto: webcrypto, console,
    setInterval: (callback) => { tick = callback; },
    browser: {
      storage: { local: {
        get: async () => ({ ...storage }),
        set: async (value) => Object.assign(storage, value),
      } },
      tabs: {
        query: async () => tabs,
        sendMessage: async (id, message) => {
          messages.push({ id, type: message.type });
          return { urls: [`https://p4.bcbits.com/download/track/${id}`] };
        },
      },
      browserAction: {
        setTitle: async () => {},
        setBadgeText: async () => {},
        onClicked: { addListener: (callback) => { click = callback; } },
      },
    },
    fetch: async (url, options) => {
      calls.push({ url, options });
      if (responses.length === 0) throw new TypeError("No CLI listener");
      const response = responses.shift();
      return {
        ok: true,
        json: async () => response,
      };
    },
  });
  vm.runInContext(source("extract.js"), context);
  vm.runInContext(source("background.js"), context);
  await settle();
  return { calls, messages, storage, tick: () => tick(), click: (tab) => click(tab) };
}

test("no localhost traffic when no download tab is open", async () => {
  const provider = await background([]);
  await provider.tick();
  assert.equal(provider.calls.length, 0);
});

test("page can be open before CLI; only selected tab is collected on demand", async () => {
  const tabs = [1, 2].map((id) => ({
    id, url: "https://bandcamp.com/download?sig=private", title: `Purchase ${id}`,
  }));
  const responses = [];
  const provider = await background(tabs, responses);
  assert.match(provider.storage.status, /waiting for bcdl/);
  responses.push({ version: 1, request_id: requestId, action: "list" }, null);
  await provider.tick();
  const listed = JSON.parse(provider.calls.at(-1).options.body);
  assert.deepEqual(listed.tabs.map((tab) => tab.id), [1, 2]);
  assert.equal(provider.messages.length, 0);
  assert.equal(JSON.stringify(provider.calls).includes("sig=private"), false);
  responses.push({ version: 1, request_id: requestId, action: "collect", tab_id: 2 }, null);
  await provider.tick();
  assert.deepEqual(provider.messages, [{ id: 2, type: "bcdl:collect" }]);
  const links = JSON.parse(provider.calls.at(-1).options.body);
  assert.deepEqual(links.urls, ["https://p4.bcbits.com/download/track/2"]);
  assert.equal(links.request_id, requestId);
  assert.equal(provider.calls.at(-1).options.headers["X-BCDL-Token"], "a".repeat(64));
  assert.equal(provider.calls.some((call) => call.url.includes("a".repeat(64))), false);
  responses.push({ version: 1, request_id: requestId, action: "collect", tab_id: 2 });
  await provider.tick();
  assert.equal(provider.messages.length, 1, "same request must not be sent twice");
});

test("toolbar action copies without starting a download", async () => {
  const provider = await background([]);
  await provider.click({ id: 3, url: "https://bandcamp.com/download?sig=private" });
  assert.deepEqual(provider.messages, [{ id: 3, type: "bcdl:copy" }]);
  assert.match(provider.storage.status, /Copied 1 links/);
  assert.equal(provider.calls.length, 0);
});

test("unknown bridge commands produce an explicit error and no collection", async () => {
  const provider = await background(
    [{ id: 1, url: "https://bandcamp.com/download" }],
    [{ version: 1, request_id: requestId, action: "execute" }],
  );
  assert.match(provider.storage.status, /Unsupported browser bridge response/);
  assert.equal(provider.messages.length, 0);
});

test("content script uses the real extraction helper and clipboard only on request", async () => {
  let onMessage;
  let clipboard;
  const context = vm.createContext({
    URL,
    document: { querySelectorAll: () => [{ href: "https://p4.bcbits.com/download/album/fixture?sig=a%2Bb" }] },
    location: { href: "https://bandcamp.com/download" },
    navigator: { clipboard: { writeText: async (text) => { clipboard = text; } } },
    browser: { runtime: {
      id: "bcdl@test",
      onMessage: { addListener: (callback) => { onMessage = callback; } },
    } },
  });
  vm.runInContext(source("extract.js"), context);
  vm.runInContext(source("content.js"), context);
  assert.equal(onMessage({ type: "bcdl:collect" }, { id: "other" }), undefined);
  const result = await onMessage({ type: "bcdl:collect" }, { id: "bcdl@test" });
  assert.equal(result.urls.length, 1);
  assert.equal(clipboard, undefined);
  await onMessage({ type: "bcdl:copy" }, { id: "bcdl@test" });
  assert.deepEqual(JSON.parse(clipboard), Array.from(result.urls));
});
