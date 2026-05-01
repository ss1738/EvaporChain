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

// ── Substrate primitives ──

export interface ChainStatusResponse {
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
}

export interface DecayForecastResponse {
  objectId: string;
  currentEnergy: number;
  maxEnergy: number;
  halfLife: number;
  currentEpoch: number;
  epochDurationMs: number;
  projectedEnergy: Array<{ epoch: number; energy: number; percent: number }>;
  evaporationEpoch: number;
  evaporationDate: string;
}

export interface PatronagePledgeRequest {
  object_id_hex: string;
  namespace_id_hex: string;
  donation_per_epoch: number;
  epochs: number;
  current_epoch: number;
}

export interface PatronagePledgeResponse {
  status: string;
  object_id_hex: string;
  pre_funded: number;
  expires_epoch: number;
  detail: string;
}

export interface PatronageStatusResponse {
  active_covenants: number;
  total_pre_funded: number;
  total_active_score: number;
  patronage_ns_hex: string;
}

export interface DemurrageOwedRequest {
  balance: number;
  last_touched_epoch: number;
  current_epoch: number;
  lambda_base_ppm: number;
  threshold: number;
}

export interface DemurrageOwedResponse {
  status: string;
  balance: number;
  last_touched_epoch: number;
  current_epoch: number;
  elapsed_epochs: number;
  rate_ppm: number;
  owed: number;
  remaining_balance: number;
  is_disabled: boolean;
}

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
