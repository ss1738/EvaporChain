/**
 * EvaporChain Wallet — Background Service Worker (Manifest V3)
 *
 * Handles:
 * - Message routing between popup, content script, and dApps
 * - Transaction approval queue
 * - Auto-lock timer
 */

const AUTO_LOCK_MS = 15 * 60 * 1000; // 15 minutes
let lockTimer: ReturnType<typeof setTimeout> | null = null;

// Reset auto-lock on any message
function resetLockTimer() {
  if (lockTimer) clearTimeout(lockTimer);
  lockTimer = setTimeout(() => {
    chrome.storage.local.set({ wallet_locked: true });
  }, AUTO_LOCK_MS);
}

// Pending dApp transaction requests
interface PendingRequest {
  id: string;
  origin: string;
  method: string;
  params: unknown;
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
}

const pendingRequests = new Map<string, PendingRequest>();

// Listen for messages from content script / popup
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  resetLockTimer();

  switch (message.type) {
    case "EVAPORCHAIN_CONNECT": {
      // dApp requesting connection
      const origin = sender.tab?.url ? new URL(sender.tab.url).origin : "unknown";
      sendResponse({ connected: true, origin });
      break;
    }

    case "EVAPORCHAIN_REQUEST": {
      // dApp requesting a transaction
      const id = crypto.randomUUID();
      const origin = sender.tab?.url ? new URL(sender.tab.url).origin : "unknown";

      // Store pending request — popup will resolve it
      chrome.storage.local.get("pending_requests", (result) => {
        const pending = result.pending_requests ?? [];
        pending.push({
          id,
          origin,
          method: message.method,
          params: message.params,
          tabId: sender.tab?.id,
        });
        chrome.storage.local.set({ pending_requests: pending });
      });

      // Open popup for approval
      chrome.action.openPopup?.();

      sendResponse({ pending: true, requestId: id });
      break;
    }

    case "EVAPORCHAIN_APPROVE": {
      // Popup approving a pending request
      const { requestId, result } = message;
      chrome.storage.local.get("pending_requests", (data) => {
        const pending = (data.pending_requests ?? []).filter(
          (r: { id: string }) => r.id !== requestId
        );
        chrome.storage.local.set({ pending_requests: pending });
      });
      sendResponse({ approved: true });
      break;
    }

    case "EVAPORCHAIN_REJECT": {
      // Popup rejecting a pending request
      const { requestId } = message;
      chrome.storage.local.get("pending_requests", (data) => {
        const pending = (data.pending_requests ?? []).filter(
          (r: { id: string }) => r.id !== requestId
        );
        chrome.storage.local.set({ pending_requests: pending });
      });
      sendResponse({ rejected: true });
      break;
    }

    case "KEEP_ALIVE": {
      resetLockTimer();
      sendResponse({ alive: true });
      break;
    }

    default:
      sendResponse({ error: "Unknown message type" });
  }

  return true; // Keep message channel open for async response
});

// On install
chrome.runtime.onInstalled.addListener(() => {
  console.log("EvaporChain Wallet extension installed");
  chrome.storage.local.set({ wallet_locked: true, pending_requests: [] });
});
