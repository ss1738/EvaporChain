/**
 * Server-side JSON proposal store.
 *
 * No DB by design — this is an off-chain coordination cache, not a system of
 * record. The chain itself is the authoritative source for whether an
 * amendment passed; the JSON file is a forum-style queue that fills up the
 * canonical request body and lets validators pile on stake until quorum.
 *
 * A simple in-process mutex serialises read-modify-write operations so two
 * concurrent endorsements can't lose updates. Good enough for the single
 * Next.js process that runs the portal; a multi-instance deployment would
 * need a real lock or move to Postgres.
 */

import { promises as fs } from "node:fs";
import path from "node:path";
import type { Proposal, ProposalState } from "./types";

const DATA_FILE = path.resolve(process.cwd(), "data", "proposals.json");

let writeLock: Promise<unknown> = Promise.resolve();

async function readAll(): Promise<Proposal[]> {
  try {
    const raw = await fs.readFile(DATA_FILE, "utf8");
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed as Proposal[];
  } catch (e: unknown) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw e;
  }
}

async function writeAll(proposals: Proposal[]): Promise<void> {
  await fs.mkdir(path.dirname(DATA_FILE), { recursive: true });
  // Two-step write so a crash mid-flush doesn't corrupt the file.
  const tmp = `${DATA_FILE}.tmp`;
  await fs.writeFile(tmp, JSON.stringify(proposals, null, 2), "utf8");
  await fs.rename(tmp, DATA_FILE);
}

/** Serialise a mutating callback against any other in-flight mutation. */
function withLock<T>(fn: () => Promise<T>): Promise<T> {
  const next = writeLock.then(fn, fn);
  // Swallow the result of the chain so a rejected mutation doesn't poison
  // every future call; we still hand the original promise back to the caller.
  writeLock = next.catch(() => undefined);
  return next;
}

export const proposalStore = {
  async list(filter?: { state?: ProposalState }): Promise<Proposal[]> {
    const all = await readAll();
    if (!filter?.state) return all;
    return all.filter((p) => p.state === filter.state);
  },

  async get(id: string): Promise<Proposal | null> {
    const all = await readAll();
    return all.find((p) => p.id === id) ?? null;
  },

  async create(p: Proposal): Promise<Proposal> {
    return withLock(async () => {
      const all = await readAll();
      if (all.some((x) => x.id === p.id)) {
        throw new Error(`proposal ${p.id} already exists`);
      }
      all.push(p);
      await writeAll(all);
      return p;
    });
  },

  async update(
    id: string,
    mutator: (p: Proposal) => Proposal,
  ): Promise<Proposal> {
    return withLock(async () => {
      const all = await readAll();
      const idx = all.findIndex((p) => p.id === id);
      if (idx === -1) throw new Error(`proposal ${id} not found`);
      const next = mutator(all[idx]);
      next.updated_at = Date.now();
      all[idx] = next;
      await writeAll(all);
      return next;
    });
  },
};

/** Sum of endorsement stakes — UX gate only; chain re-validates. */
export function endorsedStake(p: Proposal): number {
  return p.endorsements.reduce((s, e) => s + (e.stake || 0), 0);
}

export function quorumMet(p: Proposal): boolean {
  return endorsedStake(p) >= p.required_stake;
}
