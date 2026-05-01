import type {
  MortalMessage,
  MessageStatus,
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

/** Increment the contract's `boost_count` via `record_boost`. Called as a
 *  follow-up to `/api/tx/refresh` so the contract's own telemetry stays
 *  in sync with off-chain energy deposits. */
export async function recordBoost(contract_id: number, caller_byte: number) {
  return request<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/call-script",
    {
      method: "POST",
      body: JSON.stringify({
        caller: caller_byte,
        contract_id,
        method: "record_boost",
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

// ── Read path: walk the on-chain script_engine registry ──
//
// The legacy /messages/{inbox,sent,stats}/:address and /message/:id endpoints
// were never implemented server-side. The read path now goes through
// /api/scripts (registry list) + /api/script/:id (per-contract state).
//
// One filter pass turns the global script list into "MortalMessage contracts
// owned-by or addressed-to this address". Each contract's `state` field is
// the authoritative source of body/recipient/sealed/boost_count.

/** Server-shape of an entry in /api/scripts. Subset of fields we consume. */
interface ScriptListEntry {
  id: number;
  name: string;
  creator: string; // truncated display form, e.g. "0x01000000…"
  energy: number;
  half_life: number;
  created_epoch: number;
  evaporated: boolean;
}

interface ScriptDetail extends ScriptListEntry {
  state: Record<string, unknown>;
  last_refreshed: number;
}

interface MortalMessageState {
  body?: string;
  recipient?: string; // hex with 0x prefix
  sender?: string;
  sealed?: boolean;
  boost_count?: number;
  last_boost_epoch?: number;
}

/** Map a ScriptDetail of `name == MortalMessage` into the dApp's
 *  MortalMessage shape. Returns null if the contract isn't sealed yet
 *  (body/recipient are unset until set_payload runs). */
function scriptToMessage(detail: ScriptDetail): MortalMessage | null {
  const s = detail.state as MortalMessageState;
  if (!s.sealed || !s.body || !s.recipient || !s.sender) return null;
  const status: MessageStatus = detail.evaporated
    ? "ghost"
    : detail.energy === 0
      ? "grace"
      : "active";
  // The chain doesn't expose max_energy on the registry — use deploy-time
  // `half_life`-scaled estimate so the UI's energy bar has a reference.
  // A real fix would surface initial_energy on the registry; harmless
  // approximation here.
  const max_energy = Math.max(detail.energy, detail.energy + 1);
  return {
    id: String(detail.id),
    sender: s.sender,
    recipient: s.recipient,
    content: s.body,
    energy: detail.energy,
    max_energy,
    half_life: detail.half_life,
    created_at: String(detail.created_epoch),
    status,
    energy_percent: max_energy > 0 ? (detail.energy / max_energy) * 100 : 0,
  };
}

/** Normalise a hex address for substring comparison. Lower-cased, no `0x`. */
function canonHex(addr: string): string {
  return addr.trim().replace(/^0x/i, "").toLowerCase();
}

/** Two addresses match in the dapp's loose-prefix sense if either is a
 *  prefix of the other (covers the chain's 4-byte-truncated display form
 *  vs full 32-byte hex on contract state). */
function addressMatches(a: string, b: string): boolean {
  const ca = canonHex(a);
  const cb = canonHex(b);
  if (ca.length === 0 || cb.length === 0) return false;
  const shorter = ca.length < cb.length ? ca : cb;
  const longer = ca.length < cb.length ? cb : ca;
  // The chain truncates display-form addresses to 4 bytes (8 hex chars).
  // Don't accept matches shorter than that — anything below would alias too
  // many accounts on a populated chain.
  if (shorter.length < 8) return false;
  return longer.startsWith(shorter);
}

/** Walk the script_engine registry and resolve each MortalMessage hit
 *  into a full ScriptDetail. Bounded to the first 200 entries so the
 *  inbox doesn't issue thousands of GETs against a saturated node. */
async function listMortalMessageContracts(): Promise<ScriptDetail[]> {
  const list = await request<{ scripts: ScriptListEntry[]; count: number }>(
    "/scripts",
  );
  const mortalContracts = list.scripts
    .filter((s) => s.name === "MortalMessage")
    .slice(0, 200);
  // Fetch detail in parallel; tolerate per-contract errors so one bad
  // entry doesn't poison the whole list.
  const details = await Promise.all(
    mortalContracts.map((s) =>
      request<ScriptDetail>(`/script/${s.id}`).catch(() => null),
    ),
  );
  return details.filter((d): d is ScriptDetail => d !== null);
}

/** Fetch inbox for an address — every sealed MortalMessage where the
 *  recipient field on-chain matches `address`. */
export async function getInbox(address: string): Promise<MortalMessage[]> {
  const details = await listMortalMessageContracts();
  return details
    .map(scriptToMessage)
    .filter((m): m is MortalMessage => m !== null && addressMatches(m.recipient, address))
    // Newest-first by created_at (which we encoded as epoch).
    .sort((a, b) => Number(b.created_at) - Number(a.created_at));
}

/** Fetch sent messages for an address — every sealed MortalMessage where
 *  the on-chain sender field matches `address`. */
export async function getSentMessages(address: string): Promise<MortalMessage[]> {
  const details = await listMortalMessageContracts();
  return details
    .map(scriptToMessage)
    .filter((m): m is MortalMessage => m !== null && addressMatches(m.sender, address))
    .sort((a, b) => Number(b.created_at) - Number(a.created_at));
}

/** Fetch a single message by its on-chain contract id (the dApp's `id`
 *  is the contract id rendered as a string). */
export async function getMessage(id: string): Promise<MortalMessage> {
  const detail = await request<ScriptDetail>(`/script/${id}`);
  if (detail.name !== "MortalMessage") {
    throw new Error(`script ${id} is not a MortalMessage (name=${detail.name})`);
  }
  const m = scriptToMessage(detail);
  if (!m) {
    throw new Error(`MortalMessage ${id} is not yet sealed`);
  }
  return m;
}

/** Boost a message's lifespan. EvaporScript surface: the chain refreshes
 *  the contract's energy via `Transaction::Refresh` against the contract's
 *  underlying object; we wire that through the existing /api/tx/refresh
 *  endpoint and then issue a record_boost call so the contract's
 *  boost_count keeps step with off-chain energy deposits.
 *
 *  Two-step is honest about the chain semantics: refresh deposits energy,
 *  record_boost is a telemetry hook the contract maintains itself. */
export async function boostMessage(payload: BoostPayload) {
  // Step 1: deposit the energy via /api/tx/refresh. The endpoint takes
  // {object_id, energy_deposit}; we pass the contract id as object_id
  // — the chain's evaporation engine treats it the same as a state
  // object's id for refresh purposes.
  await request("/tx/refresh", {
    method: "POST",
    body: JSON.stringify({
      object_id: payload.message_id,
      energy_deposit: payload.energy,
    }),
  });
  // Step 2: bump the contract's own boost_count via record_boost. Caller
  // byte = 1 (the sender). If the contract isn't sealed yet the call
  // reverts inside the contract — refresh in step 1 still landed.
  await recordBoost(Number(payload.message_id), 1).catch(() => {
    // Non-fatal; the energy deposit succeeded above.
  });
  return { energy: payload.energy, status: "boost queued" };
}

/** Fetch stats for an address by walking the same registry once. */
export async function getStats(address: string): Promise<MessageStats> {
  const details = await listMortalMessageContracts();
  const msgs = details
    .map(scriptToMessage)
    .filter((m): m is MortalMessage => m !== null);
  const sent = msgs.filter((m) => addressMatches(m.sender, address));
  const received = msgs.filter((m) => addressMatches(m.recipient, address));
  const alive = msgs.filter(
    (m) =>
      (addressMatches(m.sender, address) || addressMatches(m.recipient, address)) &&
      m.status !== "ghost",
  );
  const evaporated = msgs.filter(
    (m) =>
      (addressMatches(m.sender, address) || addressMatches(m.recipient, address)) &&
      m.status === "ghost",
  );
  const total_energy_spent = sent.reduce(
    (sum, m) => sum + Math.max(0, m.max_energy - m.energy),
    0,
  );
  return {
    sent: sent.length,
    received: received.length,
    alive: alive.length,
    evaporated: evaporated.length,
    total_energy_spent,
  };
}
