const { test } = require("bun:test");
const assert = require("node:assert/strict");
require("../extension/extract.js");
const { extract, isDownloadPage, isDownloadLink } = globalThis.Bcdl;
const page = "https://bandcamp.com/download?cart_id=example&sig=redacted";
const album = "https://p4.bcbits.com/download/album/example?sig=a%2Bb&x=1";
const track = "https://p5.bcbits.com/download/track/example";
const document = (urls) => ({ querySelectorAll: () => urls.map((href) => ({ href })) });

test("collects album/track anchors and preserves signed query strings", () => {
  assert.deepEqual(extract(document([album, track, album, "https://bandcamp.com/help"]), page), [album, track]);
});

test("rejects lookalike pages and download hosts", () => {
  for (const value of [
    "https://bandcamp.com.evil.test/download",
    "http://bandcamp.com/download",
    "https://bandcamp.com/download/other",
    "not a URL",
  ]) assert.equal(isDownloadPage(value), false);
  for (const value of [
    "https://p4.bcbits.com.evil.test/download/album/1",
    "https://p4.bcbits.com/image/1",
    "http://p4.bcbits.com/download/album/1",
    "https://user@p4.bcbits.com/download/album/1",
    "https://p4.bcbits.com:444/download/album/1",
    `${track}#fragment`,
    `${track}?\nsig=secret`,
  ]) assert.equal(isDownloadLink(value), false);
});

test("reports unsupported pages, unready links, and excessive link counts", () => {
  assert.throws(() => extract(document([album]), "https://example.com"), /Open your Bandcamp/);
  assert.throws(() => extract(document([]), page), /No download links/);
  assert.throws(() => extract(document(
    Array.from({ length: 2001 }, (_, index) => `${track}/${index}`),
  ), page), /Too many/);
});
