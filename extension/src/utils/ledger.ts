/**
 * Ledger hardware wallet transport wrapper for EvaporChain.
 * Uses WebHID API for communication with Ledger devices.
 *
 * NOTE: This is a stub implementation — the custom EvaporChain Ledger app
 * does not exist yet. The APDU command structure and transport layer are
 * scaffolded so that integration is straightforward once the app is written.
 */

// WebHID type declarations (not in default TS lib)
declare global {
  interface Navigator {
    hid: {
      requestDevice(options: { filters: Array<{ vendorId: number }> }): Promise<HIDDevice[]>;
    };
  }
  interface HIDDevice {
    open(): Promise<void>;
    close(): Promise<void>;
    sendReport(reportId: number, data: BufferSource): Promise<void>;
    addEventListener(type: string, listener: (event: { data: DataView }) => void): void;
    removeEventListener(type: string, listener: (event: { data: DataView }) => void): void;
    productName: string;
    vendorId: number;
    productId: number;
  }
}

// ── ML-DSA APDU Commands for custom EvaporChain Ledger app ──
const CLA = 0xe0; // Application class byte
const INS_GET_VERSION = 0x01;
const INS_GET_PUBLIC_KEY = 0x02;
const INS_SIGN_TRANSACTION = 0x04;
const INS_SIGN_MESSAGE = 0x06;
const INS_GET_ADDRESS = 0x08;

// Derivation path for EvaporChain: m/44'/evap'/0'/0/N
// evap' coin type placeholder = 0x80004556 (EVAP in hex-ish)
const EVAP_COIN_TYPE = 0x4556;
const BASE_PATH = `m/44'/${EVAP_COIN_TYPE}'/0'/0`;

export interface LedgerDeviceInfo {
  model: string;
  firmware: string;
}

export interface LedgerAccount {
  path: string;
  address: string;
  publicKey: Uint8Array;
}

export type LedgerConnectionStatus = "disconnected" | "searching" | "connected" | "error";

export interface LedgerSignResult {
  signature: Uint8Array;
  success: boolean;
}

/**
 * Encodes a BIP-44 derivation path into bytes for APDU commands.
 */
function encodeDerivationPath(path: string): Uint8Array {
  const parts = path
    .replace("m/", "")
    .split("/")
    .map((p) => {
      const hardened = p.endsWith("'");
      const index = parseInt(p.replace("'", ""), 10);
      return hardened ? index + 0x80000000 : index;
    });

  const buf = new Uint8Array(1 + parts.length * 4);
  buf[0] = parts.length;
  for (let i = 0; i < parts.length; i++) {
    const offset = 1 + i * 4;
    buf[offset] = (parts[i] >> 24) & 0xff;
    buf[offset + 1] = (parts[i] >> 16) & 0xff;
    buf[offset + 2] = (parts[i] >> 8) & 0xff;
    buf[offset + 3] = parts[i] & 0xff;
  }
  return buf;
}

/**
 * Builds a raw APDU command buffer.
 */
function buildApdu(
  cla: number,
  ins: number,
  p1: number,
  p2: number,
  data?: Uint8Array
): Uint8Array {
  const len = data ? data.length : 0;
  const buf = new Uint8Array(5 + len);
  buf[0] = cla;
  buf[1] = ins;
  buf[2] = p1;
  buf[3] = p2;
  buf[4] = len;
  if (data) buf.set(data, 5);
  return buf;
}

export class LedgerManager {
  private device: HIDDevice | null = null;
  private _connected = false;
  private _status: LedgerConnectionStatus = "disconnected";

  get status(): LedgerConnectionStatus {
    return this._status;
  }

  /**
   * Initiate WebHID connection to a Ledger device.
   * Opens the browser HID device picker filtered to Ledger vendor IDs.
   */
  async connect(): Promise<boolean> {
    this._status = "searching";

    try {
      // Ledger vendor IDs: 0x2c97 (Ledger), 0x2581 (legacy)
      const devices = await navigator.hid.requestDevice({
        filters: [
          { vendorId: 0x2c97 },
          { vendorId: 0x2581 },
        ],
      });

      if (devices.length === 0) {
        this._status = "disconnected";
        return false;
      }

      this.device = devices[0];
      if (!this._connected) {
        await this.device.open();
      }

      this._connected = true;
      this._status = "connected";
      return true;
    } catch (err) {
      console.error("[LedgerManager] Connection failed:", err);
      this._status = "error";
      this._connected = false;
      return false;
    }
  }

  /**
   * Close the HID transport and release the device.
   */
  async disconnect(): Promise<void> {
    if (this.device && this._connected) {
      try {
        await this.device.close();
      } catch {
        // Device may already be closed
      }
    }
    this.device = null;
    this._connected = false;
    this._status = "disconnected";
  }

  /**
   * Check if a Ledger device is currently connected.
   */
  isConnected(): boolean {
    return this._connected && this.device !== null;
  }

