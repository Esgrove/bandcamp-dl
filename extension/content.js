"use strict";

browser.runtime.onMessage.addListener((message, sender) => {
  if (sender.id !== browser.runtime.id) return undefined;
  if (message.type !== "bcdl:collect" && message.type !== "bcdl:copy") return undefined;
  return (async () => {
    try {
      const urls = Bcdl.extract(document, location.href);
      if (message.type === "bcdl:copy") {
        await navigator.clipboard.writeText(JSON.stringify(urls, null, 2));
      }
      return { urls };
    } catch (error) {
      return { error: error.message };
    }
  })();
});
