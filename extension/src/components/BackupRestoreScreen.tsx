import { useRef, useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { BrowserKeyStore } from "@/crypto/keystore";
import { Header } from "./Header";

type Stage = "idle" | "exporting" | "importing" | "done";

export function BackupRestoreScreen() {
  const { setView, keystore, preferences } = useWallet();
  const [stage, setStage] = useState<Stage>("idle");
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  async function handleExport() {
    if (!keystore) return;
    setStage("exporting");
    setError(null);
    try {
      const data = {
        version: 1,
        exportedAt: new Date().toISOString(),
        keystore: (keystore as unknown as { data: unknown }).data,
        preferences,
      };
      const blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `evaporchain-backup-${Date.now()}.json`;
      a.click();
      URL.revokeObjectURL(url);
      setInfo("Backup downloaded. Store it somewhere safe — keys are encrypted but guard the file.");
      setStage("done");
    } catch (e) {
      setError(String(e));
      setStage("idle");
    }
  }

  async function handleImport(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    setStage("importing");
    setError(null);
    setInfo(null);
    try {
      const text = await file.text();
      const data = JSON.parse(text);
      if (!data.keystore) throw new Error("Invalid backup file — missing keystore field");

      const restored = new BrowserKeyStore(data.keystore);
      await restored.save();

      const accountCount = restored.listAccounts().length;
      setInfo(`Restored ${accountCount} account${accountCount !== 1 ? "s" : ""}. Reload the extension to see them.`);
      setStage("done");
    } catch (e) {
      setError(String(e));
      setStage("idle");
    }
    // Reset input so same file can be re-selected
    if (fileRef.current) fileRef.current.value = "";
  }

  return (
    <div className="flex flex-col h-full">
      <Header />
      <div className="px-4 pt-4 pb-2">
        <button
          onClick={() => setView("settings")}
          className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
        >
          ← Settings
        </button>
        <h2 className="text-lg font-semibold text-zinc-100">Backup &amp; Restore</h2>
        <p className="text-xs text-zinc-500 mt-1">
          Export an encrypted backup of your accounts and preferences.
          Your keys are encrypted with your password — the backup file alone is not sufficient to steal funds.
        </p>
      </div>

      <div className="flex-1 overflow-y-auto px-4 space-y-4 pb-4">
        {/* Export */}
        <div className="px-4 py-4 rounded-xl bg-evap-surface border border-evap-border space-y-2">
          <h3 className="text-sm font-semibold text-zinc-200">Export Backup</h3>
          <p className="text-xs text-zinc-500">
            Downloads a JSON file containing all your encrypted accounts and settings.
          </p>
          <button
            onClick={handleExport}
            disabled={stage === "exporting" || stage === "importing"}
            className="w-full py-2 rounded-lg bg-evap-cyan/10 border border-evap-cyan/30 text-xs text-evap-cyan hover:bg-evap-cyan/20 disabled:opacity-50 transition"
          >
            {stage === "exporting" ? "Preparing…" : "Download Backup"}
          </button>
        </div>

        {/* Import */}
        <div className="px-4 py-4 rounded-xl bg-evap-surface border border-evap-border space-y-2">
          <h3 className="text-sm font-semibold text-zinc-200">Restore from Backup</h3>
          <p className="text-xs text-zinc-500">
            Select a previously exported backup file. This will overwrite your current keystore.
          </p>
          <input
            ref={fileRef}
            type="file"
            accept=".json,application/json"
            onChange={handleImport}
            disabled={stage === "exporting" || stage === "importing"}
            className="hidden"
            id="backup-file-input"
          />
          <label
            htmlFor="backup-file-input"
            className={`block w-full py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-center text-zinc-300 hover:border-evap-cyan/40 cursor-pointer transition ${
              stage === "importing" ? "opacity-50 pointer-events-none" : ""
            }`}
          >
            {stage === "importing" ? "Restoring…" : "Choose Backup File"}
          </label>
        </div>

        {/* Status messages */}
        {error && (
          <div className="px-4 py-3 rounded-xl bg-evap-red/10 border border-evap-red/30">
            <p className="text-xs text-evap-red">{error}</p>
          </div>
        )}
        {info && (
          <div className="px-4 py-3 rounded-xl bg-evap-cyan/10 border border-evap-cyan/30">
            <p className="text-xs text-evap-cyan">{info}</p>
          </div>
        )}

        {/* Warning */}
        <div className="px-4 py-3 rounded-xl bg-yellow-500/5 border border-yellow-500/20">
          <p className="text-xs text-yellow-400/80">
            Never share your backup file unencrypted. Anyone with your backup and password can access your funds.
          </p>
        </div>
      </div>
    </div>
  );
}
