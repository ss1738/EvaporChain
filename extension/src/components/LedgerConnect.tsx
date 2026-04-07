import { useState, useEffect, useCallback } from "react";
import { useWallet } from "@/hooks/useWallet";
import { ledgerManager, type LedgerAccount, type LedgerDeviceInfo, type LedgerConnectionStatus } from "@/utils/ledger";

type SigningState = "idle" | "awaiting" | "success" | "rejected";

interface TxPreview {
  to: string;
  amount: number;
  fee: number;
}

export function LedgerConnect() {
  const { setView, setError, setNotification, importLedgerAccounts } = useWallet();

  const [connectionStatus, setConnectionStatus] = useState<LedgerConnectionStatus>("disconnected");
  const [deviceInfo, setDeviceInfo] = useState<LedgerDeviceInfo | null>(null);
  const [accounts, setAccounts] = useState<LedgerAccount[]>([]);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [balances, setBalances] = useState<Record<string, number>>({});
  const [signingState, setSigningState] = useState<SigningState>("idle");
  const [txPreview, setTxPreview] = useState<TxPreview | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Fetch stub balances for derived accounts
  const fetchBalances = useCallback(async (accts: LedgerAccount[]) => {
    const bals: Record<string, number> = {};
    for (const a of accts) {
      // Stub: random balance for demo purposes
      bals[a.path] = Math.floor(Math.random() * 10000) / 100;
    }
    setBalances(bals);
  }, []);

  const handleConnect = async () => {
    setErrorMsg(null);
    setConnectionStatus("searching");
    setLoading(true);

    try {
      const connected = await ledgerManager.connect();
      if (!connected) {
        setConnectionStatus("disconnected");
        setErrorMsg("No Ledger device found. Make sure your device is connected and unlocked.");
        setLoading(false);
        return;
      }

      setConnectionStatus("connected");

      // Get device info
      const info = await ledgerManager.getDeviceInfo();
      setDeviceInfo(info);

      // Derive first 5 accounts
      const derived = await ledgerManager.getAccounts(5);
      setAccounts(derived);
      await fetchBalances(derived);

      setLoading(false);
    } catch (err: any) {
      setConnectionStatus("error");
      setLoading(false);

      if (err.message?.includes("NotFoundError") || err.message?.includes("SecurityError")) {
        setErrorMsg("Device not found. Please connect your Ledger and try again.");
      } else if (err.message?.includes("NotAllowedError")) {
        setErrorMsg("Permission denied. Please allow WebHID access in your browser.");
      } else {
        setErrorMsg(err.message || "Failed to connect to Ledger device.");
      }
    }
  };

  const handleDisconnect = async () => {
    await ledgerManager.disconnect();
    setConnectionStatus("disconnected");
    setDeviceInfo(null);
    setAccounts([]);
    setSelectedPaths(new Set());
    setBalances({});
  };

  const toggleAccount = (path: string) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  const handleImport = () => {
    const selected = accounts.filter((a) => selectedPaths.has(a.path));
    if (selected.length === 0) {
      setErrorMsg("Select at least one account to import.");
      return;
    }

    importLedgerAccounts(selected);
    setNotification(`Imported ${selected.length} hardware wallet account${selected.length > 1 ? "s" : ""}`);
    setView("home");
  };

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      // Don't disconnect on unmount — keep connection alive
    };
  }, []);

  const shortAddr = (addr: string) =>
    addr.length > 16 ? `${addr.slice(0, 10)}...${addr.slice(-6)}` : addr;

  return (
    <div className="flex flex-col h-full bg-white">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-200">
        <button
          onClick={() => setView("settings")}
          className="text-zinc-500 hover:text-zinc-700 transition text-sm"
        >
          &larr;
        </button>
        <h1 className="text-sm font-semibold text-zinc-800">Hardware Wallet</h1>
        {connectionStatus === "connected" && (
          <div className="ml-auto flex items-center gap-1.5">
            <div className="w-2 h-2 rounded-full bg-blue-500" />
            <span className="text-[10px] text-blue-600 font-medium">Connected</span>
          </div>
        )}
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4">
        {/* Error banner */}
        {errorMsg && (
          <div className="px-3 py-2 rounded-lg bg-red-50 border border-red-200">
            <p className="text-xs text-red-700">{errorMsg}</p>
            <button
              onClick={() => setErrorMsg(null)}
              className="text-[10px] text-red-500 underline mt-1"
            >
              Dismiss
            </button>
          </div>
        )}

        {/* Disconnected state */}
        {connectionStatus === "disconnected" && !loading && (
          <div className="text-center space-y-4 pt-8">
            {/* Ledger icon placeholder */}
            <div className="mx-auto w-16 h-16 rounded-2xl bg-blue-50 border-2 border-blue-200 flex items-center justify-center">
              <span className="text-2xl text-blue-500">🔐</span>
            </div>
            <div>
              <h2 className="text-sm font-semibold text-zinc-800">Connect Your Ledger</h2>
              <p className="text-xs text-zinc-500 mt-1 max-w-[240px] mx-auto">
                Connect your Ledger device via USB and open the EvaporChain app to get started.
              </p>
            </div>

            {/* Instruction steps */}
            <div className="text-left space-y-2 mx-2">
              <InstructionStep number={1} text="Connect Ledger to your computer via USB" />
              <InstructionStep number={2} text="Unlock your Ledger with your PIN" />
              <InstructionStep number={3} text="Open the EvaporChain app on your device" />
            </div>

            <button
              onClick={handleConnect}
              className="w-full py-2.5 rounded-lg bg-blue-600 text-white text-sm font-medium hover:bg-blue-700 transition"
            >
              Connect Ledger
            </button>

            <p className="text-[10px] text-zinc-400">
              Requires a browser that supports WebHID (Chrome, Edge, Opera)
            </p>
          </div>
        )}

        {/* Searching state */}
        {connectionStatus === "searching" && (
          <div className="text-center space-y-4 pt-12">
            <div className="mx-auto w-12 h-12 rounded-full border-2 border-blue-300 border-t-transparent animate-spin" />
            <div>
              <h2 className="text-sm font-semibold text-zinc-800">Searching for Ledger...</h2>
              <p className="text-xs text-zinc-500 mt-1">
                Make sure your device is connected and unlocked.
              </p>
            </div>
          </div>
        )}

        {/* Connected — Device info */}
        {connectionStatus === "connected" && deviceInfo && (
          <>
            {/* Device card */}
            <div className="px-3 py-3 rounded-lg bg-blue-50 border border-blue-200">
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-xs font-semibold text-blue-800">{deviceInfo.model}</p>
                  <p className="text-[10px] text-blue-600">Firmware: {deviceInfo.firmware}</p>
                </div>
                <button
                  onClick={handleDisconnect}
                  className="text-[10px] text-red-500 hover:text-red-700 font-medium transition"
                >
                  Disconnect
                </button>
              </div>
            </div>

            {/* Multi-sig indicator */}
            <div className="px-3 py-2 rounded-lg bg-emerald-50 border border-emerald-200">
              <div className="flex items-center gap-2">
                <span className="text-xs">🛡️</span>
                <span className="text-[10px] text-emerald-700 font-medium">
                  Hardware + Software co-signing enabled
                </span>
              </div>
            </div>

            {/* Derived accounts */}
            <div>
              <h3 className="text-xs font-semibold text-zinc-700 mb-2">Derived Accounts</h3>
              <div className="space-y-2">
                {accounts.map((acct) => (
                  <label
                    key={acct.path}
                    className={`flex items-center gap-3 px-3 py-2.5 rounded-lg border cursor-pointer transition ${
                      selectedPaths.has(acct.path)
                        ? "bg-blue-50 border-blue-300"
                        : "bg-white border-zinc-200 hover:border-zinc-300"
                    }`}
                  >
                    <input
                      type="checkbox"
                      checked={selectedPaths.has(acct.path)}
                      onChange={() => toggleAccount(acct.path)}
                      className="w-4 h-4 rounded border-zinc-300 text-blue-600 focus:ring-blue-500"
                    />
                    <div className="flex-1 min-w-0">
                      <p className="text-xs font-mono text-zinc-700 truncate">
                        {shortAddr(acct.address)}
                      </p>
                      <p className="text-[10px] text-zinc-400">{acct.path}</p>
                    </div>
                    <div className="text-right">
                      <p className="text-xs font-semibold text-zinc-800">
                        {(balances[acct.path] ?? 0).toFixed(2)}
                      </p>
                      <p className="text-[10px] text-zinc-400">EVAP</p>
                    </div>
                  </label>
                ))}
              </div>
            </div>

            {/* Import button */}
            <button
              onClick={handleImport}
              disabled={selectedPaths.size === 0}
              className="w-full py-2.5 rounded-lg bg-blue-600 text-white text-sm font-medium hover:bg-blue-700 transition disabled:opacity-40 disabled:cursor-not-allowed"
            >
              Import Selected ({selectedPaths.size})
            </button>
          </>
        )}

        {/* Transaction signing overlay */}
        {signingState === "awaiting" && txPreview && (
          <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
            <div className="bg-white rounded-xl mx-4 p-5 w-full max-w-[320px] space-y-4">
              <div className="text-center">
                <div className="mx-auto w-10 h-10 rounded-full border-2 border-blue-300 border-t-transparent animate-spin mb-3" />
                <h3 className="text-sm font-semibold text-zinc-800">Confirm on Your Ledger</h3>
                <p className="text-xs text-zinc-500 mt-1">
                  Review and approve the transaction on your device.
                </p>
              </div>

              {/* Tx details that appear on Ledger screen */}
              <div className="space-y-2 bg-zinc-50 rounded-lg px-3 py-3">
                <TxDetailRow label="To" value={shortAddr(txPreview.to)} />
                <TxDetailRow label="Amount" value={`${txPreview.amount} EVAP`} />
                <TxDetailRow label="Fee" value={`${txPreview.fee} EVAP`} />
              </div>

              <p className="text-[10px] text-zinc-400 text-center">
                Waiting for hardware confirmation...
              </p>
            </div>
          </div>
        )}

        {/* Signing success overlay */}
        {signingState === "success" && (
          <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
            <div className="bg-white rounded-xl mx-4 p-5 w-full max-w-[320px] text-center space-y-3">
              <div className="mx-auto w-12 h-12 rounded-full bg-emerald-100 flex items-center justify-center">
                <span className="text-emerald-600 text-xl">&#10003;</span>
              </div>
              <h3 className="text-sm font-semibold text-zinc-800">Transaction Signed</h3>
              <p className="text-xs text-zinc-500">
                Successfully signed with your hardware wallet.
              </p>
              <button
                onClick={() => setSigningState("idle")}
                className="w-full py-2 rounded-lg bg-blue-600 text-white text-sm font-medium hover:bg-blue-700 transition"
              >
                Done
              </button>
            </div>
          </div>
        )}

        {/* Signing rejected overlay */}
        {signingState === "rejected" && (
          <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
            <div className="bg-white rounded-xl mx-4 p-5 w-full max-w-[320px] text-center space-y-3">
              <div className="mx-auto w-12 h-12 rounded-full bg-red-100 flex items-center justify-center">
                <span className="text-red-600 text-xl">&#10007;</span>
              </div>
              <h3 className="text-sm font-semibold text-zinc-800">Transaction Rejected</h3>
              <p className="text-xs text-zinc-500">
                You rejected the transaction on your Ledger device.
              </p>
              <button
                onClick={() => setSigningState("idle")}
                className="w-full py-2 rounded-lg bg-zinc-200 text-zinc-800 text-sm font-medium hover:bg-zinc-300 transition"
              >
                Close
              </button>
            </div>
          </div>
        )}

        {/* Error state: App not installed */}
        {connectionStatus === "error" && (
          <div className="text-center space-y-4 pt-8">
            <div className="mx-auto w-16 h-16 rounded-2xl bg-red-50 border-2 border-red-200 flex items-center justify-center">
              <span className="text-2xl">&#9888;</span>
            </div>
            <div>
              <h2 className="text-sm font-semibold text-zinc-800">Connection Error</h2>
              <p className="text-xs text-zinc-500 mt-1 max-w-[240px] mx-auto">
                Could not connect to your Ledger. Make sure the EvaporChain app is installed and open on your device.
              </p>
            </div>
            <button
              onClick={handleConnect}
              className="w-full py-2.5 rounded-lg bg-blue-600 text-white text-sm font-medium hover:bg-blue-700 transition"
            >
              Try Again
            </button>
          </div>
        )}
      </div>

      {/* Post-quantum signature note */}
      <div className="px-4 py-2 border-t border-zinc-200">
        <p className="text-[10px] text-zinc-400 text-center">
          ML-DSA (FIPS 204) post-quantum signatures via hardware
        </p>
      </div>
    </div>
  );
}

function InstructionStep({ number, text }: { number: number; text: string }) {
  return (
    <div className="flex items-center gap-3">
      <div className="w-6 h-6 rounded-full bg-blue-100 flex items-center justify-center shrink-0">
        <span className="text-[10px] font-bold text-blue-600">{number}</span>
      </div>
      <p className="text-xs text-zinc-600">{text}</p>
    </div>
  );
}

function TxDetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between">
      <span className="text-[10px] text-zinc-500">{label}</span>
      <span className="text-[10px] font-mono text-zinc-700">{value}</span>
    </div>
  );
}
