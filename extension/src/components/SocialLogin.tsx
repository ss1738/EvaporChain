import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";

export function SocialLogin() {
  const { socialLogin, setView, loading, error } = useWallet();
  const [activeProvider, setActiveProvider] = useState<"google" | "apple" | null>(null);

  const handleSocialLogin = async (provider: "google" | "apple") => {
    setActiveProvider(provider);
    try {
      await socialLogin(provider);
      // After successful social login, the store will navigate to tutorial or home
    } catch {
      // Error is set in the store
    } finally {
      setActiveProvider(null);
    }
  };

  return (
    <div className="flex flex-col h-full bg-evap-bg">
      {/* Top section */}
      <div className="flex-1 flex flex-col items-center justify-center px-6">
        {/* Logo area */}
        <div className="w-16 h-16 rounded-2xl bg-evap-surface border border-evap-border flex items-center justify-center mb-6">
          <span className="text-2xl text-evap-cyan font-bold">EC</span>
        </div>

        <h1 className="text-xl font-bold text-zinc-100 text-center mb-1">
          Welcome to EvaporChain
        </h1>
        <p className="text-sm text-zinc-500 text-center mb-8">
          Get started in seconds
        </p>

        {/* Error banner */}
        {error && (
          <div className="w-full mb-4 px-3 py-2 rounded-lg bg-red-500/10 border border-red-500/30">
            <p className="text-xs text-red-400 text-center">{error}</p>
          </div>
        )}

        {/* Social login buttons */}
        <div className="w-full space-y-3">
          {/* Google button */}
          <button
            onClick={() => handleSocialLogin("google")}
            disabled={loading}
            className="w-full flex items-center justify-center gap-3 py-3 px-4 rounded-xl bg-white text-zinc-800 text-sm font-medium hover:bg-zinc-100 transition border border-zinc-200 disabled:opacity-50"
          >
            <span className="text-lg font-bold" style={{ color: "#4285F4" }}>G</span>
            <span>{activeProvider === "google" ? "Connecting..." : "Continue with Google"}</span>
          </button>

          {/* Apple button */}
          <button
            onClick={() => handleSocialLogin("apple")}
            disabled={loading}
            className="w-full flex items-center justify-center gap-3 py-3 px-4 rounded-xl bg-zinc-900 text-white text-sm font-medium hover:bg-zinc-800 transition border border-zinc-700 disabled:opacity-50"
          >
            <span className="text-lg">{"\uF8FF"}</span>
            <span>{activeProvider === "apple" ? "Connecting..." : "Continue with Apple"}</span>
          </button>
        </div>

        {/* Divider */}
        <div className="flex items-center gap-3 w-full my-6">
          <div className="flex-1 h-px bg-evap-border" />
          <span className="text-xs text-zinc-500">or</span>
          <div className="flex-1 h-px bg-evap-border" />
        </div>

        {/* Seed phrase link */}
        <button
          onClick={() => setView("create")}
          className="text-sm text-evap-cyan hover:text-evap-cyan/80 transition"
        >
          Create with Seed Phrase
        </button>

        <button
          onClick={() => setView("import")}
          className="text-xs text-zinc-500 hover:text-zinc-300 transition mt-2"
        >
          Import existing wallet
        </button>
      </div>

      {/* Reassurance text */}
      <div className="px-6 pb-6">
        <div className="px-4 py-3 rounded-xl bg-evap-surface border border-evap-border">
          <p className="text-[11px] text-zinc-400 text-center leading-relaxed">
            Your wallet is secured on this device. No one else has access.
          </p>
        </div>
      </div>
    </div>
  );
}
