/**
 * AI command parser for the EvaporChain wallet extension.
 * Uses pattern-matching (regex-based, no LLM API) to parse natural language
 * commands into structured intents for the wallet.
 */

// ── Types ──

export type AiIntent =
  | "send_transfer"
  | "refresh_object"
  | "check_balance"
  | "list_objects"
  | "list_nfts"
  | "decay_forecast"
  | "refresh_strategy"
  | "bridge"
  | "chain_status"
  | "help"
  | "unknown";

export interface ParsedCommand {
  intent: AiIntent;
  params: Record<string, string | number>;
  confidence: number;
}

export interface AiResponse {
  message: string;
  action?: AiAction;
}

export type AiAction =
  | { type: "show_balance"; balance: number }
  | { type: "show_objects"; objects: any[] }
  | { type: "show_nfts"; nfts: any[] }
  | { type: "show_chain_status"; status: any }
  | { type: "preview_transfer"; to: string; amount: number }
  | { type: "preview_refresh"; objectName: string; objectId: string; energy: number }
  | { type: "preview_bridge"; amount: number; chain: string }
  | { type: "decay_forecast"; objects: any[] }
  | { type: "refresh_strategy"; recommendations: RefreshRecommendation[] }
  | { type: "none" };

export interface RefreshRecommendation {
  objectId: string;
  objectName: string;
  currentEnergy: number;
  maxEnergy: number;
  urgency: "critical" | "warning" | "safe";
  suggestedEnergy: number;
  estimatedCost: number;
}

export interface WalletApi {
  getBalance: () => number;
  getAddress: () => string;
  getObjects: () => any[];
  getNfts: () => any[];
  getChainStatus: () => any;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  text: string;
  action?: AiAction;
  timestamp: number;
}

// ── Pattern definitions ──

interface PatternRule {
  intent: AiIntent;
  patterns: RegExp[];
  extract?: (match: RegExpMatchArray, input: string) => Record<string, string | number>;
}

