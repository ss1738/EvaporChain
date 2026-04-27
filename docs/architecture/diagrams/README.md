# EvaporChain Architecture Diagrams

Mermaid diagram source files. GitHub renders `.mmd` natively. For local rendering use [mermaid-cli](https://github.com/mermaid-js/mermaid-cli) or paste into [mermaid.live](https://mermaid.live).

| File | What it shows |
|------|---------------|
| [`tx_lifecycle.mmd`](./tx_lifecycle.mmd) | Transaction journey: client → mempool → block → execution → finalization → DA attestation |
| [`consensus_state_machine.mmd`](./consensus_state_machine.mmd) | Tendermint phases: propose → prevote → precommit → commit, with view-change |
| [`da_flow.mmd`](./da_flow.mmd) | Data availability: encode → row/col commitments → cell proofs → light-client sample → certificate |
| [`validator_key_lifecycle.mmd`](./validator_key_lifecycle.mmd) | BLS + ML-DSA key generate → store → load → sign → rotate |
| [`cross_shard_messaging.mmd`](./cross_shard_messaging.mmd) | Origin shard → receipt → destination shard execute |

**Audience:** external auditors, validator operators, contributors getting their bearings.

**Scope:** these diagrams describe the protocol design as built in the workspace today (commit hash to be added at audit kickoff). When code drifts from the diagrams, fix the diagrams.
