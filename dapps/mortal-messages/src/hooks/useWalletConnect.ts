import { useState, useEffect, useCallback } from "react";

export function useWalletConnect() {
  const [address, setAddress] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const check = async () => {
      try {
        if (window.evaporchain) {
          const isConn = await window.evaporchain.isConnected();
          if (isConn) {
            const addr = await window.evaporchain.getAddress();
            setAddress(addr);
            setConnected(true);
          }
        }
      } catch {
        /* wallet not available */
      } finally {
        setLoading(false);
      }
    };
    check();
  }, []);

  const connect = useCallback(async () => {
    if (!window.evaporchain) {
      alert("EvaporChain wallet extension not found. Please install it first.");
      return;
    }
    try {
      const { address: addr } = await window.evaporchain.connect();
      setAddress(addr);
      setConnected(true);
    } catch (err) {
      console.error("Wallet connect failed:", err);
    }
  }, []);

  const disconnect = useCallback(async () => {
    if (window.evaporchain) {
      await window.evaporchain.disconnect();
    }
    setAddress(null);
    setConnected(false);
  }, []);

  return { address, connected, loading, connect, disconnect };
}