const PATTERN_RULES: PatternRule[] = [
  {
    intent: "send_transfer",
    patterns: [
      /(?:send|transfer|pay)\s+(\d+(?:\.\d+)?)\s*(?:evap|tokens?)?\s+to\s+(0x[a-fA-F0-9]+)/i,
      /(?:send|transfer|pay)\s+(0x[a-fA-F0-9]+)\s+(\d+(?:\.\d+)?)\s*(?:evap|tokens?)?/i,
      /(?:send|transfer|pay)\s+(\d+(?:\.\d+)?)\s*(?:evap|tokens?)/i,
    ],
    extract: (match, input) => {
      // Try full pattern first: "send X to ADDRESS"
      const full = input.match(/(?:send|transfer|pay)\s+(\d+(?:\.\d+)?)\s*(?:evap|tokens?)?\s+to\s+(0x[a-fA-F0-9]+)/i);
      if (full) {
        return { amount: parseFloat(full[1]), to: full[2] };
      }
      // Try reverse: "send ADDRESS X"
      const rev = input.match(/(?:send|transfer|pay)\s+(0x[a-fA-F0-9]+)\s+(\d+(?:\.\d+)?)/i);
      if (rev) {
        return { to: rev[1], amount: parseFloat(rev[2]) };
      }
      // Amount only
      const amountOnly = input.match(/(\d+(?:\.\d+)?)/);
      if (amountOnly) {
        return { amount: parseFloat(amountOnly[1]), to: "" };
      }
      return { amount: 0, to: "" };
    },
  },
  {
    intent: "refresh_object",
    patterns: [
      /(?:refresh|recharge|refill|top.?up|energi[sz]e)\s+(?:my\s+)?(?:nft\s+)?(.+)/i,
    ],
    extract: (match) => {
      const name = match[1]?.trim().replace(/\s*(?:nft|object|token)$/i, "").trim();
      return { objectName: name || "" };
    },
  },
  {
    intent: "check_balance",
    patterns: [
      /(?:what(?:'s| is)\s+my\s+)?balance/i,
      /how\s+much\s+(?:evap|money|funds?|tokens?)\s+(?:do\s+)?i\s+have/i,
      /(?:show|check|get)\s+(?:my\s+)?balance/i,
      /my\s+balance/i,
    ],
  },
  {
    intent: "list_objects",
    patterns: [
      /(?:show|list|get|display|what(?:'s| is| are))\s+(?:my\s+)?(?:state\s+)?objects/i,
      /(?:show|list|get|display)\s+(?:my\s+)?(?:assets|inventory|items)/i,
      /(?:what|which)\s+objects?\s+(?:do\s+)?i\s+(?:have|own)/i,
      /how\s+much\s+energy\s+(?:do\s+)?my\s+objects?\s+have/i,
    ],
  },
  {
    intent: "list_nfts",
    patterns: [
      /(?:show|list|get|display|what(?:'s| is| are))\s+(?:my\s+)?nfts?/i,
      /(?:show|list|get|display)\s+(?:my\s+)?collectibles?/i,
      /(?:what|which)\s+nfts?\s+(?:do\s+)?i\s+(?:have|own)/i,
    ],
  },
  {
    intent: "decay_forecast",
    patterns: [
      /when\s+will\s+(?:my\s+)?(.+?)\s+(?:evaporate|die|expire|decay|disappear)/i,
      /(?:decay|evaporation)\s+(?:forecast|prediction|estimate)/i,
      /how\s+long\s+(?:will|until|before)\s+(?:my\s+)?(.+?)\s+(?:last|survive|evaporate)/i,
    ],
    extract: (match) => {
      const name = match[1]?.trim() || "";
      return { objectName: name };
    },
  },
  {
    intent: "refresh_strategy",
    patterns: [
      /(?:cheapest|optimal|best|most efficient)\s+(?:way\s+)?(?:to\s+)?(?:refresh|keep|maintain|save)/i,
      /refresh\s+strateg/i,
      /(?:keep|save)\s+(?:all\s+)?(?:my\s+)?objects?\s+alive/i,
      /(?:how\s+(?:can|do|should)\s+i\s+)?(?:efficiently|cheaply)\s+refresh/i,
    ],
  },
  {
    intent: "bridge",
    patterns: [
      /bridge\s+(\d+(?:\.\d+)?)\s*(?:evap|tokens?)?\s+to\s+(\w+)/i,
      /(?:send|transfer|move)\s+(\d+(?:\.\d+)?)\s*(?:evap|tokens?)?\s+(?:to|on|via)\s+(?:the\s+)?(\w+)\s+(?:chain|network|bridge)/i,
    ],
    extract: (match) => {
      return { amount: parseFloat(match[1] || "0"), chain: match[2] || "" };
    },
  },
  {
    intent: "chain_status",
    patterns: [
      /(?:what(?:'s| is)\s+(?:the\s+)?)?(?:current\s+)?block\s*(?:height|number)/i,
      /(?:chain|network)\s+status/i,
      /(?:show|get|check)\s+(?:the\s+)?(?:chain|network)\s+(?:status|info|state)/i,
      /(?:how\s+is|what's)\s+(?:the\s+)?(?:chain|network)\s+(?:doing|looking)/i,
    ],
  },
  {
    intent: "help",
    patterns: [
      /^(?:help|what\s+can\s+you\s+do|commands?|how\s+(?:do|does)\s+(?:this|it)\s+work)/i,
      /^(?:hi|hello|hey|yo)$/i,
    ],
  },
];

// ── AI Engine ──

export class AiEngine {
  /** Parse a natural language command into a structured intent */
  parseCommand(input: string): ParsedCommand {
    const trimmed = input.trim();
    if (!trimmed) {
      return { intent: "unknown", params: {}, confidence: 0 };
    }

    for (const rule of PATTERN_RULES) {
      for (const pattern of rule.patterns) {
        const match = trimmed.match(pattern);
        if (match) {
          const params = rule.extract ? rule.extract(match, trimmed) : {};
          return {
            intent: rule.intent,
            params,
            confidence: 0.9,
          };
        }
      }
    }

    // Fuzzy fallback: check for keywords
    const lower = trimmed.toLowerCase();
    const fuzzyMap: Array<[string[], AiIntent]> = [
      [["send", "transfer", "pay"], "send_transfer"],
      [["refresh", "recharge", "energy", "top up"], "refresh_object"],
      [["balance", "funds", "how much"], "check_balance"],
      [["objects", "assets", "inventory"], "list_objects"],
      [["nft", "collectible", "collection"], "list_nfts"],
      [["evaporate", "decay", "die", "expire"], "decay_forecast"],
      [["strategy", "cheapest", "optimize", "efficient"], "refresh_strategy"],
      [["bridge", "cross-chain"], "bridge"],
      [["block", "chain", "status", "height", "network"], "chain_status"],
      [["help", "commands"], "help"],
    ];

    for (const [keywords, intent] of fuzzyMap) {
      if (keywords.some(k => lower.includes(k))) {
        return { intent, params: {}, confidence: 0.5 };
      }
    }

    return { intent: "unknown", params: {}, confidence: 0 };
  }

  /** Execute a parsed intent against the wallet API and return a response */
  executeIntent(intent: AiIntent, params: Record<string, string | number>, walletApi: WalletApi): AiResponse {
    switch (intent) {
      case "check_balance": {
        const balance = walletApi.getBalance();
        return {
          message: `Your current balance is **${balance.toLocaleString()} EVAP**.`,
          action: { type: "show_balance", balance },
        };
      }

      case "list_objects": {
        const objects = walletApi.getObjects();
        if (objects.length === 0) {
          return { message: "You don't have any state objects yet.", action: { type: "show_objects", objects: [] } };
        }
        const summary = objects.map((o: any) =>
          `- **${o.name}**: ${o.current_energy}/${o.max_energy} energy (${o.state})`
        ).join("\n");
        return {
          message: `You have **${objects.length}** objects:\n\n${summary}`,
          action: { type: "show_objects", objects },
        };
      }

      case "list_nfts": {
        const nfts = walletApi.getNfts();
        if (nfts.length === 0) {
          return { message: "You don't own any NFTs yet.", action: { type: "show_nfts", nfts: [] } };
        }
        const summary = nfts.map((n: any) =>
          `- **${n.name}** (${n.collection}): ${n.current_energy}/${n.max_energy} energy`
        ).join("\n");
        return {
          message: `You own **${nfts.length}** NFTs:\n\n${summary}`,
          action: { type: "show_nfts", nfts },
        };
      }

      case "chain_status": {
        const status = walletApi.getChainStatus();
        if (!status) {
          return { message: "Unable to fetch chain status. The node may be unreachable.", action: { type: "none" } };
        }
        return {
          message: `**Chain Status:**\n- Block Height: ${status.block_height?.toLocaleString()}\n- Active Objects: ${status.active_objects?.toLocaleString()}\n- Ghosts: ${status.ghost_count?.toLocaleString()}\n- Peers: ${status.peer_count}\n- Epoch: ${status.epoch}`,
          action: { type: "show_chain_status", status },
        };
      }

      case "send_transfer": {
        const amount = Number(params.amount) || 0;
        const to = String(params.to || "");
        if (!amount) {
          return { message: "How much EVAP would you like to send? Please specify an amount.", action: { type: "none" } };
        }
        if (!to) {
          return { message: `Got it, you want to send **${amount} EVAP**. What's the recipient address?`, action: { type: "none" } };
        }
        const balance = walletApi.getBalance();
        if (amount > balance) {
          return { message: `Insufficient balance. You have **${balance} EVAP** but want to send **${amount} EVAP**.`, action: { type: "none" } };
        }
        return {
          message: `Ready to send **${amount} EVAP** to \`${to.slice(0, 8)}...${to.slice(-4)}\`. Please confirm the transaction below.`,
          action: { type: "preview_transfer", to, amount },
        };
      }

      case "refresh_object": {
        const objectName = String(params.objectName || "");
        if (!objectName) {
          return { message: "Which object would you like to refresh? Please provide the name.", action: { type: "none" } };
        }
        const objects = walletApi.getObjects();
        const target = objects.find((o: any) =>
          o.name.toLowerCase().includes(objectName.toLowerCase()) ||
          o.id.toLowerCase().includes(objectName.toLowerCase())
        );
        if (!target) {
          return { message: `I couldn't find an object matching "${objectName}". Try checking your objects list.`, action: { type: "none" } };
        }
        const energyNeeded = target.max_energy - target.current_energy;
        return {
          message: `Found **${target.name}** (${target.current_energy}/${target.max_energy} energy, ${target.state}). Suggest refreshing with **${energyNeeded} energy**. Confirm below.`,
          action: { type: "preview_refresh", objectName: target.name, objectId: target.id, energy: energyNeeded },
        };
      }

      case "decay_forecast": {
        const objectName = String(params.objectName || "");
        const objects = walletApi.getObjects();
        if (objectName) {
          const target = objects.find((o: any) =>
            o.name.toLowerCase().includes(objectName.toLowerCase())
          );
          if (!target) {
            return { message: `I couldn't find an object matching "${objectName}".`, action: { type: "none" } };
          }
          const epochsLeft = Math.max(0, Math.floor(target.current_energy / (target.max_energy / target.half_life)));
          return {
            message: `**${target.name}** has ${target.current_energy}/${target.max_energy} energy.\nEstimated ~${epochsLeft} epochs until evaporation.\nCurrent state: **${target.state}**`,
            action: { type: "decay_forecast", objects: [target] },
          };
        }
        // Show all objects' decay
        if (objects.length === 0) {
          return { message: "You don't have any objects to forecast.", action: { type: "none" } };
        }
        const forecasts = objects.map((o: any) => {
          const epochsLeft = Math.max(0, Math.floor(o.current_energy / (o.max_energy / o.half_life)));
          return `- **${o.name}**: ~${epochsLeft} epochs remaining (${o.state})`;
        }).join("\n");
        return {
          message: `**Decay Forecast:**\n\n${forecasts}`,
          action: { type: "decay_forecast", objects },
        };
      }

      case "refresh_strategy": {
        const objects = walletApi.getObjects();
        if (objects.length === 0) {
          return { message: "You don't have any objects to optimize.", action: { type: "none" } };
        }
        const recommendations: RefreshRecommendation[] = objects
          .map((o: any) => {
            const pct = (o.current_energy / o.max_energy) * 100;
            let urgency: "critical" | "warning" | "safe" = "safe";
            if (pct < 20 || o.state === "Grace") urgency = "critical";
            else if (pct < 50) urgency = "warning";
            const suggestedEnergy = o.max_energy - o.current_energy;
            return {
              objectId: o.id,
              objectName: o.name,
              currentEnergy: o.current_energy,
              maxEnergy: o.max_energy,
              urgency,
              suggestedEnergy,
              estimatedCost: suggestedEnergy,
            };
          })
          .sort((a: RefreshRecommendation, b: RefreshRecommendation) => {
            const order = { critical: 0, warning: 1, safe: 2 };
            return order[a.urgency] - order[b.urgency];
          });

        const totalCost = recommendations.filter(r => r.urgency !== "safe").reduce((sum, r) => sum + r.estimatedCost, 0);
        const critical = recommendations.filter(r => r.urgency === "critical");
        const warnings = recommendations.filter(r => r.urgency === "warning");

        let msg = "**Refresh Strategy:**\n\n";
        if (critical.length > 0) {
          msg += `**Critical (${critical.length}):**\n`;
          msg += critical.map(r => `- ${r.objectName}: needs ${r.suggestedEnergy} energy (~${r.estimatedCost} EVAP)`).join("\n");
          msg += "\n\n";
        }
        if (warnings.length > 0) {
          msg += `**Warning (${warnings.length}):**\n`;
          msg += warnings.map(r => `- ${r.objectName}: needs ${r.suggestedEnergy} energy (~${r.estimatedCost} EVAP)`).join("\n");
          msg += "\n\n";
        }
        msg += `**Total estimated cost for urgent refreshes: ${totalCost} EVAP**`;

        return {
          message: msg,
          action: { type: "refresh_strategy", recommendations },
        };
      }

      case "bridge": {
        const amount = Number(params.amount) || 0;
        const chain = String(params.chain || "");
        if (!amount || !chain) {
          return { message: "Please specify an amount and destination chain. Example: \"Bridge 50 EVAP to Ethereum\"", action: { type: "none" } };
        }
        return {
          message: `Bridge **${amount} EVAP** to **${chain}**. Please confirm the transaction below.\n\n*Note: Bridge transactions may take 10-30 minutes to finalize.*`,
          action: { type: "preview_bridge", amount, chain },
        };
      }

      case "help": {
        return {
          message: `I can help you with:\n\n- **Check balance** — "What's my balance?"\n- **Send EVAP** — "Send 100 EVAP to 0x..."\n- **View objects** — "Show my objects"\n- **View NFTs** — "Show my NFTs"\n- **Refresh objects** — "Refresh my Genesis #001"\n- **Decay forecast** — "When will my NFT evaporate?"\n- **Refresh strategy** — "Cheapest way to keep all objects alive"\n- **Bridge** — "Bridge 50 EVAP to Ethereum"\n- **Chain status** — "What's the current block height?"`,
          action: { type: "none" },
        };
      }

      case "unknown":
      default:
        return {
          message: "I'm not sure what you mean. Try asking about your balance, objects, or type \"help\" to see what I can do.",
          action: { type: "none" },
        };
    }
  }

  /** Get contextual suggested prompts based on wallet state */
  getSuggestions(walletApi: WalletApi): string[] {
    const suggestions: string[] = [];
    const objects = walletApi.getObjects();
    const balance = walletApi.getBalance();

    suggestions.push("Check my balance");

    if (objects.length > 0) {
      suggestions.push("Show my objects");

      const atRisk = objects.filter((o: any) => {
        const pct = (o.current_energy / o.max_energy) * 100;
        return pct < 30 || o.state === "Grace";
      });
      if (atRisk.length > 0) {
        suggestions.push("Refresh urgent objects");
      }
    }

    if (balance > 0) {
      suggestions.push("Send EVAP");
    }

    suggestions.push("Show my NFTs");
    suggestions.push("Chain status");

    return suggestions.slice(0, 4);
  }
}

/** Singleton instance */
export const aiEngine = new AiEngine();

/** Generate a unique message ID */
export function createMessageId(): string {
  return `msg_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
}
