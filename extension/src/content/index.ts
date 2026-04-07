/**
 * Content script — injected into every page.
 *
 * Bridges messages between the in-page provider (window.evaporchain)
 * and the background service worker.
 */

// Inject the in-page provider script
const script = document.createElement("script");
script.src = chrome.runtime.getURL("inpage.js");
script.type = "module";
(document.head || document.documentElement).appendChild(script);
script.onload = () => script.remove();

// Relay messages from page → background
window.addEventListener("message", (event) => {
  if (event.source !== window) return;
  if (!event.data?.type?.startsWith("EVAPORCHAIN_")) return;

  chrome.runtime.sendMessage(event.data, (response) => {
    window.postMessage(
      { type: event.data.type + "_RESPONSE", ...response },
      "*"
    );
  });
});

// Relay messages from background → page
chrome.runtime.onMessage.addListener((message) => {
  if (message.type?.startsWith("EVAPORCHAIN_")) {
    window.postMessage(message, "*");
  }
});
