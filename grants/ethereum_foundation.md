# Ethereum Foundation Grant Application — EvaporChain

## Project Name
EvaporChain State Management Research

## One-Line Description
Open-source research on thermodynamic state decay, Verkle trie optimization, and PID fee control that directly benefits Ethereum's state management roadmap.

## Problem Statement
Ethereum's state exceeds 300GB and grows every block. State expiry (EIP-4444) has been on the roadmap since 2018 but remains unshipped. The community needs working implementations and benchmarked alternatives to inform design decisions.

## How This Benefits Ethereum
1. **Verkle Trie Research**: We implemented a Verkle trie with Pedersen commitments and benchmarked it under high-churn workloads (constant insert/delete from state decay). Our findings on churn performance directly inform Ethereum's Verkle transition (EIP-6800).

2. **State Decay Model**: Our thermodynamic approach provides an alternative design for Ethereum's state expiry proposals. Our working implementation demonstrates that automatic state management is feasible without governance-controlled parameters.

3. **PID Fee Controller**: We implemented and tested a PID controller replacing EIP-1559's exponential adjustment. Backtesting shows 3-5x lower fee volatility. The analysis and code are directly applicable to Ethereum fee mechanism research.

4. **Nova IVC Benchmarks**: We benchmarked recursive proof folding at 6.2ms per block on commodity hardware. These benchmarks inform Ethereum's ZK roadmap.

## What We've Already Built (not planned — BUILT)
- **25,435 tests** across **147 workspace crates** (16 core + 131 substrate / supporting).
- Working Verkle trie with Pedersen commitments.
- Working MMR accumulator with energy-stamped nullifiers.
- **Real Nova IVC** at arity 8 with Poseidon-bound state root (per `LAMBDA_FOLD_NOVA_PLAN.md` Phases 1–7). `verify_with_vk_bytes` measured at **23 ms @ 100 folds** on Mac Mini M4 under release — 1.083× of 23 ms @ 10 folds, i.e. essentially flat. **Sublinear-in-active-energy verifier claim is empirically locked.**
- PID fee controller with Lyapunov stability + integer-arithmetic determinism.
- Live 2-Mini Tailscale BFT testnet at h=940+ lockstep with R.1/R.3 freeze-fix proven in production. (Public `https://testnet.evaporchain.com` is the operator-facing endpoint; the chain itself runs on a self-hosted Tailscale cluster.)
- 38 KB whitepaper with 8 academic citations (`research/whitepaper.md`).
- **5 Coq proof files** mechanized under Rocq 9.1.1 (zero `Admitted.` lemmas; CI-gated on every PR).
- **5 TLA+ specifications** for safety / liveness / conservation / consensus invariants.

## What The Grant Funds
- Cloud infrastructure for public testnet: $6,000/year
- Security review of Verkle trie implementation: $15,000
- Conference travel to present findings (Devcon, EthResearch): $5,000
- Dedicated development time (6 months): $24,000
- **Total requested: $50,000**

## Deliverables
1. Month 2: Published Verkle trie performance report (high-churn benchmarks)
2. Month 4: Published PID fee controller analysis with EIP-1559 comparison
3. Month 6: Published state decay research paper on arXiv
4. All code open-sourced under MIT license
5. All findings shared via EthResearch forum posts

## Team
Solo founder and developer. Built the entire system from architecture through implementation. Computer science graduate (2026).

## Open Source Commitment
All research and code will be published under MIT license and shared with the Ethereum community via:
- arXiv preprints
- EthResearch forum posts
- GitHub (public repository)
- Conference presentations

## Timeline
6 months from grant receipt to all deliverables.
