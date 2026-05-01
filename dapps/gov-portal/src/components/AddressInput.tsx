"use client";

import { useEffect, useState } from "react";
import { Wallet } from "lucide-react";
import { govApi } from "@/lib/api";
import type { Validator } from "@evaporchain/wallet-sdk";

interface Props {
  value: string;
  onChange: (address: string, validator: Validator | null) => void;
}

/**
 * Paste-mode validator picker. Real WalletConnect / Sign-in-with-EvaporChain
 * lives in the extension; here we accept a 32-byte hex address and look it
 * up against /api/validators. If the chain doesn't recognise the address,
 * the form still lets the user proceed, but stake defaults to 0 and the
 * Endorse button stays disabled — chains can't pass quorum on phantom stake.
 */
export function AddressInput({ value, onChange }: Props) {
  const [validator, setValidator] = useState<Validator | null>(null);
  const [loading, setLoading] = useState(false);
  const [touched, setTouched] = useState(false);

  useEffect(() => {
    let cancelled = false;
    if (!value || value.length < 8) {
      setValidator(null);
      onChange(value, null);
      return;
    }
    setLoading(true);
    govApi
      .findValidatorByAddress(value)
      .then((v) => {
        if (cancelled) return;
        setValidator(v);
        onChange(value, v);
      })
      .catch(() => {
        if (!cancelled) {
          setValidator(null);
          onChange(value, null);
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  const stale = touched && value && !validator && !loading;

  return (
    <div>
      <label className="mb-1 flex items-center gap-1 text-[10px] font-semibold uppercase tracking-wider text-zinc-500">
        <Wallet className="h-3 w-3" />
        Validator address
      </label>
      <input
        value={value}
        onChange={(e) => {
          setTouched(true);
          onChange(e.target.value, null);
        }}
        placeholder="0x... (32-byte hex)"
        className="w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-xs font-mono focus:border-evap-violet focus:outline-none focus:ring-1 focus:ring-evap-violet"
      />
      {loading && (
        <p className="mt-1 text-[10px] text-zinc-400">Looking up validator…</p>
      )}
      {validator && (
        <p className="mt-1 text-[10px] text-evap-emerald">
          {validator.name || "Validator"} · stake{" "}
          <span className="font-mono">
            {validator.stake.toLocaleString()}
          </span>{" "}
          · {validator.status}
        </p>
      )}
      {stale && (
        <p className="mt-1 text-[10px] text-evap-amber">
          Address not found in active validator set — chain will reject the
          stake at submit time.
        </p>
      )}
    </div>
  );
}
