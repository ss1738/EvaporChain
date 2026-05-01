import type {
  MortalMessage,
  MessageStats,
  SendMessagePayload,
  BoostPayload,
  PatronagePledgeRequest,
  PatronagePledgeResponse,
  PatronageStatusResponse,
  DemurrageOwedRequest,
  DemurrageOwedResponse,
  ChainStatusResponse,
  DecayForecastResponse,
} from "./types";

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

/** Fetch chain status — used to align decay forecasts to the current epoch. */
export function getStatus() {
  return request<ChainStatusResponse>("/status");
}

/** Decay forecast: projected energy curve + evaporation epoch for an object. */
export function getDecayForecast(objectId: string) {
  return request<DecayForecastResponse>(`/object/${objectId}/forecast`);
}

/** Open a Patronage Covenant against the message's underlying state object. */
export function pledgePatronage(req: PatronagePledgeRequest) {
  return request<PatronagePledgeResponse>("/patronage/pledge", {
    method: "POST",
    body: JSON.stringify(req),
  });
}

/** Read the chain's patronage namespace + active covenant summary. */
export function getPatronageStatus() {
  return request<PatronageStatusResponse>("/patronage/status");
}

/** Pure-compute helper: how much demurrage does an account currently owe? */
export function computeDemurrage(req: DemurrageOwedRequest) {
  return request<DemurrageOwedResponse>("/demurrage/owed", {
    method: "POST",
    body: JSON.stringify(req),
  });
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