  /**
   * Query the Ledger for EvaporChain app version and device model.
   * STUB: Returns placeholder data until the real Ledger app exists.
   */
  async getDeviceInfo(): Promise<LedgerDeviceInfo> {
    if (!this.isConnected()) {
      throw new Error("Ledger not connected");
    }

    // STUB — In production, send INS_GET_VERSION APDU and parse response
    const _apdu = buildApdu(CLA, INS_GET_VERSION, 0x00, 0x00);
    // const response = await this.sendApdu(apdu);

    return {
      model: this.device?.productName ?? "Ledger Nano S Plus",
      firmware: "1.0.0-stub",
    };
  }

  /**
   * Derive N accounts from the Ledger using the EvaporChain derivation path.
   * STUB: Returns placeholder addresses until the real Ledger app exists.
   */
  async getAccounts(count: number = 5): Promise<LedgerAccount[]> {
    if (!this.isConnected()) {
      throw new Error("Ledger not connected");
    }

    const accounts: LedgerAccount[] = [];
    for (let i = 0; i < count; i++) {
      const path = `${BASE_PATH}/${i}`;
      const pathBytes = encodeDerivationPath(path);

      // STUB — In production, send INS_GET_ADDRESS and parse the response
      const _apdu = buildApdu(CLA, INS_GET_ADDRESS, 0x00, 0x00, pathBytes);
      // const response = await this.sendApdu(apdu);

      // Generate deterministic placeholder address
      const stubAddr = `evap1hw${i.toString().padStart(4, "0")}${"0".repeat(34)}`;
      const stubPubKey = new Uint8Array(32).fill(i);

      accounts.push({
        path,
        address: stubAddr,
        publicKey: stubPubKey,
      });
    }

    return accounts;
  }

  /**
   * Request the public key at a specific derivation path.
   * STUB: Returns placeholder key until the real Ledger app exists.
   */
  async getPublicKey(path: string): Promise<Uint8Array> {
    if (!this.isConnected()) {
      throw new Error("Ledger not connected");
    }

    const pathBytes = encodeDerivationPath(path);
    const _apdu = buildApdu(CLA, INS_GET_PUBLIC_KEY, 0x00, 0x00, pathBytes);
    // const response = await this.sendApdu(apdu);

    // STUB — return 2528 bytes for ML-DSA-65 public key
    return new Uint8Array(2528).fill(0xab);
  }

  /**
   * Sign a transaction on the Ledger device.
   * The user must physically confirm on the hardware screen.
   * STUB: Returns placeholder signature until the real Ledger app exists.
   */
  async signTransaction(path: string, txBytes: Uint8Array): Promise<Uint8Array> {
    if (!this.isConnected()) {
      throw new Error("Ledger not connected");
    }

    const pathBytes = encodeDerivationPath(path);

    // In production: send path first, then tx data in chunks
    // P1=0x00 for first chunk with path, P1=0x80 for subsequent data chunks
    const _initApdu = buildApdu(CLA, INS_SIGN_TRANSACTION, 0x00, 0x00, pathBytes);
    // await this.sendApdu(initApdu);

    // Send transaction data (would be chunked for large txs)
    const _dataApdu = buildApdu(CLA, INS_SIGN_TRANSACTION, 0x80, 0x00, txBytes);
    // const response = await this.sendApdu(dataApdu);

    // STUB — return 3309 bytes for ML-DSA-65 signature
    // Simulate a small delay as if the user is confirming on device
    await new Promise((resolve) => setTimeout(resolve, 500));
    return new Uint8Array(3309).fill(0xcd);
  }

  /**
   * Sign an arbitrary message on the Ledger device.
   * STUB: Returns placeholder signature until the real Ledger app exists.
   */
  async signMessage(path: string, message: Uint8Array): Promise<Uint8Array> {
    if (!this.isConnected()) {
      throw new Error("Ledger not connected");
    }

    const pathBytes = encodeDerivationPath(path);
    const payload = new Uint8Array(pathBytes.length + message.length);
    payload.set(pathBytes, 0);
    payload.set(message, pathBytes.length);

    const _apdu = buildApdu(CLA, INS_SIGN_MESSAGE, 0x00, 0x00, payload);
    // const response = await this.sendApdu(apdu);

    // STUB — ML-DSA-65 signature
    await new Promise((resolve) => setTimeout(resolve, 500));
    return new Uint8Array(3309).fill(0xef);
  }

  /**
   * Low-level APDU send (placeholder for actual WebHID write/read).
   * Will be implemented when the custom Ledger app is available.
   */
  // private async sendApdu(apdu: Uint8Array): Promise<Uint8Array> {
  //   if (!this.device) throw new Error("No device");
  //   // WebHID report: reportId=0, data=apdu padded to 64 bytes
  //   const report = new Uint8Array(64);
  //   report.set(apdu, 0);
  //   await this.device.sendReport(0, report);
  //   // Read response
  //   // ... HID framing protocol would go here
  //   return new Uint8Array(0);
  // }
}

/** Singleton instance for use across the extension */
export const ledgerManager = new LedgerManager();
