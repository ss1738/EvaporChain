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

/** Send a mortal message — EvaporScript-first pipeline.
 *
 * The legacy `/messages/send` REST endpoint never had a server-side handler
 * (verified 2026-05-01). Per `feedback_evaporscript_first` rule, message
 * lifecycle now lives in an on-chain EvaporScript contract:
 *
 *   1. **Deploy** a fresh `MortalMessage` contract instance with the chosen
 *      energy + half-life. The contract's own thermodynamic decay IS the
 *      message lifespan.
 *   2. **Seal** it with sender + recipient + body via `set_payload`. The
 *      contract is unreadable until sealed (`read()` reverts on
 *      `sealed == false`).
 *
 * Why this isn't one call: `/api/tx/deploy-script` returns a tx_hash
 * immediately, but the chain assigns the actual `contract_id` only at
 * execution time. The seal call needs that id. Today's `/api/tx/:hash`
 * does not yet surface `contract_id` on its receipt — that's a known
 * server-side gap (TODO at the bottom of this file).
 *
 * Until the receipt exposes contract_id, this function does **only the
 * deploy** and returns its tx_hash. The UI must then either: (a) wait for
 * the deploy to finalise and read contract_id off the deployer account's
 * latest contract list, or (b) call `sealMortalMessage(contract_id, …)`
 * once that id is known by some other means.
 *
 * This is honest: the pilot proves the dapp can reach EvaporScript via
 * deploy-script. Closing the seal-handoff loop needs one server-side
 * change (extend `TxStatusResponse` with `contract_id?: u64` for
 * deploy-script tx types).
 */
export async function sendMessage(payload: SendMessagePayload) {
  const deploy = await deployMortalMessageContract(payload.energy, payload.half_life);
  // Stash the seal arguments on the response so a follow-up handler can
  // pick them up once contract_id is known. The legacy `id` field
  // is repurposed as the deploy tx_hash to preserve the call shape.
  return {
    id: deploy.tx_hash ?? "",
    tx_hash: deploy.tx_hash ?? "",
    pending_seal: {
      body: payload.content,
      recipient_hex: payload.to,
    },
  };
}

/** Step 1 — deploys a fresh `MortalMessage` instance with the operator's
 *  energy + half-life. The contract is empty until sealed.
 *  Hits `/api/tx/deploy-script`. */
export async function deployMortalMessageContract(energy: number, half_life: number) {
  const { MORTAL_MESSAGE_SOURCE } = await import("./contract");
  return request<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/deploy-script",
    {
      method: "POST",
      body: JSON.stringify({
        deployer: 1, // address byte; real wallet integration replaces this
        source_code: MORTAL_MESSAGE_SOURCE,
        energy,
        half_life,
      }),
    }
  );
}

/** Step 2 — seal a deployed `MortalMessage` contract with the recipient +
 *  body via `set_payload`. Hits `/api/tx/call-script`. The dapp needs to
 *  source `contract_id` once the deploy finalises (see TODO below). */
export async function sealMortalMessage(
  contract_id: number,
  body: string,
  recipient_hex: string
) {
  return request<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/call-script",
    {
      method: "POST",
      body: JSON.stringify({
        caller: 1, // sender address byte
        contract_id,
        method: "set_payload",
        args: [body, recipient_hex],
        epoch: 0,
      }),
    }
  );
}

/** Read a sealed message back from the contract via `call-script` /
 *  `read`. Recipient or sender only — anyone else is rejected by the
 *  contract's own `require(caller == self.recipient || caller == owner)`. */
export async function readMortalMessage(contract_id: number, caller_byte: number) {
  return request<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/call-script",
    {
      method: "POST",
      body: JSON.stringify({
        caller: caller_byte,
        contract_id,
        method: "read",
        args: [],
        epoch: 0,
      }),
    }
  );
}

// TODO(server): extend `TxStatusResponse` (api.rs) with `contract_id?: u64`
// populated from the receipt for `Transaction::DeployContract` and
// `Transaction::DeployScript`. Without it, the seal step has no way to
// learn the chain-assigned contract id — the dapp ends up needing
// out-of-band signalling. One-line backend change; deferred until the
// pilot UX demands it.

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
