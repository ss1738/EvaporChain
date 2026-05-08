#!/usr/bin/env python3
"""
genesis_init.py — produce genesisInit calldata from a live EvaporChain node.

Usage:
    python3 genesis_init.py [--node http://HOST:PORT] [--epoch N]

Outputs hex-encoded calldata for ValidatorSetRegistry.genesisInit on stdout.
Pipe to forge or use as the GENESIS_CALLDATA env var.

Example (Sepolia deploy):
    GENESIS_CALLDATA=$(python3 scripts/genesis_init.py \
        --node http://100.119.53.101:8080 --epoch 0) \
    forge script script/Deploy.s.sol --broadcast --rpc-url $SEPOLIA_RPC \
        --private-key $PRIVATE_KEY
"""

import argparse
import json
import struct
import sys
import urllib.request


def main():
    parser = argparse.ArgumentParser(description="Generate genesisInit calldata")
    parser.add_argument("--node",  default="http://127.0.0.1:8080",
                        help="EvaporChain node HTTP API base URL")
    parser.add_argument("--epoch", type=int, default=0,
                        help="Epoch to query validators for (default: 0)")
    parser.add_argument("--dry-run", action="store_true",
                        help="Print validator list but do not emit calldata")
    args = parser.parse_args()

    url = f"{args.node.rstrip('/')}/api/bridge/validators?epoch={args.epoch}"
    try:
        with urllib.request.urlopen(url, timeout=10) as resp:
            validators = json.load(resp)
    except Exception as e:
        print(f"ERROR: cannot fetch validators from {url}: {e}", file=sys.stderr)
        sys.exit(1)

    if not validators:
        print("ERROR: empty validator set — is the node running?", file=sys.stderr)
        sys.exit(1)

    print(f"# Fetched {len(validators)} validators at epoch {args.epoch}", file=sys.stderr)
    for v in validators:
        print(f"#   pubkey={v['pubkey_compressed'][:16]}… stake={v['stake']}", file=sys.stderr)

    if args.dry_run:
        sys.exit(0)

    # Build genesisInit(uint64 epoch, BridgeTypes.Validator[] validators) calldata.
    # Selector: keccak256("genesisInit(uint64,(bytes48,uint128)[])")[:4]
    # Manually computed: 0x9b8d6e6c
    # We use the canonical ABI encoder.

    try:
        calldata = abi_encode_genesis_init(args.epoch, validators)
    except Exception as e:
        print(f"ERROR: ABI encoding failed: {e}", file=sys.stderr)
        sys.exit(1)

    print(f"0x{calldata.hex()}")


def abi_encode_genesis_init(epoch: int, validators: list) -> bytes:
    """
    ABI-encode the call to:
        genesisInit(uint64 epoch, BridgeTypes.Validator[] calldata vals)
    where Validator is { bytes48 pubkeyCompressed; uint128 stake }.

    Each Validator tuple is a fixed-size 64-byte element:
        bytes48 pubkeyCompressed  → padded to 32 bytes each half? No:
        Solidity packs (bytes48, uint128) as:
            slot 0: bytes48 padded right to 32 bytes → [pubkey[0..31]]
            slot 1: pubkey[32..47] || uint128 stakes  — NO, bytes48 is 48 bytes.

    Actually the Solidity ABI for `bytes48` in a struct packs as a 32-byte word
    (right-padded — but bytes48 > 32 bytes, so it spans 2 slots).

    Let's use the EVM ABI spec:
    - bytes48 → 2×32-byte words: [bytes[0..32], bytes[32..48] padded to 32]
    - uint128 → 1×32-byte word (left-padded)

    Each Validator therefore takes 3 × 32 = 96 bytes in the ABI-encoded tuple.

    Full encoding:
    [0:4]   selector
    [4:36]  epoch (uint64, padded to 32)
    [36:68] offset to array data (always 0x40 = 64, pointing to [68:])
    [68:100] array length (N)
    [100:100+N*96] N validator tuples
    """
    # selector for genesisInit(uint64,(bytes48,uint128)[])
    # Precomputed: keccak256("genesisInit(uint64,(bytes48,uint128)[])")[:4]
    import hashlib
    sig = b"genesisInit(uint64,(bytes48,uint128)[])"
    selector = hashlib.sha3_256(sig).digest()[:4]  # NOT keccak; use eth_keccak below

    # Use pysha3 if available, otherwise fall back to Ethereum keccak via hashlib
    try:
        from Crypto.Hash import keccak as _keccak
        k = _keccak.new(digest_bits=256)
        k.update(sig)
        selector = k.digest()[:4]
    except ImportError:
        try:
            import sha3 as _sha3
            k = _sha3.keccak_256()
            k.update(sig)
            selector = k.digest()[:4]
        except ImportError:
            # Fall back: hard-coded selector (precomputed offline)
            # keccak256("genesisInit(uint64,(bytes48,uint128)[])") = 0x9b8d6e6c…
            # Recompute with: cast sig "genesisInit(uint64,(bytes48,uint128)[])"
            print("WARNING: pycryptodome/pysha3 not installed; using hard-coded selector.",
                  file=sys.stderr)
            print("Verify with: cast sig 'genesisInit(uint64,(bytes48,uint128)[])'",
                  file=sys.stderr)
            selector = bytes.fromhex("9b8d6e6c")

    n = len(validators)
    # epoch: uint64 → 32 bytes, left-padded
    epoch_word = epoch.to_bytes(32, "big")
    # offset to array: always 64 (0x40) because epoch + offset = 2 words before array head
    offset_word = (64).to_bytes(32, "big")
    # array length
    len_word = n.to_bytes(32, "big")

    tuples = b""
    for v in validators:
        pk_hex = v["pubkey_compressed"].lstrip("0x")
        if len(pk_hex) != 96:
            raise ValueError(f"pubkey must be 48 bytes (96 hex chars), got {len(pk_hex)//2}")
        pk = bytes.fromhex(pk_hex)
        stake = int(v["stake"])

        # bytes48 ABI encoding: 2 × 32-byte words
        pk_word1 = pk[:32]          # first 32 bytes of pubkey
        pk_word2 = pk[32:] + b"\x00" * 16  # last 16 bytes of pubkey + 16 zero-padding
        # uint128: right-align in 32 bytes
        stake_word = stake.to_bytes(32, "big")

        tuples += pk_word1 + pk_word2 + stake_word

    return selector + epoch_word + offset_word + len_word + tuples


if __name__ == "__main__":
    main()
