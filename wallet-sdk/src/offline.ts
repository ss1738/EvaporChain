/**
 * Offline signing support for EvaporChain.
 *
 * Enables cold-wallet and hardware-wallet flows:
 *   1. `prepare()` — fetch canonical bytes to sign from the node
 *   2. User signs the bytes externally with their ML-DSA private key
 *   3. `broadcast()` — attach signature and submit the transaction
 *
 * The transaction type + params remain unchanged from the normal API —
 * the only difference is that `signature` and `publicKey` are supplied
 * by the caller rather than the browser extension.
 */

import type {
  OfflineTxType,
  OfflineTransferParams,
  OfflineCreateObjectParams,
  OfflineRefreshParams,
  OfflineDeployContractParams,
  OfflineCallContractParams,
  SignableTransaction,
  SignedTransaction,
  TxResult,
} from "./types";

type OfflineParams =
  | OfflineTransferParams
  | OfflineCreateObjectParams
  | OfflineRefreshParams
  | OfflineDeployContractParams
  | OfflineCallContractParams;

/** Normalise SDK camelCase params to the snake_case keys the API expects. */
function toApiParams(
  txType: OfflineTxType,
  params: OfflineParams,
): Record<string, unknown> {
  switch (txType) {
    case "transfer": {
      const p = params as OfflineTransferParams;
      return { from: p.from, to: p.to, amount: p.amount };
    }
    case "create_object": {
      const p = params as OfflineCreateObjectParams;
      return {
        creator: p.creator,
        object_id: p.objectId,
        energy: p.energy,
        half_life: p.halfLife,
        ...(p.data !== undefined ? { data: p.data } : {}),
      };
    }
    case "refresh": {
      const p = params as OfflineRefreshParams;
      return { object_id: p.objectId, energy_deposit: p.energyDeposit };
    }
    case "deploy_contract": {
      const p = params as OfflineDeployContractParams;
      return {
        deployer: p.deployer,
        template: p.template,
        init_args: p.initArgs,
        energy: p.energy,
        half_life: p.halfLife,
        ...(p.rules !== undefined ? { rules: p.rules } : {}),
      };
    }
    case "call_contract": {
      const p = params as OfflineCallContractParams;
      return {
        caller: p.caller,
        contract_id: p.contractId,
        method: p.method,
        args: p.args,
        epoch: p.epoch,
      };
    }
  }
}

/** Build the broadcast request body from already-normalised API params. */
function toBroadcastBody(
  params: Record<string, unknown>,
  signatureHex: string,
  publicKeyHex: string,
): Record<string, unknown> {
  return { ...params, signature: signatureHex, public_key: publicKeyHex };
}

/** Endpoint path for each tx type. */
function broadcastPath(txType: OfflineTxType): string {
  switch (txType) {
    case "transfer":
      return "/api/tx/transfer";
    case "create_object":
      return "/api/tx/create-object";
    case "refresh":
      return "/api/tx/refresh";
    case "deploy_contract":
      return "/api/tx/deploy-contract";
    case "call_contract":
      return "/api/tx/call-contract";
  }
}

export class OfflineSigner {
  private _rpcUrl: string;

  constructor(rpcUrl: string) {
    this._rpcUrl = rpcUrl.replace(/\/$/, "");
  }

  /**
   * Fetch the hex-encoded bytes that must be signed for the given transaction.
   * The returned `signableHex` is what you pass to `mlDsaSign()` in the
   * evaporchain-crypto-wasm module (or any compatible ML-DSA3 implementation).
   */
  async prepare(
    txType: OfflineTxType,
    params: OfflineParams,
  ): Promise<SignableTransaction> {
    const apiParams = toApiParams(txType, params);
    const resp = await fetch(`${this._rpcUrl}/api/tx/signable`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ tx_type: txType, params: apiParams }),
    });
    if (!resp.ok) {
      throw new Error(`prepare failed: ${resp.status} ${resp.statusText}`);
    }
    const json = await resp.json();
    if (!json.ok) {
      throw new Error(`prepare failed: ${json.error ?? "unknown error"}`);
    }
    return {
      txType,
      signableHex: json.signable_hex as string,
      chainId: json.chain_id as string,
      params: apiParams,
    };
  }

  /**
   * Attach an externally-produced ML-DSA signature to a prepared transaction.
   * `signatureHex` and `publicKeyHex` come from signing `signableHex` with
   * your private key (e.g. via `mlDsaSign` from evaporchain-crypto-wasm).
   */
  attach(
    prepared: SignableTransaction,
    signatureHex: string,
    publicKeyHex: string,
  ): SignedTransaction {
    return {
      txType: prepared.txType,
      params: prepared.params,
      signatureHex,
      publicKeyHex,
    };
  }

  /**
   * Broadcast a pre-signed transaction to the node.
   * The node will verify the ML-DSA signature before executing.
   */
  async broadcast(
    signed: SignedTransaction,
  ): Promise<TxResult> {
    const body = toBroadcastBody(
      signed.params,
      signed.signatureHex,
      signed.publicKeyHex,
    );
    const path = broadcastPath(signed.txType);
    const resp = await fetch(`${this._rpcUrl}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!resp.ok) {
      throw new Error(`broadcast failed: ${resp.status} ${resp.statusText}`);
    }
    const json = await resp.json();
    return {
      success: json.success as boolean,
      message: json.message as string,
      txHash: json.tx_hash as string | undefined,
    };
  }

  /**
   * Convenience: fetch nonce for an address.
   * Useful when constructing transactions manually before calling `prepare()`.
   */
  async getNonce(address: string): Promise<number> {
    const resp = await fetch(
      `${this._rpcUrl}/api/tx/nonce/${encodeURIComponent(address)}`,
    );
    if (!resp.ok) {
      throw new Error(`getNonce failed: ${resp.status}`);
    }
    const json = await resp.json();
    return (json.nonce as number) ?? 0;
  }
}
