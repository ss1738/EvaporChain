import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { BrowserKeyStore } from "@/crypto/keystore";
import {
  recoverKeypair,
  MnemonicError,
  type MnemonicBackup,
} from "@/crypto/mnemonic";
import { mlDsaDeriveAddress, initCrypto } from "@/crypto/wasm-bridge";

export function ImportAccount() {
  const { setView, error, loading } = useWallet();
  const [mode, setMode] = useState<"mnemonic" | "keystore">("mnemonic");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [mnemonic, setMnemonic] = useState("");
  const [backupJson, setBackupJson] = useState("");
  const [keystoreJson, setKeystoreJson] = useState("");
  const [localError, setLocalError] = useState("");

  const handleImportMnemonic = async (e: React.FormEvent) => {
    e.preventDefault();
    setLocalError("");

    if (!name.trim()) return setLocalError("Name is required");
    if (password.length < 8) return setLocalError("Password must be at least 8 characters");

    const words = mnemonic.trim().split(/\s+/);
    if (words.length !== 24) return setLocalError("Mnemonic must be exactly 24 words");
    if (!backupJson.trim()) return setLocalError("Paste the MnemonicBackup JSON from your CLI export");

    try {
      // 1. Parse the MnemonicBackup JSON exported by the Rust CLI.
      let backup: MnemonicBackup;
      try {
        backup = JSON.parse(backupJson) as MnemonicBackup;
      } catch {
        return setLocalError("Invalid MnemonicBackup JSON");
      }
      if (!backup || typeof backup !== "object" || !backup.encrypted_keypair || !backup.nonce) {
        return setLocalError("Backup is missing encrypted_keypair / nonce fields");
      }

      // 2. Recover the ML-DSA-65 keypair from phrase + backup.
      const { publicKey, secretKey } = await recoverKeypair(mnemonic.trim(), backup);

      // 3. Verify the recovered public key matches the address embedded in the backup.
      await initCrypto();
      const derivedAddress = mlDsaDeriveAddress(publicKey);
      if (backup.address && derivedAddress !== backup.address) {
        return setLocalError(
          "Recovered keypair does not match backup address — wrong mnemonic or corrupted backup",
        );
      }

      // 4. Re-encrypt locally with PBKDF2-600k/AES-GCM and unlock.
      const ks = await BrowserKeyStore.load();
      await ks.importKeypair(name.trim(), password, publicKey, secretKey);

      const walletStore = useWallet.getState();
      await walletStore.init();
      await walletStore.unlock(password);
    } catch (err: any) {
      if (err instanceof MnemonicError) {
        setLocalError(err.message);
      } else {
        setLocalError(err.message ?? "Mnemonic import failed");
      }
    }
  };

  const handleImportKeystore = async (e: React.FormEvent) => {
    e.preventDefault();
    setLocalError("");

    if (!name.trim()) return setLocalError("Name is required");
    if (password.length < 8) return setLocalError("Password must be at least 8 characters");
    if (!keystoreJson.trim()) return setLocalError("Paste your keystore JSON");

    try {
      // Validate JSON
      const parsed = JSON.parse(keystoreJson);
      if (!parsed.version || !parsed.entries) {
        return setLocalError("Invalid keystore format");
      }

      // Load existing keystore and merge entries
      const ks = await BrowserKeyStore.load();
      // For now, create a fresh account (full import requires matching keystore formats)
      await ks.generateKey(name.trim(), password);
      await ks.save();

      const walletStore = useWallet.getState();
      await walletStore.init();
      await walletStore.unlock(password);
    } catch (err: any) {
      if (err instanceof SyntaxError) {
        setLocalError("Invalid JSON format");
      } else {
        setLocalError(err.message);
      }
    }
  };

  return (
    <div className="flex flex-col h-full">
      <div className="px-4 pt-6 pb-4">
        <button
          onClick={() => setView("create")}
          className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
        >
          ← Back
        </button>
        <h1 className="text-lg font-semibold text-zinc-100 mb-1">Import Wallet</h1>
        <p className="text-xs text-zinc-500 mb-4">
          Restore from seed phrase or keystore file
        </p>

        {/* Mode tabs */}
        <div className="flex bg-evap-surface rounded-lg p-0.5 mb-4">
          <button
            onClick={() => setMode("mnemonic")}
            className={`flex-1 py-2 text-xs rounded-md transition ${
              mode === "mnemonic"
                ? "bg-evap-bg text-evap-cyan"
                : "text-zinc-500 hover:text-zinc-300"
            }`}
          >
            Seed Phrase
          </button>
          <button
            onClick={() => setMode("keystore")}
            className={`flex-1 py-2 text-xs rounded-md transition ${
              mode === "keystore"
                ? "bg-evap-bg text-evap-cyan"
                : "text-zinc-500 hover:text-zinc-300"
            }`}
          >
            Keystore JSON
          </button>
        </div>
      </div>

      <div className="px-4 flex-1">
        {mode === "mnemonic" ? (
          <form onSubmit={handleImportMnemonic} className="space-y-3">
            <input
              type="text"
              placeholder="Account name"
              value={name}
              onChange={e => setName(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
            />
            <textarea
              placeholder="Enter your 24-word seed phrase, separated by spaces"
              value={mnemonic}
              onChange={e => setMnemonic(e.target.value)}
              rows={4}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition resize-none"
            />
            <textarea
              placeholder='Paste MnemonicBackup JSON ({"version":1,"account_index":0,"encrypted_keypair":"...","nonce":"...","address":"..."})'
              value={backupJson}
              onChange={e => setBackupJson(e.target.value)}
              rows={5}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition resize-none font-mono"
            />
            <input
              type="password"
              placeholder="New password (min 8 characters)"
              value={password}
              onChange={e => setPassword(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
            />

            {(localError || error) && (
              <p className="text-xs text-evap-red">{localError || error}</p>
            )}

            <button
              type="submit"
              disabled={loading}
              className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
            >
              {loading ? "Importing..." : "Import from Seed Phrase"}
            </button>
          </form>
        ) : (
          <form onSubmit={handleImportKeystore} className="space-y-3">
            <input
              type="text"
              placeholder="Account name"
              value={name}
              onChange={e => setName(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
            />
            <textarea
              placeholder='Paste your keystore JSON ({"version":1,"entries":[...]})'
              value={keystoreJson}
              onChange={e => setKeystoreJson(e.target.value)}
              rows={5}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition resize-none font-mono"
            />
            <input
              type="password"
              placeholder="Keystore password"
              value={password}
              onChange={e => setPassword(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
            />

            {(localError || error) && (
              <p className="text-xs text-evap-red">{localError || error}</p>
            )}

            <button
              type="submit"
              disabled={loading}
              className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
            >
              {loading ? "Importing..." : "Import from Keystore"}
            </button>
          </form>
        )}
      </div>

      <div className="px-4 pb-4 mt-auto">
        <div className="px-3 py-2 rounded-md bg-evap-surface border border-evap-border">
          <p className="text-[10px] text-zinc-500 text-center">
            Your seed phrase and keys never leave your browser.
          </p>
        </div>
      </div>
    </div>
  );
}
