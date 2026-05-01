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
 * The chain assigns `contract_id` at execution time. We poll
 * `/api/tx/:hash` for `state == "finalised"` (or `"included"` in
 * single-node dev mode) and read `contract_id` off the receipt — the
 * server-side resolver matches by (deployer-prefix, epoch) over the
 * script-engine registry. Then we issue the seal call.
 *
 * Polling budget: 30 cycles × 2s = 60s. Plenty for a chain producing
 * blocks at the default 2s interval; bounded so a stuck deploy can't
 * pin the dapp forever.
 */
export async function sendMessage(payload: SendMessagePayload) {
  const deploy = await deployMortalMessageContract(payload.energy, payload.half_life);
  // Schedule the seal in the background. The UI returns immediately with
  // a "queued" state; the seal resolves once the deploy lands on-chain.
  sealMortalMessageWhenReady(
    deploy.tx_hash ?? "",
    payload.content,
    payload.to,
  ).catch((err) => console.warn("[mortal-messages] seal handoff failed:", err));
  return {
    id: deploy.tx_hash ?? "",
    tx_hash: deploy.tx_hash ?? "",
  };
}

/** Background helper: poll `/api/tx/:hash` until the deploy finalises,
 *  pull `contract_id` from the receipt, then seal. Bounded retry. */
async function sealMortalMessageWhenReady(
  deploy_tx_hash: string,
  body: string,
  recipient_hex: string,
): Promise<void> {
  if (!deploy_tx_hash) {
    throw new Error("missing deploy tx_hash; deploy may have been rejected");
  }
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 2000));
    try {
      const status = await request<{
        state: string;
        contract_id?: number;
      }>(`/tx/${deploy_tx_hash}`);
      if (
        (status.state === "finalised" || status.state === "included") &&
        typeof status.contract_id === "number"
      ) {
        await sealMortalMessage(status.contract_id, body, recipient_hex);
        return;
      }
      if (status.state === "rejected") {
        throw new Error(`deploy tx rejected: ${deploy_tx_hash}`);
      }
    } catch (e) {
      if (e instanceof Error && e.message.includes("rejected")) throw e;
      // Other transient errors during polling are non-fatal; retry.
    }
  }
  throw new Error(
    `deploy tx ${deploy_tx_hash} did not finalise within 60s — seal not issued`,
  );
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

// Server-side resolver shipped 2026-05-01: TxStatusResponse now carries
// `contract_id?: u64` populated by walking the script_engine /
// contract_engine registry by (deployer-prefix, epoch). One known caveat:
// the prefix match aliases deployers sharing the same first 4 bytes.
// Production-grade fix (full-address index `tx_hash → contract_id` written
// at execution time) lives behind a TODO in persistence.rs::index_block_
// transactions — when many deployers collide, swap to that.

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
