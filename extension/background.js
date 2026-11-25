"use strict";

let busy = false;
let profileId;
const completed = new Set();
let lastStatus;

async function status(text, badge = "") {
  if (text === lastStatus) return;
  lastStatus = text;
  await browser.storage.local.set({ status: text });
  await browser.browserAction.setTitle({ title: `${text}\nClick to copy download links.` });
  await browser.browserAction.setBadgeText({ text: badge });
}

async function collect(tabId, type) {
  try {
    const result = await browser.tabs.sendMessage(tabId, { type });
    if (!result || (!Array.isArray(result.urls) && typeof result.error !== "string")) {
      throw new Error("Invalid content-script response");
    }
    return result;
  } catch {
    return { error: "Cannot read this tab. Reload the Bandcamp download page and retry." };
  }
}

async function poll() {
  if (busy) return;
  busy = true;
  try {
    const tabs = (await browser.tabs.query({ url: "https://bandcamp.com/download*" }))
      .filter((tab) => Bcdl.isDownloadPage(tab.url))
      .sort((a, b) => a.id - b.id);
    if (tabs.length === 0) return;
    const { pairing } = await browser.storage.local.get("pairing");
    if (!pairing) {
      await status("Run bcdl --browser-setup and enter pairing in extension settings.", "!");
      return;
    }
    async function call(path, body) {
      const response = await fetch(`http://127.0.0.1:${pairing.port}/v1/${path}`, {
        method: body ? "POST" : "GET",
        headers: {
          "X-BCDL-Token": pairing.token,
          ...(body ? { "Content-Type": "application/json" } : {}),
        },
        ...(body ? { body: JSON.stringify(body) } : {}),
        cache: "no-store",
        credentials: "omit",
        referrerPolicy: "no-referrer",
        signal: AbortSignal.timeout(3000),
      });
      if (!response.ok) {
        throw new Error(response.status === 403
          ? "Pairing rejected. Check the CLI and extension pairing settings."
          : `Browser bridge returned HTTP ${response.status}. Retry the CLI command.`);
      }
      return response;
    }
    let response;
    try {
      response = await call(`request?profile_id=${encodeURIComponent(profileId)}`);
    } catch (error) {
      if (error instanceof TypeError || error.name === "TimeoutError") {
        await status("Ready; waiting for bcdl --browser.");
        return;
      }
      throw error;
    }
    const request = await response.json();
    if (request.version !== 1 || !/^[0-9a-f]{64}$/.test(request.request_id)
        || !["list", "collect", "idle"].includes(request.action)
        || (request.action === "collect" && !Number.isInteger(request.tab_id))) {
      throw new Error("Unsupported browser bridge response. Update the CLI and extension.");
    }
    const common = { request_id: request.request_id, profile_id: profileId };
    if (request.action === "list") {
      await call("tabs", {
        ...common,
        tabs: tabs.map((tab) => ({
          id: tab.id,
          title: (tab.title || "Bandcamp download page")
            .replace(/https?:\/\/\S+/gi, "[link]").slice(0, 120),
        })),
      });
      await status("Connected; choose a tab in the terminal if prompted.");
    } else if (request.action === "collect" && !completed.has(request.request_id)) {
      const tab = tabs.find((candidate) => candidate.id === request.tab_id);
      const result = tab
        ? await collect(tab.id, "bcdl:collect")
        : { error: "The selected tab was closed. Retry the CLI command." };
      await call("links", {
        ...common,
        tab_id: request.tab_id,
        urls: result.urls || [],
        error: result.error || null,
      });
      completed.add(request.request_id);
      if (completed.size > 16) completed.delete(completed.values().next().value);
      await status(result.error || `Sent ${result.urls.length} links; follow progress in the terminal.`,
        result.error ? "!" : "");
    }
  } catch (error) {
    console.error("bcdl:", error.message);
    await status(error.message, "!");
  } finally {
    busy = false;
  }
}

browser.browserAction.onClicked.addListener(async (tab) => {
  if (!Bcdl.isDownloadPage(tab.url)) {
    await status("Open your Bandcamp purchase download page first.", "!");
    return;
  }
  const result = await collect(tab.id, "bcdl:copy");
  await status(result.error || `Copied ${result.urls.length} links as JSON.`,
    result.error ? "!" : String(result.urls.length));
});

async function start() {
  const stored = await browser.storage.local.get("profileId");
  profileId = stored.profileId || crypto.randomUUID();
  if (!stored.profileId) await browser.storage.local.set({ profileId });
  await poll();
  setInterval(poll, 2000);
}

start().catch((error) => {
  console.error("bcdl initialization failed:", error.message);
  status("Extension initialization failed. Reload the extension.", "!");
});
