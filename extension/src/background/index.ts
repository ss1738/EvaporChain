/**
 * EvaporChain Wallet — Background Service Worker (Manifest V3)
 *
 * Handles:
 * - Message routing between popup, content script, and dApps
 * - Transaction approval queue with signing
 * - Auto-lock via chrome.alarms (survives MV3 worker suspension)
 */

import { signMessage } from "@/crypto/keystore";
import { initCrypto } from "@/crypto/wasm-bridge";

const AUTO_LOCK_MINUTES = 15;
const AUTO_LOCK_MS = AUTO_LOCK_MINUTES * 60 * 1000;
const AUTO_LOCK_ALARM = "evaporchain_auto_lock";

// Reset auto-lock on any message. Uses chrome.alarms (not setTimeout) because
// MV3 service workers can be suspended at any time, so a JS-level timer would
// silently die. Also persists `wallet_unlocked_until` so an alarm that fires
// after the worker was woken from cold can decide whether the wallet should
// already be locked even if the alarm hasn't fired yet.
function resetLockTimer() {
  const unlockedUntil = Date.now() + AUTO_LOCK_MS;
  chrome.storage.local.set({ wallet_unlocked_until: unlockedUntil });
  chrome.alarms.create(AUTO_LOCK_ALARM, { delayInMinutes: AUTO_LOCK_MINUTES });
}

// If the worker was suspended past the unlock window, lock immediately on
// the next message rather than waiting for the alarm.
async function lockIfExpired(): Promise<void> {
  return new Promise((resolve) => {
    chrome.storage.local.get("wallet_unlocked_until", (result) => {
      const until = result.wallet_unlocked_until as number | undefined;
      if (typeof until === "number" && Date.now() >= until) {
        chrome.storage.local.set({ wallet_locked: true }, () => resolve());
      } else {
        resolve();
      }
    });
  });
}

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === AUTO_LOCK_ALARM) {
    chrome.storage.local.set({ wallet_locked: true });
  }
});

// ── Hex helpers ──

function toHex(bytes: Uint8Array): string {
  return Array.from(bytes).map(b => b.toString(16).padStart(2, "0")).join("");
}

function fromHex(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    bytes[i / 2] = parseInt(hex.substring(i, i + 2), 16);
  }
  return bytes;
}

// Listen for messages from content script / popup
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  // Lock first if the worker was suspended past the unlock window, then
  // refresh the auto-lock window for this message.
  lockIfExpired().then(() => {
    resetLockTimer();
  });

  switch (message.type) {
    case "EVAPORCHAIN_CONNECT": {
      // dApp requesting connection — read active account from storage
      chrome.storage.local.get("evaporchain_keystore", (result) => {
        try {
          const data = result.evaporchain_keystore ? JSON.parse(result.evaporchain_keystore) : null;
          const activeEntry = data?.entries?.find((e: { name: string }) => e.name === data.activeAccount);
          const origin = sender.tab?.url ? new URL(sender.tab.url).origin : "unknown";
          sendResponse({
            connected: !!activeEntry,
            accounts: activeEntry ? [activeEntry.address] : [],
            origin,
          });
        } catch {
          sendResponse({ connected: false, accounts: [] });
        }
      });
      break;
    }

    case "EVAPORCHAIN_REQUEST": {
      // dApp requesting a transaction or signature
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
          requestId: message.requestId,
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
      // Popup approving a pending request — sign and broadcast
      const { requestId, result: approvalResult } = message;

      chrome.storage.local.get("pending_requests", (data) => {
        const pending = (data.pending_requests ?? []);
        const request = pending.find((r: { id: string }) => r.id === requestId);

        // Remove from pending
        const remaining = pending.filter((r: { id: string }) => r.id !== requestId);
        chrome.storage.local.set({ pending_requests: remaining });

        // Send result back to the content script / dApp
        if (request?.tabId) {
          chrome.tabs.sendMessage(request.tabId, {
            type: "EVAPORCHAIN_REQUEST_RESPONSE",
            requestId: request.requestId,
            result: approvalResult,
          });
        }
      });

      sendResponse({ approved: true });
      break;
    }

    case "EVAPORCHAIN_REJECT": {
      // Popup rejecting a pending request
      const { requestId: rejId } = message;

      chrome.storage.local.get("pending_requests", (data) => {
        const pending = (data.pending_requests ?? []);
        const request = pending.find((r: { id: string }) => r.id === rejId);

        const remaining = pending.filter((r: { id: string }) => r.id !== rejId);
        chrome.storage.local.set({ pending_requests: remaining });

        if (request?.tabId) {
          chrome.tabs.sendMessage(request.tabId, {
            type: "EVAPORCHAIN_REQUEST_RESPONSE",
            requestId: request.requestId,
            error: "User rejected the request",
          });
        }
      });

      sendResponse({ rejected: true });
      break;
    }

    case "EVAPORCHAIN_SIGN": {
      // Internal: popup asks background to sign a message with the active key
      // message.payload: { secretKeyHex, txPayload, publicKeyHex? }
      handleSign(message.payload)
        .then(result => sendResponse(result))
        .catch(err => sendResponse({ error: err.message }));
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

/**
 * Sign a transaction payload using the decrypted secret key.
 * Called by the popup when user approves a dApp transaction.
 *
 * `publicKeyHex` may be supplied in the payload to avoid a storage lookup;
 * otherwise we read the active account's public key from
 * `chrome.storage.local.evaporchain_keystore`.
 */
async function handleSign(payload: {
  secretKeyHex: string;
  txPayload: string;
  publicKeyHex?: string;
}): Promise<{ signature: string; publicKey: string }> {
  await initCrypto();

  const secretKey = fromHex(payload.secretKeyHex);
  const txBytes = new TextEncoder().encode(payload.txPayload);
  const signature = await signMessage(secretKey, txBytes);

  let publicKey = payload.publicKeyHex ?? "";
  if (!publicKey) {
    publicKey = await new Promise<string>((resolve) => {
      chrome.storage.local.get("evaporchain_keystore", (result) => {
        try {
          const data = result.evaporchain_keystore
            ? JSON.parse(result.evaporchain_keystore)
            : null;
          const activeEntry = data?.entries?.find(
            (e: { name: string }) => e.name === data.activeAccount,
          );
          resolve(activeEntry?.publicKey ?? "");
        } catch {
          resolve("");
        }
      });
    });
  }

  return {
    signature: toHex(signature),
    publicKey,
  };
}

// On install
chrome.runtime.onInstalled.addListener(() => {
  console.log("EvaporChain Wallet extension installed");
  chrome.storage.local.set({ wallet_locked: true, pending_requests: [] });
});
