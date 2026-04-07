import { useMemo } from "react";
import { useWallet } from "@/hooks/useWallet";
import { Header } from "./Header";
import { generateQRDataUrl } from "@/utils/qr";

export function ReceiveScreen() {
  const { activeAccount, setView, setNotification } = useWallet();

  const qrDataUrl = useMemo(() => {
    if (!activeAccount) return null;
    return generateQRDataUrl(activeAccount.address, 200);
  }, [activeAccount]);

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
          Share your address or QR code to receive EVAP tokens
        </p>
      </div>

      <div className="flex flex-col items-center px-4">
        {/* QR Code */}
        <div className="w-52 h-52 rounded-xl bg-white p-3 mb-4 shadow-lg shadow-evap-cyan/5">
          {qrDataUrl ? (
            <img
              src={qrDataUrl}
              alt="Wallet address QR code"
              className="w-full h-full rounded"
            />
          ) : (
            <div className="w-full h-full flex items-center justify-center">
              <div className="w-5 h-5 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
            </div>
          )}
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
          className="mt-3 w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition"
        >
          Copy Address
        </button>

        <p className="text-[10px] text-zinc-600 mt-3 text-center">
          Only send EVAP to this address. Sending other tokens may result in loss.
        </p>
      </div>
    </div>
  );
}
