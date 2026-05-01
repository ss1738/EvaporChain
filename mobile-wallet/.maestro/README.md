# Mobile-wallet Maestro harness

Three end-to-end flows mirroring the extension's Playwright specs (see
`extension/tests/e2e/specs/`) — adapted to RN's PIN-based create flow.

## Install Maestro

Maestro is a CLI, not an npm package. Install once per machine.

```sh
# macOS (preferred)
brew install maestro

# or — direct installer
curl -Ls https://get.maestro.mobile.dev | bash
```

## Boot an emulator/simulator

iOS:
```sh
xcrun simctl boot 'iPhone 15 Pro'
open -a Simulator
```

Android:
```sh
emulator -list-avds
emulator -avd <avd-name> &
```

## Run

The dev build of the wallet must already be installed on the
emulator/simulator (`expo run:ios` or `expo run:android` from
`mobile-wallet/`).

```sh
export WALLET_NODE_URL=http://satyawan.local:8080
# Optional — recipient for spec 03 (defaults to all-zero address).
export RECIPIENT_ADDRESS=0x0000…0001

# All flows
bash .maestro/run.sh

# One flow
maestro test .maestro/01-create-wallet.yaml
```

## Debug

```sh
# Interactive — point/click to discover testIDs.
maestro studio

# Per-flow trace + screenshots in .maestro/output/.
maestro test --debug-output .maestro/output .maestro/03-send-and-track.yaml
```

## Known limits

- `_create-wallet.yaml` unrolls 24 indices because Maestro YAML lacks
  loops. Each iteration is a no-op for the 21 indices not picked.
- Spec 03 confirms via `LocalAuthentication`. On the iOS simulator,
  trigger Face ID success with `Features → Face ID → Matching Face`
  during the test. Real devices need an enrolled biometric.
- Mobile uses `Alert.alert("Sent!", …)` rather than the extension's
  in-popup toast — the spec asserts the dialog text + the pending pill.
