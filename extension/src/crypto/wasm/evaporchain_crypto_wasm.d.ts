/* tslint:disable */
/* eslint-disable */

/**
 * Derive an EvaporChain address from a public key.
 *
 * address = "0x" + hex(SHA-256(publicKey))
 */
export function deriveAddress(public_key: Uint8Array): string;

/**
 * Generate a new ML-DSA keypair.
 *
 * Returns a JS object `{ publicKey: Uint8Array, secretKey: Uint8Array }`.
 */
export function mlDsaKeygen(): any;

/**
 * Sign a message with an ML-DSA secret key.
 *
 * Accepts the full secret key bytes and message.
 * Returns the raw signature bytes (3293 bytes for Dilithium3).
 */
export function mlDsaSign(secret_key: Uint8Array, message: Uint8Array): Uint8Array;

/**
 * Verify an ML-DSA signature.
 *
 * Returns `true` if the signature is valid for the given message and public key.
 */
export function mlDsaVerify(message: Uint8Array, signature: Uint8Array, public_key: Uint8Array): boolean;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly deriveAddress: (a: number, b: number, c: number) => void;
    readonly mlDsaKeygen: (a: number) => void;
    readonly mlDsaSign: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly mlDsaVerify: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly __wbindgen_export: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export2: (a: number, b: number) => number;
    readonly __wbindgen_export3: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
