export type MessageStatus = "active" | "grace" | "ghost";

export interface MortalMessage {
  id: string;
  sender: string;
  recipient: string;
  content: string;
  energy: number;
  max_energy: number;
  half_life: number;
  created_at: string;
  status: MessageStatus;
  energy_percent: number;
}

export interface MessageStats {
  sent: number;
  received: number;
  alive: number;
  evaporated: number;
  total_energy_spent: number;
}

export interface SendMessagePayload {
  to: string;
  content: string;
  energy: number;
  half_life: number;
}

export interface BoostPayload {
  message_id: string;
  energy: number;
}

export interface HalfLifePreset {
  label: string;
  epochs: number;
  description: string;
}

export const HALF_LIFE_PRESETS: HalfLifePreset[] = [
  { label: "1 Hour", epochs: 60, description: "~1 hour" },
  { label: "1 Day", epochs: 1440, description: "~24 hours" },
  { label: "1 Week", epochs: 10080, description: "~7 days" },
  { label: "1 Month", epochs: 43200, description: "~30 days" },
];

declare global {
  interface Window {
    evaporchain?: {
      isConnected: () => Promise<boolean>;
      connect: () => Promise<{ address: string }>;
      disconnect: () => Promise<void>;
      getAddress: () => Promise<string>;
      signTransaction: (tx: unknown) => Promise<unknown>;
    };
  }
}
