# Public Testnet Faucet

`evaporchain-faucet` is a standalone HTTP service that fronts a running
`evaporchain-node`'s `/api/faucet` endpoint with per-IP rate limiting,
optional CAPTCHA, and a minimal claim form. Run it on its own port (or
host) and keep the chain's API on a private network — the faucet is the
only thing public.

## How to run

```
evaporchain-faucet \
  --port 7676 \
  --node-url http://127.0.0.1:8080 \
  --admin-token "$EVAPORCHAIN_ADMIN_KEY" \
  --rate-limit-per-12h 1 \
  --rate-limit-storage /var/lib/evaporchain-faucet/claims.json
```

If `EVAPORCHAIN_ADMIN_KEY` isn't set on the node, leave `--admin-token`
empty. Pass `--captcha-key <secret>` to enable Cloudflare Turnstile or
hCaptcha (auto-detected); ship a custom `index.html` for the matching
widget.

## Endpoints

| Method | Path     | Purpose |
|--------|----------|---------|
| GET    | `/`      | Static claim form (light, no CDN) |
| POST   | `/claim` | `{address, captcha_response?}` → forwards to node `/api/faucet` |
| GET    | `/health`| `{ok, claims_total, claims_today, rate_limit_per_12h, captcha_enabled, max_claim_amount}` |

## Behind nginx / Caddy with TLS

Caddy:

```
faucet.testnet.evaporchain.com {
  reverse_proxy 127.0.0.1:7676
}
```

nginx (essentials):

```
location / {
  proxy_pass http://127.0.0.1:7676;
  proxy_set_header X-Forwarded-For $remote_addr;
  proxy_set_header X-Real-IP $remote_addr;
}
```

The faucet honours `X-Forwarded-For` and `X-Real-IP`, so per-IP limits
stay correct behind a proxy.

## Monitoring

- Scrape `/health` for `claims_total` (lifetime) and `claims_today`
  (since UTC midnight). Cheap GET, no auth.
- `claims.json` is plain JSON; back it up if you care about history
  surviving a re-image.
