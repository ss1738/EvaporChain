import { useState, useRef, useEffect } from "react";
import { useWallet } from "@/hooks/useWallet";
import {
  aiEngine,
  createMessageId,
  type ChatMessage,
  type AiAction,
  type WalletApi,
} from "@/utils/ai";

export function AiAssistant() {
  const {
    setView, balance, objects, nfts, chainStatus,
    activeAccount, sendTransfer, loading, refreshObjects,
  } = useWallet();

  const [messages, setMessages] = useState<ChatMessage[]>([
    {
      id: createMessageId(),
      role: "assistant",
      text: "Hi! I'm your EvaporChain assistant. Ask me anything about your wallet, objects, or transactions. Type \"help\" to see what I can do.",
      timestamp: Date.now(),
    },
  ]);
  const [input, setInput] = useState("");
  const [pendingAction, setPendingAction] = useState<{ msgId: string; action: AiAction } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    refreshObjects();
  }, [refreshObjects]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [messages]);

  const walletApi: WalletApi = {
    getBalance: () => balance,
    getAddress: () => activeAccount?.address ?? "",
    getObjects: () => objects,
    getNfts: () => nfts,
    getChainStatus: () => chainStatus,
  };

  const suggestions = aiEngine.getSuggestions(walletApi);

  const handleSend = (text?: string) => {
    const msg = (text ?? input).trim();
    if (!msg) return;

    const userMsg: ChatMessage = {
      id: createMessageId(),
      role: "user",
      text: msg,
      timestamp: Date.now(),
    };

    const parsed = aiEngine.parseCommand(msg);
    const response = aiEngine.executeIntent(parsed.intent, parsed.params, walletApi);

    const assistantMsg: ChatMessage = {
      id: createMessageId(),
      role: "assistant",
      text: response.message,
      action: response.action,
      timestamp: Date.now(),
    };

    setMessages(prev => [...prev, userMsg, assistantMsg]);
    setInput("");

    // Track if there's a confirmable action
    if (
      response.action &&
      (response.action.type === "preview_transfer" ||
        response.action.type === "preview_refresh" ||
        response.action.type === "preview_bridge")
    ) {
      setPendingAction({ msgId: assistantMsg.id, action: response.action });
    }
  };

  const handleConfirm = async () => {
    if (!pendingAction) return;
    const { action } = pendingAction;

    if (action.type === "preview_transfer") {
      const result = await sendTransfer(action.to, action.amount);
      const confirmMsg: ChatMessage = {
        id: createMessageId(),
        role: "assistant",
        text: result.success
          ? `Transaction sent successfully! ${action.amount} EVAP transferred to ${action.to.slice(0, 8)}...`
          : `Transaction failed: ${result.message}`,
        timestamp: Date.now(),
      };
      setMessages(prev => [...prev, confirmMsg]);
    } else if (action.type === "preview_refresh") {
      const confirmMsg: ChatMessage = {
        id: createMessageId(),
        role: "assistant",
        text: `Refresh request submitted for ${action.objectName}. Check the Objects view for status.`,
        timestamp: Date.now(),
      };
      setMessages(prev => [...prev, confirmMsg]);
    } else if (action.type === "preview_bridge") {
      const confirmMsg: ChatMessage = {
        id: createMessageId(),
        role: "assistant",
        text: `Bridge request submitted. ${action.amount} EVAP will be bridged to ${action.chain}. This may take 10-30 minutes.`,
        timestamp: Date.now(),
      };
      setMessages(prev => [...prev, confirmMsg]);
    }

    setPendingAction(null);
  };

  const handleCancel = () => {
    const cancelMsg: ChatMessage = {
      id: createMessageId(),
      role: "assistant",
      text: "Transaction cancelled. Is there anything else I can help with?",
      timestamp: Date.now(),
    };
    setMessages(prev => [...prev, cancelMsg]);
    setPendingAction(null);
  };

  return (
    <div className="flex flex-col h-full bg-zinc-50">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 bg-white border-b border-zinc-100">
        <button
          onClick={() => setView("home")}
          className="w-8 h-8 flex items-center justify-center rounded-lg hover:bg-zinc-50 text-zinc-600 transition"
        >
          <span className="text-lg">&larr;</span>
        </button>
        <div className="flex items-center gap-2">
          <div className="w-7 h-7 rounded-full bg-gradient-to-br from-cyan-400 to-cyan-600 flex items-center justify-center">
            <span className="text-white text-xs font-bold">AI</span>
          </div>
          <div>
            <h1 className="text-sm font-semibold text-zinc-800">AI Assistant</h1>
            <p className="text-xs text-zinc-400">Natural language wallet</p>
          </div>
        </div>
      </div>

      {/* Messages */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-4 py-3 space-y-3">
        {messages.map(msg => (
          <MessageBubble
            key={msg.id}
            message={msg}
            isPending={pendingAction?.msgId === msg.id}
            onConfirm={handleConfirm}
            onCancel={handleCancel}
            loading={loading}
          />
        ))}
      </div>

      {/* Suggested prompts */}
      {messages.length <= 2 && (
        <div className="flex gap-1.5 px-4 pb-2 overflow-x-auto scrollbar-hide">
          {suggestions.map((s, i) => (
            <button
              key={i}
              onClick={() => handleSend(s)}
              className="px-3 py-1.5 rounded-full bg-white border border-zinc-200 text-xs text-zinc-600 font-medium whitespace-nowrap hover:border-cyan-300 hover:text-cyan-700 transition"
            >
              {s}
            </button>
          ))}
        </div>
      )}

      {/* Input */}
      <div className="px-4 py-3 bg-white border-t border-zinc-100">
        <div className="flex gap-2">
          <input
            type="text"
            placeholder="Ask anything..."
            value={input}
            onChange={e => setInput(e.target.value)}
            onKeyDown={e => e.key === "Enter" && handleSend()}
            className="flex-1 px-3 py-2.5 rounded-xl bg-zinc-50 border border-zinc-200 text-sm text-zinc-800 placeholder:text-zinc-400 focus:outline-none focus:border-cyan-400 focus:ring-1 focus:ring-cyan-400/30 transition"
          />
          <button
            onClick={() => handleSend()}
            disabled={!input.trim()}
            className="w-10 h-10 rounded-xl bg-cyan-600 text-white flex items-center justify-center hover:bg-cyan-700 disabled:bg-zinc-200 disabled:text-zinc-400 transition"
          >
            <span className="text-sm font-bold">&uarr;</span>
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Message Bubble ──

function MessageBubble({
  message,
  isPending,
  onConfirm,
  onCancel,
  loading,
}: {
  message: ChatMessage;
  isPending: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  loading: boolean;
}) {
  const isUser = message.role === "user";

  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[85%] px-3.5 py-2.5 rounded-2xl shadow-sm ${
          isUser
            ? "bg-cyan-600 text-white rounded-br-md"
            : "bg-white text-zinc-800 border border-zinc-100 rounded-bl-md"
        }`}
      >
        {/* Message text with basic markdown-like formatting */}
        <div className={`text-[13px] leading-relaxed whitespace-pre-wrap ${isUser ? "" : ""}`}>
          {renderFormattedText(message.text, isUser)}
        </div>

        {/* Action card */}
        {message.action && message.action.type !== "none" && (
          <ActionCard action={message.action} />
        )}

        {/* Confirm/Cancel buttons for pending actions */}
        {isPending && (
          <div className="flex gap-2 mt-2.5 pt-2 border-t border-zinc-100">
            <button
              onClick={onConfirm}
              disabled={loading}
              className="flex-1 py-2 rounded-lg bg-cyan-600 text-white text-xs font-semibold hover:bg-cyan-700 disabled:opacity-50 transition"
            >
              {loading ? "Sending..." : "Confirm"}
            </button>
            <button
              onClick={onCancel}
              disabled={loading}
              className="flex-1 py-2 rounded-lg bg-zinc-100 text-zinc-600 text-xs font-semibold hover:bg-zinc-200 transition"
            >
              Cancel
            </button>
          </div>
        )}

        {/* Timestamp */}
        <p className={`text-[10px] mt-1.5 ${isUser ? "text-cyan-200" : "text-zinc-300"}`}>
          {new Date(message.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
        </p>
      </div>
    </div>
  );
}

// ── Formatted text (bold markers) ──

function renderFormattedText(text: string, isUser: boolean) {
  // Split on **bold** markers
  const parts = text.split(/(\*\*.*?\*\*)/g);
  return parts.map((part, i) => {
    if (part.startsWith("**") && part.endsWith("**")) {
      return (
        <span key={i} className={`font-semibold ${isUser ? "text-white" : "text-zinc-900"}`}>
          {part.slice(2, -2)}
        </span>
      );
    }
    // Handle backtick code spans
    const codeParts = part.split(/(`.*?`)/g);
    if (codeParts.length > 1) {
      return codeParts.map((cp, j) => {
        if (cp.startsWith("`") && cp.endsWith("`")) {
          return (
            <code
              key={`${i}-${j}`}
              className={`px-1 py-0.5 rounded text-xs font-mono ${
                isUser ? "bg-cyan-700/50" : "bg-zinc-100"
              }`}
            >
              {cp.slice(1, -1)}
            </code>
          );
        }
        return <span key={`${i}-${j}`}>{cp}</span>;
      });
    }
    return <span key={i}>{part}</span>;
  });
}

// ── Action Card ──

function ActionCard({ action }: { action: AiAction }) {
  switch (action.type) {
    case "preview_transfer":
      return (
        <div className="mt-2 p-3 rounded-lg bg-cyan-50 border border-cyan-200">
          <p className="text-xs font-semibold text-cyan-700 uppercase tracking-wide mb-1">Transfer Preview</p>
          <div className="space-y-1">
            <div className="flex justify-between">
              <span className="text-xs text-zinc-500">Amount</span>
              <span className="text-xs font-semibold text-zinc-800">{action.amount} EVAP</span>
            </div>
            <div className="flex justify-between">
              <span className="text-xs text-zinc-500">To</span>
              <span className="text-xs font-mono text-zinc-600">
                {action.to.slice(0, 10)}...{action.to.slice(-6)}
              </span>
            </div>
          </div>
        </div>
      );

    case "preview_refresh":
      return (
        <div className="mt-2 p-3 rounded-lg bg-amber-50 border border-amber-200">
          <p className="text-xs font-semibold text-amber-700 uppercase tracking-wide mb-1">Refresh Preview</p>
          <div className="space-y-1">
            <div className="flex justify-between">
              <span className="text-xs text-zinc-500">Object</span>
              <span className="text-xs font-semibold text-zinc-800">{action.objectName}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-xs text-zinc-500">Energy</span>
              <span className="text-xs font-semibold text-zinc-800">{action.energy} units</span>
            </div>
          </div>
        </div>
      );

    case "preview_bridge":
      return (
        <div className="mt-2 p-3 rounded-lg bg-violet-50 border border-violet-200">
          <p className="text-xs font-semibold text-violet-700 uppercase tracking-wide mb-1">Bridge Preview</p>
          <div className="space-y-1">
            <div className="flex justify-between">
              <span className="text-xs text-zinc-500">Amount</span>
              <span className="text-xs font-semibold text-zinc-800">{action.amount} EVAP</span>
            </div>
            <div className="flex justify-between">
              <span className="text-xs text-zinc-500">Destination</span>
              <span className="text-xs font-semibold text-zinc-800">{action.chain}</span>
            </div>
          </div>
        </div>
      );

    case "show_balance":
      return (
        <div className="mt-2 p-3 rounded-lg bg-emerald-50 border border-emerald-200">
          <div className="flex items-baseline gap-2">
            <span className="text-lg font-bold text-emerald-700">{action.balance.toLocaleString()}</span>
            <span className="text-xs text-emerald-500">EVAP</span>
          </div>
        </div>
      );

    case "show_chain_status":
      if (!action.status) return null;
      return (
        <div className="mt-2 p-3 rounded-lg bg-zinc-50 border border-zinc-200">
          <div className="grid grid-cols-2 gap-2">
            <MiniStat label="Block" value={action.status.block_height?.toLocaleString()} />
            <MiniStat label="Epoch" value={action.status.epoch?.toString()} />
            <MiniStat label="Objects" value={action.status.active_objects?.toLocaleString()} />
            <MiniStat label="Peers" value={action.status.peer_count?.toString()} />
          </div>
        </div>
      );

    case "refresh_strategy":
      if (!action.recommendations || action.recommendations.length === 0) return null;
      return (
        <div className="mt-2 space-y-1">
          {action.recommendations.slice(0, 5).map((r, i) => (
            <div
              key={i}
              className={`p-2 rounded-lg border ${
                r.urgency === "critical"
                  ? "bg-red-50 border-red-200"
                  : r.urgency === "warning"
                  ? "bg-amber-50 border-amber-200"
                  : "bg-zinc-50 border-zinc-200"
              }`}
            >
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium text-zinc-700">{r.objectName}</span>
                <span className={`text-xs font-semibold uppercase ${
                  r.urgency === "critical" ? "text-red-600" : r.urgency === "warning" ? "text-amber-600" : "text-zinc-400"
                }`}>
                  {r.urgency}
                </span>
              </div>
              <div className="flex justify-between mt-0.5">
                <span className="text-xs text-zinc-500">
                  {r.currentEnergy}/{r.maxEnergy} energy
                </span>
                <span className="text-xs text-zinc-500">
                  ~{r.estimatedCost} EVAP
                </span>
              </div>
            </div>
          ))}
        </div>
      );

    default:
      return null;
  }
}

function MiniStat({ label, value }: { label: string; value?: string }) {
  return (
    <div>
      <p className="text-xs text-zinc-400">{label}</p>
      <p className="text-xs font-semibold text-zinc-700">{value ?? "--"}</p>
    </div>
  );
}
