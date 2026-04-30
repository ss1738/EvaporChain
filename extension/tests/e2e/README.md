# Wallet extension — Playwright e2e

End-to-end harness for the EvaporChain MV3 wallet. Drives the actual
built `dist/` extension in real Chromium against a live node.

## Run on a Mini

Builds and tests are policy-restricted to the Minis (apsarth /
satyawan / etc) — never on the MacBook.

```bash
# one-time, on the Mini
cd /path/to/EvaporChain/extension
npm install
npx playwright install chromium    # ~150MB download
npm run build                       # produces dist/

# every run
WALLET_NODE_URL=http://localhost:8080 npm run test:e2e
```

`WALLET_NODE_URL` defaults to `http://satyawan.local:8080` (Tailscale
hostname for mini-1). `globalSetup` aborts the suite if the node's
`/api/status` is unreachable.

## Debug

```bash
PWDEBUG=1 npm run test:e2e -- specs/03-send-and-track.spec.ts
```

Opens the Playwright Inspector. Traces / videos / screenshots are
retained on failure under `test-results/`.

## How it works

* `playwright.config.ts` resolves `dist/` and the node URL, sets a
  120s timeout, single worker, retries:0.
* `fixtures.ts` launches `chromium.launchPersistentContext` with
  `--disable-extensions-except` + `--load-extension`. Each test gets
  a fresh on-disk profile, so `chrome.storage.local` is clean.
* The extension ID is resolved by waiting for the MV3 service worker
  to register and parsing `chrome-extension://<id>/...` from its URL.
* The Zustand store is exposed on `globalThis.__zustandStore` only
  when `import.meta.env.MODE === "test"` (see `useWallet.ts` bottom).
  Specs use it to drive flows without coupling to UI labels.

## Known limitations / flake risks

* MV3 service workers can't run in `headless: true`; the suite uses
  headed Chromium. Set `xvfb-run` on Linux Minis if needed.
* Service worker registration is async — the extension-ID fixture
  waits up to 20s. If your Mini is overloaded, bump the timeout.
* `serviceWorkers()` returns the background context; use it (not
  `pages()`) when you need to inspect background-side state.
* The popup is opened by `goto`-ing `popup.html` directly, not via
  the toolbar action. Behaviour is identical for our purposes but
  toolbar-action quirks (e.g. `chrome.action.onClicked`) won't trigger.
* `chrome.storage.local.clear()` runs from the popup page. The
  background SW has its own context — if a future spec needs to wipe
  state seen only by the SW, add a worker-side wipe via
  `extensionContext.serviceWorkers()[0].evaluate(...)`.
