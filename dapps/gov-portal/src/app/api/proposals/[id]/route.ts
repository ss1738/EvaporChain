/**
 * /api/proposals/[id] — fetch detail, append an endorsement, or mark active
 * after a successful chain submit.
 *
 *   GET   /api/proposals/:id
 *   PATCH /api/proposals/:id
 *         body (one of):
 *           { action: "endorse",  endorsement: Endorsement }
 *           { action: "activate", on_chain_tx_hash: string }
 *           { action: "archive",  reason: string }
 */

import { NextResponse } from "next/server";
import { proposalStore } from "@/lib/proposalStore";
import type { Endorsement } from "@/lib/types";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

interface RouteCtx {
  params: Promise<{ id: string }>;
}

export async function GET(_req: Request, ctx: RouteCtx) {
  const { id } = await ctx.params;
  const proposal = await proposalStore.get(id);
  if (!proposal) {
    return NextResponse.json({ error: "not found" }, { status: 404 });
  }
  return NextResponse.json({ proposal });
}

interface EndorseBody {
  action: "endorse";
  endorsement: Endorsement;
}
interface ActivateBody {
  action: "activate";
  on_chain_tx_hash: string;
}
interface ArchiveBody {
  action: "archive";
  reason: string;
}
type PatchBody = EndorseBody | ActivateBody | ArchiveBody;

export async function PATCH(req: Request, ctx: RouteCtx) {
  const { id } = await ctx.params;
  let body: PatchBody;
  try {
    body = (await req.json()) as PatchBody;
  } catch {
    return NextResponse.json({ error: "invalid JSON body" }, { status: 400 });
  }

  try {
    const updated = await proposalStore.update(id, (p) => {
      if (body.action === "endorse") {
        const e = body.endorsement;
        if (!e?.address || !Number.isFinite(e.stake) || e.stake <= 0) {
          throw new Error("endorsement requires address + positive stake");
        }
        if (p.state !== "open") {
          throw new Error(`cannot endorse a ${p.state} proposal`);
        }
        // Replace any prior endorsement from the same address — validators may
        // re-sign with updated stake, but we want one row per endorser so the
        // sum doesn't double-count.
        const filtered = p.endorsements.filter(
          (x) => x.address.toLowerCase() !== e.address.toLowerCase(),
        );
        return {
          ...p,
          endorsements: [
            ...filtered,
            { ...e, signed_at: e.signed_at || Date.now() },
          ],
        };
      }
      if (body.action === "activate") {
        if (!body.on_chain_tx_hash) {
          throw new Error("on_chain_tx_hash required to activate");
        }
        return {
          ...p,
          state: "active",
          on_chain_tx_hash: body.on_chain_tx_hash,
        };
      }
      if (body.action === "archive") {
        return {
          ...p,
          state: "historical",
          historical_reason: body.reason || "archived",
        };
      }
      throw new Error("unknown action");
    });
    return NextResponse.json({ proposal: updated });
  } catch (e) {
    const msg = e instanceof Error ? e.message : "update failed";
    const status = msg.includes("not found") ? 404 : 400;
    return NextResponse.json({ error: msg }, { status });
  }
}
