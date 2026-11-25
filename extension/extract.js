"use strict";

globalThis.Bcdl = (() => {
  function isDownloadPage(value) {
    try {
      const url = new URL(value);
      return url.origin === "https://bandcamp.com" && url.pathname === "/download";
    } catch {
      return false;
    }
  }

  function isDownloadLink(value) {
    try {
      if (value.length > 8192 || /[\u0000-\u0020\u007f]/.test(value)) return false;
      const url = new URL(value);
      return (
        url.protocol === "https:" &&
        /^p[0-9]+\.bcbits\.com$/.test(url.hostname) &&
        !url.port &&
        !url.username &&
        !url.password &&
        !url.hash &&
        /^\/download\/(album|track)\//.test(url.pathname)
      );
    } catch {
      return false;
    }
  }

  function extract(document, pageUrl) {
    if (!isDownloadPage(pageUrl)) {
      throw new Error("Open your Bandcamp purchase download page first.");
    }
    const urls = [...new Set(
      Array.from(document.querySelectorAll("a[href]"), (anchor) => anchor.href)
        .filter(isDownloadLink),
    )];
    if (urls.length === 0) {
      throw new Error("No download links are ready. Select your format, wait for the links, and retry.");
    }
    if (urls.length > 2000) {
      throw new Error("Too many download links (maximum 2000).");
    }
    return urls;
  }

  return { isDownloadPage, isDownloadLink, extract };
})();
