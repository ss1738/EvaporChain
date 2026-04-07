import { useWallet } from "@/hooks/useWallet";
import { Header } from "./Header";

export function ReceiveScreen() {
  const { activeAccount, setView, setNotification } = useWallet();

  if (!activeAccount) return null;

  const copyAddress = () => {
    navigator.clipboard.writeText(activeAccount.address);
    setNotification("Address copied!");
  };

  return (
    <div className="flex flex-col h-full">
      <Header />
      <div className="px-4 pt-4">
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
        >
          ← Back
        </button>
        <h2 className="text-lg font-semibold text-zinc-100 mb-1">Receive EVAP</h2>
        <p className="text-xs text-zinc-500 mb-6">
          Share your address to receive EVAP tokens
        </p>
      </div>

      <div className="flex flex-col items-center px-4">
        {/* QR placeholder */}
        <div className="w-48 h-48 rounded-lg bg-white p-3 mb-4">
          <div className="w-full h-full bg-evap-surface rounded flex items-center justify-center">
            <span className="text-xs text-zinc-500">QR Code</span>
          </div>
        </div>

        {/* Address */}
        <div className="w-full px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-[10px] text-zinc-500 mb-1">Your Address</p>
          <p className="text-xs font-mono text-zinc-300 break-all leading-relaxed">
            {activeAccount.address}
          </p>
        </div>

        <button
          onClick={copyAddress}
          className="mt-3 w-full py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-300 hover:border-evap-cyan/40 transition"
        >
          Copy Address
        </button>
      </div>
    </div>
  );
}
