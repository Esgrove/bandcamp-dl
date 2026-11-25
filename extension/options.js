"use strict";

const field = document.getElementById("pairing");
const message = document.getElementById("message");
const status = document.getElementById("status");

document.getElementById("pairing-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    const input = JSON.parse(field.value);
    if (!input || typeof input !== "object"
        || !Number.isInteger(input.port) || input.port < 1 || input.port > 65535
        || typeof input.token !== "string" || !/^[0-9a-f]{64}$/i.test(input.token)) {
      throw new Error("Expected JSON with port (1-65535) and a 64-character hexadecimal token.");
    }
    await browser.storage.local.set({ pairing: { port: input.port, token: input.token } });
    field.value = "";
    message.textContent = "Pairing saved. Run bcdl --browser in your terminal.";
  } catch (error) {
    message.textContent = error instanceof SyntaxError ? "Invalid JSON. Paste the output of bcdl --browser-setup." : error.message;
  }
});

document.getElementById("remove").addEventListener("click", async () => {
  try {
    await browser.storage.local.remove("pairing");
    field.value = "";
    message.textContent = "Pairing removed. Toolbar copying still works.";
  } catch (error) {
    message.textContent = `Cannot remove pairing: ${error.message}`;
  }
});

browser.storage.local.get(["pairing", "status"]).then((stored) => {
  message.textContent = stored.pairing ? "This Firefox profile is paired." : "Not paired yet.";
  status.textContent = stored.status || "";
}, (error) => { message.textContent = `Cannot read settings: ${error.message}`; });

browser.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.status) status.textContent = changes.status.newValue || "";
});
