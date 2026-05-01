"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { GitBranch, Cpu, Loader2 } from "lucide-react";
import type {
  AttractorSpec,
  ForkChoicePayload,
  ProposalType,
  UpgradeContractPayload,
} from "@/lib/types";

export default function NewProposalPage() {
  const router = useRouter();
  const [type, setType] = useState<ProposalType>("fork_choice");
  const [title, setTitle] = useState("");
  const [rationale, setRationale] = useState("");
  const [requiredStake, setRequiredStake] = useState(1500);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Fork-choice fields.
  const [mode, setMode] = useState<"mcc" | "singh_attractor">("singh_attractor");
  const [attractorJson, setAttractorJson] = useState<string>(
    JSON.stringify([{ center: 1000, basin_radius: 200 }], null, 2),
  );

  // UpgradeContract fields.
  const [owner, setOwner] = useState("");
  const [contractId, setContractId] = useState(0);
  const [bytecodeHex, setBytecodeHex] = useState("");
  const [bytecodeHashHex, setBytecodeHashHex] = useState("");
  const [nonce, setNonce] = useState(0);

  const buildPayload = (): ForkChoicePayload | UpgradeContractPayload => {
    if (type === "fork_choice") {
      let attractors: AttractorSpec[] = [];
      if (mode === "singh_attractor") {
        const parsed = JSON.parse(attractorJson) as AttractorSpec[];
        if (!Array.isArray(parsed)) throw new Error("attractors must be an array");
        attractors = parsed.map((a) => ({
          center: Number(a.center),
          basin_radius: Number(a.basin_radius),
        }));
      }
      return { mode, attractors };
    }
    if (!/^[0-9a-fA-F]{64}$/.test(bytecodeHashHex)) {
      throw new Error("new_bytecode_hash_hex must be 64 hex chars (BLAKE3 of bytecode)");
    }
    if (!/^[0-9a-fA-F]+$/.test(bytecodeHex)) {
      throw new Error("new_bytecode_hex must be hex");
    }
    return {
      owner,
      contract_id: contractId,
      new_bytecode_hex: bytecodeHex,
      new_bytecode_hash_hex: bytecodeHashHex,
      nonce,
    };
  };

  const handleSubmit = async () => {
    setError(null);
    if (!title.trim()) {
      setError("Title is required");
      return;
    }
    if (!Number.isFinite(requiredStake) || requiredStake <= 0) {
      setError("Required stake must be > 0");
      return;
    }
    let payload: ForkChoicePayload | UpgradeContractPayload;
    try {
      payload = buildPayload();
    } catch (e) {
      setError(e instanceof Error ? e.message : "invalid payload");
      return;
    }
    setSubmitting(true);
    try {
      const res = await fetch("/api/proposals", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          type,
          title,
          rationale,
          payload,
          required_stake: requiredStake,
        }),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error || `HTTP ${res.status}`);
      }
      const j = (await res.json()) as { proposal: { id: string } };
      router.push(`/proposals/${encodeURIComponent(j.proposal.id)}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "submit failed");
      setSubmitting(false);
    }
  };

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <div>
        <h1 className="text-xl font-bold text-zinc-900">Draft a Proposal</h1>
        <p className="mt-1 text-xs text-zinc-500">
          Off-chain draft. Validators endorse with stake; once quorum is met,
          anyone can broadcast the amendment to the chain.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <TypeCard
          active={type === "fork_choice"}
          onSelect={() => setType("fork_choice")}
          icon={<GitBranch className="h-4 w-4" />}
          title="Fork-choice mode"
          blurb="Switch between MCC and Singh-Attractor. Optional basin set."
        />
        <TypeCard
          active={type === "upgrade_contract"}
          onSelect={() => setType("upgrade_contract")}
          icon={<Cpu className="h-4 w-4" />}
          title="Contract upgrade"
          blurb="Replace deployed bytecode by id. Governance path — no admin sig."
        />
      </div>

      <Section title="Metadata">
        <Field label="Title">
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Promote Singh-Attractor with two basins…"
            className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
          />
        </Field>
        <Field label="Rationale (markdown)">
          <textarea
            value={rationale}
            onChange={(e) => setRationale(e.target.value)}
            rows={5}
            className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-xs font-mono focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
          />
        </Field>
        <Field label="Required quorum (EVAP stake)">
          <input
            type="number"
            min={1}
            value={requiredStake}
            onChange={(e) => setRequiredStake(Number(e.target.value))}
            className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
          />
        </Field>
      </Section>

      {type === "fork_choice" ? (
        <Section title="Fork-choice payload">
          <Field label="Target mode">
            <select
              value={mode}
              onChange={(e) => setMode(e.target.value as "mcc" | "singh_attractor")}
              className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
            >
              <option value="mcc">MCC (Most-Cumulative-Choice)</option>
              <option value="singh_attractor">Singh-Attractor</option>
            </select>
          </Field>
          {mode === "singh_attractor" && (
            <Field label="Attractors (JSON array)">
              <textarea
                value={attractorJson}
                onChange={(e) => setAttractorJson(e.target.value)}
                rows={6}
                className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-xs font-mono focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
              />
            </Field>
          )}
        </Section>
      ) : (
        <Section title="Upgrade-contract payload">
          <div className="grid grid-cols-2 gap-3">
            <Field label="Owner (proposer hex address)">
              <input
                value={owner}
                onChange={(e) => setOwner(e.target.value)}
                className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-xs font-mono focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
              />
            </Field>
            <Field label="Contract ID">
              <input
                type="number"
                value={contractId}
                onChange={(e) => setContractId(Number(e.target.value))}
                className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
              />
            </Field>
            <Field label="Nonce">
              <input
                type="number"
                value={nonce}
                onChange={(e) => setNonce(Number(e.target.value))}
                className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
              />
            </Field>
            <Field label="BLAKE3(bytecode) — 64 hex">
              <input
                value={bytecodeHashHex}
                onChange={(e) => setBytecodeHashHex(e.target.value.trim())}
                className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-xs font-mono focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
              />
            </Field>
          </div>
          <Field label="New bytecode (hex)">
            <textarea
              value={bytecodeHex}
              onChange={(e) => setBytecodeHex(e.target.value.trim())}
              rows={6}
              className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-xs font-mono focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
            />
          </Field>
        </Section>
      )}

      {error && (
        <div className="rounded-lg border border-evap-red/30 bg-red-50 px-3 py-2 text-sm text-evap-red">
          {error}
        </div>
      )}

      <div className="flex gap-3">
        <button
          onClick={() => router.push("/")}
          className="flex-1 rounded-lg border border-evap-border bg-white px-4 py-2 text-sm font-medium text-zinc-600 hover:bg-zinc-50"
        >
          Cancel
        </button>
        <button
          onClick={handleSubmit}
          disabled={submitting}
          className="flex flex-1 items-center justify-center gap-2 rounded-lg bg-gradient-to-r from-evap-violet to-evap-cyan px-4 py-2 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-40"
        >
          {submitting ? <Loader2 className="h-4 w-4 animate-spin" /> : "Publish draft"}
        </button>
      </div>
    </div>
  );
}

function TypeCard({
  active,
  onSelect,
  icon,
  title,
  blurb,
}: {
  active: boolean;
  onSelect: () => void;
  icon: React.ReactNode;
  title: string;
  blurb: string;
}) {
  return (
    <button
      onClick={onSelect}
      className={`rounded-xl border p-4 text-left transition ${
        active
          ? "border-evap-violet bg-violet-50"
          : "border-evap-border bg-white hover:border-zinc-300"
      }`}
    >
      <div className="flex items-center gap-2">
        <div
          className={`flex h-7 w-7 items-center justify-center rounded-md ${
            active ? "bg-evap-violet text-white" : "bg-zinc-100 text-zinc-600"
          }`}
        >
          {icon}
        </div>
        <span className="text-sm font-semibold text-zinc-900">{title}</span>
      </div>
      <p className="mt-2 text-[11px] text-zinc-500">{blurb}</p>
    </button>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border border-evap-border bg-white p-4">
      <p className="mb-3 text-[10px] font-semibold uppercase tracking-wider text-zinc-500">
        {title}
      </p>
      <div className="space-y-3">{children}</div>
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="mb-1 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
        {label}
      </label>
      {children}
    </div>
  );
}
