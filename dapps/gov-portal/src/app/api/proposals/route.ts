/**
 * /api/proposals — list and create off-chain proposals.
 *
 *   GET  /api/proposals?state=open|active|historical
 *   POST /api/proposals
 *        body: { type, title, rationale, payload, required_stake }
 *
 * The route handler runs server-side so it can touch data/proposals.json
 * directly via the file-backed proposalStore. It does NOT shadow the chain's
 * /api endpoints because next.config.ts uses a `fallback` rewrite — local
 * matches resolve here, everything else proxies to EVAPORCHAIN_RPC.
 */

import { NextResponse } from "next/server";
import { ulid } from "ulid";
import { proposalStore } from "@/lib/proposalStore";
import type {
  ForkChoicePayload,
  Proposal,
  ProposalState,
  ProposalType,
  UpgradeContractPayload,
} from "@/lib/types";

export const runtime = "nodejs";
// File-backed store — must not be cached.
export const dynamic = "force-dynamic";

interface CreateBody {
  type: ProposalType;
  title: string;
  rationale: string;
  payload: ForkChoicePayload | UpgradeContractPayload;
  required_stake: number;
}

export async function GET(req: Request) {
  const url = new URL(req.url);
  const state = url.searchParams.get("state") as ProposalState | null;
  const list = await proposalStore.list(
    state ? { state } : undefined,
  );
  // Newest first — proposals are time-keyed by created_at.
  list.sort((a, b) => b.created_at - a.created_at);
  return NextResponse.json({ proposals: list });
}

export async function POST(req: Request) {
  let body: CreateBody;
  try {
    body = (await req.json()) as CreateBody;
  } catch {
    return NextResponse.json({ error: "invalid JSON body" }, { status: 400 });
  }

  if (!body.type || !body.title || !body.payload) {
    return NextResponse.json(
      { error: "type, title, payload are required" },
      { status: 400 },
    );
  }
  if (body.type !== "fork_choice" && body.type !== "upgrade_contract") {
    return NextResponse.json({ error: "unknown proposal type" }, { status: 400 });
  }
  const reqStake = Number(body.required_stake);
  if (!Number.isFinite(reqStake) || reqStake <= 0) {
    return NextResponse.json(
      { error: "required_stake must be a positive number" },
      { status: 400 },
    );
  }

  const now = Date.now();
  const proposal: Proposal = {
    id: ulid(),
    type: body.type,
    title: body.title.trim().slice(0, 200),
    rationale: (body.rationale ?? "").slice(0, 8_000),
    payload: body.payload,
    required_stake: Math.floor(reqStake),
    endorsements: [],
    state: "open",
    created_at: now,
    updated_at: now,
  };

  const created = await proposalStore.create(proposal);
  return NextResponse.json({ proposal: created }, { status: 201 });
}
