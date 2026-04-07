import type { MortalMessage, MessageStats, SendMessagePayload, BoostPayload } from "./types";

const BASE = "/api";

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...options,
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "Unknown error");
    throw new Error(`API ${res.status}: ${body}`);
  }
  return res.json() as Promise<T>;
}

/** Send a mortal message */
export function sendMessage(payload: SendMessagePayload) {
  return request<{ id: string; tx_hash: string }>("/messages/send", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

/** Fetch inbox for an address */
export function getInbox(address: string) {
  return request<MortalMessage[]>(`/messages/inbox/${address}`);
}

/** Fetch sent messages for an address */
export function getSentMessages(address: string) {
  return request<MortalMessage[]>(`/messages/sent/${address}`);
}

/** Fetch a single message with current energy */
export function getMessage(id: string) {
  return request<MortalMessage>(`/message/${id}`);
}

/** Boost a message with additional energy */
export function boostMessage(payload: BoostPayload) {
  return request<{ energy: number; status: string }>("/message/boost", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

/** Fetch stats for an address */
export function getStats(address: string) {
  return request<MessageStats>(`/messages/stats/${address}`);
}
