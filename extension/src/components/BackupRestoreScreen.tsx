import { useRef, useState } from "react";
import { ArrowLeft, Download, Upload, AlertTriangle } from "lucide-react";
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
          className="inline-flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-300 mb-4 transition"
        >
          <ArrowLeft className="w-3.5 h-3.5" strokeWidth={1.5} /> Settings
        </button>
        <h2 className="text-xl font-semibold text-zinc-100">Backup &amp; Restore</h2>
        <p className="text-sm text-zinc-500 mt-1.5">
          Export an encrypted snapshot of your accounts. Your keys are
          encrypted with your password — the backup alone cannot steal funds.
        </p>
      </div>

      <div className="flex-1 overflow-y-auto px-4 space-y-3 pb-4">
        {/* Export */}
        <div className="px-4 py-4 rounded-xl bg-evap-surface border border-evap-border">
          <div className="flex items-start gap-3 mb-3">
            <div className="w-9 h-9 rounded-lg bg-evap-cyan/10 border border-evap-cyan/20 flex items-center justify-center shrink-0">
              <Download className="w-4 h-4 text-evap-cyan" strokeWidth={1.5} />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-zinc-100">Export</h3>
              <p className="text-xs text-zinc-500 mt-0.5">
                Downloads a JSON file with your encrypted accounts and settings.
              </p>
            </div>
          </div>
          <button
            onClick={handleExport}
            disabled={stage === "exporting" || stage === "importing"}
            className="w-full py-2.5 rounded-lg bg-evap-cyan/10 border border-evap-cyan/30 text-sm font-medium text-evap-cyan hover:bg-evap-cyan/20 disabled:opacity-50 transition"
          >
            {stage === "exporting" ? "Preparing…" : "Download backup"}
          </button>
        </div>

        {/* Import */}
        <div className="px-4 py-4 rounded-xl bg-evap-surface border border-evap-border">
          <div className="flex items-start gap-3 mb-3">
            <div className="w-9 h-9 rounded-lg bg-zinc-700/40 border border-zinc-600/40 flex items-center justify-center shrink-0">
              <Upload className="w-4 h-4 text-zinc-300" strokeWidth={1.5} />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-zinc-100">Restore</h3>
              <p className="text-xs text-zinc-500 mt-0.5">
                Select a previously exported backup. This overwrites your
                current keystore.
              </p>
            </div>
          </div>
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
            className={`block w-full py-2.5 rounded-lg bg-evap-bg border border-evap-border text-sm font-medium text-center text-zinc-300 hover:border-evap-cyan/40 cursor-pointer transition ${
              stage === "importing" ? "opacity-50 pointer-events-none" : ""
            }`}
          >
            {stage === "importing" ? "Restoring…" : "Choose backup file"}
          </label>
        </div>

        {/* Status messages */}
        {error && (
          <div className="px-4 py-3 rounded-xl bg-evap-red/10 border border-evap-red/30">
            <p className="text-xs text-evap-red">{error}</p>
          </div>
        )}
        {info && (
          <div className="px-4 py-3 rounded-xl bg-evap-green/10 border border-evap-green/30">
            <p className="text-xs text-evap-green">{info}</p>
          </div>
        )}

        {/* Warning */}
        <div className="flex items-start gap-2.5 px-4 py-3 rounded-xl bg-amber-500/5 border border-amber-500/20">
          <AlertTriangle className="w-4 h-4 text-amber-400 shrink-0 mt-0.5" strokeWidth={1.5} />
          <p className="text-xs text-amber-400/90 leading-relaxed">
            Never share your backup file together with your password.
            Anyone with both can access your funds.
          </p>
        </div>
      </div>
    </div>
  );
}
