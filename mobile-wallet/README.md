# EvaporChain Mobile Wallet

Native iOS / Android wallet for EvaporChain. Built on Expo + React Native. Provides the same account model as the browser extension (post-quantum ML-DSA signing, energy-decay-aware account list) plus camera-based QR signing and biometric unlock.

## Status

Pre-1.0 (`v0.1.0`). Tier-2 surface coverage: onboarding, send/receive (QR), staking. Not yet shipped to App Store / Play Store.

## Stack

- **Expo SDK 52** + **React Native 0.76**
- **React Navigation v7** (native stack + bottom tabs)
- Post-quantum crypto: **`@noble/post-quantum`** (ML-DSA), **`@noble/hashes`**, **`@noble/ciphers`**, **`bip39`**
- **`expo-secure-store`** for keystore (iOS Keychain / Android Keystore)
- **`expo-local-authentication`** for biometric unlock
- **`expo-camera`** + **`react-native-qrcode-svg`** for QR send / receive
- **`expo-notifications`** for evaporation-warning pushes
- **Maestro** for E2E

## Develop

```bash
cd mobile-wallet
npm install

npm run start            # Expo Metro bundler
npm run ios              # iOS simulator
npm run android          # Android emulator
npm run web              # web preview (limited)

npm run ts:check
```

## E2E

```bash
npm run test:e2e         # runs all .maestro/*.yaml flows
npm run test:e2e:01      # single flow: create wallet
```

See `.maestro/README.md` for the per-flow conventions.

## What's covered (Tier 2)

| Surface | Path |
|---|---|
| Onboarding (mnemonic create/import, passphrase) | `src/screens/onboarding/` |
| Account list with live decay state | `src/screens/wallet/` |
| Send / receive via QR | `src/screens/send/`, `src/screens/receive/` |
| Staking (delegate, undelegate, claim) | `src/screens/staking/` |
| Biometric unlock + secure-store keystore | `src/utils/keystore.ts` |
| RPC client | `src/utils/rpc.ts` |

## Related

- Browser extension: `../extension/`
- Shared dApp SDK: `../wallet-sdk/`
- Node + JSON-RPC: `../crates/evaporchain-node/`

## License

MIT
