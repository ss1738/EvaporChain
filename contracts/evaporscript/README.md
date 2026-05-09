# EvaporScript Stdlib

Reference contracts demonstrating decay-native primitives in EvaporScript.

Every contract here ships the same shape: a header block explaining the **decay-thesis hook** (what makes this version unlike the equivalent on every other chain), a `state` block, deployer-gated setup methods, open-or-gated business methods, and the canonical lifecycle hooks (`on_grace`, `on_refresh`, `on_evaporate`).

## Pilot trio (original — drove the dApps in `apps/`)

| File | Hook |
|---|---|
| [`mortal_message.es`](mortal_message.es) | The contract's energy IS the message's lifespan |
| [`mortal_nft.es`](mortal_nft.es) | NFT can die — scarcity enforced by entropy, not by storage |
| [`energy_pool.es`](energy_pool.es) | Aggregated energy stake protects a set of objects |

## Stdlib seed-12 (added 2026-05-09)

| # | File | Decay-thesis hook |
|---|---|---|
| 1 | [`payment_split.es`](payment_split.es) | Unclaimed shares lapse — claim while alive or your slice forfeits |
| 2 | [`sealed_bid_auction.es`](sealed_bid_auction.es) | Bid weight decays during reveal — early reveal wins ties |
| 3 | [`vesting_schedule.es`](vesting_schedule.es) | Vested-but-unclaimed amount forfeits at evaporation |
| 4 | [`time_lock.es`](time_lock.es) | Claim window bounded by contract energy, not a second timer |
| 5 | [`attestation.es`](attestation.es) | Strength decays — silence is decay; refresh keeps the claim live |
| 6 | [`oracle_feed.es`](oracle_feed.es) | Stale data can't physically exist on-chain — freshness is enforced by chain physics |
| 7 | [`subscription.es`](subscription.es) | Energy decay IS the cancellation trigger — no off-chain reaper |
| 8 | [`multisig.es`](multisig.es) | Proposal mortal — lost-key signers cannot block forever |
| 9 | [`lottery.es`](lottery.es) | Unresolved draws void by physics — entries refund automatically |
| 10 | [`bounty.es`](bounty.es) | Forgotten bounties refund — no abandonware |
| 11 | [`dead_man_switch.es`](dead_man_switch.es) | The contract EvaporChain was made for — decay IS the trigger |
| 12 | [`energy_marketplace.es`](energy_marketplace.es) | Mortal liquidity — no eternal order book |

## How to deploy

```bash
# Submit raw source to the chain via DeployScript
curl -X POST http://<NODE>:8080/api/tx/deploy-script \
  -H "Content-Type: application/json" \
  -d "$(jq -Rsn --arg src "$(cat payment_split.es)" '{
    deployer: 1,
    source_code: $src,
    energy: 100000,
    half_life: 200
  }')"
```

`energy` and `half_life` together set the contract's lifespan curve. Higher energy = longer initial life. Smaller half_life = faster decay. Pick based on the contract's purpose:

| Contract type | Suggested energy | Suggested half_life |
|---|---|---|
| Single-shot (auction, lottery, bounty) | match the event window | ~½ the event window |
| Recurring (subscription, oracle feed) | enough for one period | ~the period length |
| Long-lived (vesting, time-lock) | full grant duration | ~grant duration |
| Sentinel (dead-man switch) | check-in interval × cushion | ~½ the interval |

## Conventions every contract follows

1. **Header doc** — opens with the decay-thesis hook (one paragraph explaining what would be impossible / forever-broken on a non-decaying chain).
2. **`sealed: bool` flag** — every contract that has multi-call setup uses `sealed` to lock configuration.
3. **`caller == owner` for deployer-gated methods** — the builtin `owner` is the original deployer, immutable after deploy.
4. **Bounded state** — no unbounded loops, no unbounded array allocations. Maps are O(1) lookups; iteration over keys is avoided in the pilot grammar.
5. **Lifecycle trio** — `on_grace`, `on_refresh`, `on_evaporate` always wired (even if `on_refresh` only emits an event).
6. **Doctrine moment in `on_evaporate`** — every contract documents what its evaporation means: forfeit, void, refund, release. The decay-thesis hook stated up top is honoured here.

## Testing

Parser-roundtrip + lifecycle-hook regression for the seed-12 lives in [`crates/evaporchain-script/tests/stdlib_parse_check.rs`](../../crates/evaporchain-script/tests/stdlib_parse_check.rs). Per-contract behavioural pilots (parse + execute + assert state transitions, modelled on `mortal_nft_pilot.rs` etc.) are added incrementally.

```bash
# Verify the stdlib still parses (run on the Mini cluster)
cargo test -p evaporchain-script --test stdlib_parse_check
```
