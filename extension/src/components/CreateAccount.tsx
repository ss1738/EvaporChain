import { useState, useMemo } from "react";
import { Eye, EyeOff } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";

type Strength = { score: 0 | 1 | 2 | 3 | 4; label: string; color: string };

function scorePassword(p: string): Strength {
  if (!p) return { score: 0, label: "", color: "bg-zinc-700" };
  let s = 0;
  if (p.length >= 8) s++;
  if (p.length >= 12) s++;
  if (/[A-Z]/.test(p) && /[a-z]/.test(p)) s++;
  if (/[0-9]/.test(p)) s++;
  if (/[^A-Za-z0-9]/.test(p)) s++;
  const map: Strength[] = [
    { score: 0, label: "", color: "bg-zinc-700" },
    { score: 1, label: "Weak", color: "bg-evap-red" },
    { score: 2, label: "Fair", color: "bg-amber-500" },
    { score: 3, label: "Good", color: "bg-evap-cyan" },
    { score: 4, label: "Strong", color: "bg-evap-green" },
  ];
  const clamped = Math.min(s - 1, 4) as 0 | 1 | 2 | 3 | 4;
  return map[Math.max(clamped, 1)] ?? map[1];
}

export function CreateAccount() {
  const { createAccount, error, loading, setView } = useWallet();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [showPw, setShowPw] = useState(false);
  const [localError, setLocalError] = useState("");

  const strength = useMemo(() => scorePassword(password), [password]);
  const passwordsMatch = confirm.length > 0 && password === confirm;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLocalError("");
    if (!name.trim()) return setLocalError("Name is required");
    if (password.length < 8) return setLocalError("Password must be at least 8 characters");
    if (password !== confirm) return setLocalError("Passwords don't match");
    try {
      await createAccount(name.trim(), password);
    } catch {
      // Error handled by store
    }
  };

  return (
    <div className="flex flex-col items-center justify-center h-full px-6 py-8">
      <div className="w-12 h-12 rounded-xl bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center mb-5 ring-1 ring-evap-cyan/20">
        <span className="text-base font-semibold text-black">E</span>
      </div>
      <h1 className="text-xl font-semibold text-zinc-100 mb-1.5">Create Wallet</h1>
      <p className="text-sm text-zinc-500 mb-6">Generate a new account on EvaporChain</p>

      <form onSubmit={handleSubmit} className="w-full space-y-3">
        <div>
          <label className="text-xs font-medium text-zinc-400 mb-1.5 block">Name</label>
          <input
            type="text"
            placeholder="e.g. main"
            value={name}
            onChange={e => setName(e.target.value)}
            className="w-full px-3.5 py-2.5 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-100 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
            autoFocus
          />
        </div>

        <div>
          <label className="text-xs font-medium text-zinc-400 mb-1.5 block">Password</label>
          <div className="relative">
            <input
              type={showPw ? "text" : "password"}
              placeholder="At least 8 characters"
              value={password}
              onChange={e => setPassword(e.target.value)}
              className="w-full px-3.5 py-2.5 pr-10 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-100 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
            />
            <button
              type="button"
              onClick={() => setShowPw(v => !v)}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 p-1 text-zinc-500 hover:text-zinc-300 transition"
              tabIndex={-1}
              aria-label={showPw ? "Hide password" : "Show password"}
            >
              {showPw
                ? <EyeOff className="w-4 h-4" strokeWidth={1.5} />
                : <Eye className="w-4 h-4" strokeWidth={1.5} />}
            </button>
          </div>
          {password.length > 0 && (
            <div className="mt-2">
              <div className="flex gap-1">
                {[1, 2, 3, 4].map(i => (
                  <div
                    key={i}
                    className={`h-1 flex-1 rounded-full transition ${
                      i <= strength.score ? strength.color : "bg-zinc-800"
                    }`}
                  />
                ))}
              </div>
              {strength.label && (
                <p className={`text-xs mt-1 ${strength.color.replace("bg-", "text-")}`}>
                  {strength.label}
                </p>
              )}
            </div>
          )}
        </div>

        <div>
          <label className="text-xs font-medium text-zinc-400 mb-1.5 block">Confirm password</label>
          <input
            type={showPw ? "text" : "password"}
            placeholder="Re-enter password"
            value={confirm}
            onChange={e => setConfirm(e.target.value)}
            className={`w-full px-3.5 py-2.5 rounded-lg bg-evap-surface border text-sm text-zinc-100 placeholder-zinc-600 focus:outline-none transition ${
              confirm.length > 0 && !passwordsMatch
                ? "border-evap-red/60 focus:border-evap-red"
                : "border-evap-border focus:border-evap-cyan"
            }`}
          />
          {confirm.length > 0 && !passwordsMatch && (
            <p className="text-xs text-evap-red mt-1">Passwords don't match</p>
          )}
        </div>

        {(localError || error) && (
          <p className="text-xs text-evap-red">{localError || error}</p>
        )}

        <button
          type="submit"
          disabled={loading || !name || !password || !passwordsMatch}
          className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-40"
        >
          {loading ? "Creating..." : "Create Wallet"}
        </button>
      </form>

      <button
        onClick={() => setView("import")}
        className="mt-5 text-xs text-evap-cyan hover:underline"
      >
        Already have a wallet? Import instead
      </button>

      <p className="mt-5 text-xs text-zinc-600 text-center max-w-[240px]">
        Keys are encrypted with AES-256-GCM and never leave your browser.
      </p>
    </div>
  );
}
