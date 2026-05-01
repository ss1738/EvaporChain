// Single source of truth: `contracts/evaporscript/mortal_message.es`.
// This file is the inline copy the dapp ships with — keep it byte-identical
// to the .es file. The Rust integration test
// `crates/evaporchain-script/tests/mortal_message_pilot.rs` is the regression
// barrier that proves this exact source compiles + runs cleanly through the
// VM. Don't edit the body here without running that test.
//
// The deploy flow is two-step:
//   1. POST /api/tx/deploy-script  with `source_code` = MORTAL_MESSAGE_SOURCE
//      → returns {tx_hash}; chain assigns a contract_id at execution time.
//   2. POST /api/tx/call-script    with method = "set_payload",
//      args = [body, recipient]
//      → seals the message body + recipient.
//
// Timing gap: the seal step needs the contract_id from step 1's receipt.
// Polling /api/tx/:hash for state == "finalised" + reading the receipt's
// contract_id is the natural follow-up. For the pilot we expose both
// halves separately so the UI can sequence them.

export const MORTAL_MESSAGE_SOURCE = `// MortalMessage — pilot EvaporScript contract for the mortal-messages dApp.
//
// Each message is a contract instance. The contract's *own* energy IS the
// message's lifespan: when energy hits zero the message evaporates. This
// is the canonical pattern for "the lifecycle is the entity" — the chain
// runtime, not the contract code, drives the message through
// active → grace → ghost.
//
// Why a contract, not a state object: the contract's \`on_grace\` and
// \`on_evaporate\` hooks let us emit events the dApp can subscribe to, and
// the \`read\` method can gate access (sender + recipient only).
//
// Deploy via DeployScript with:
//   - \`energy\`     = initial message lifespan (caller picks)
//   - \`half_life\`  = decay rate (HALF_LIFE_PRESETS in the dApp)
// Then call \`set_payload(body, recipient)\` exactly once to populate.
// Subsequent calls are read-only or boost-only.
//
// All on-chain state is bounded (single string body, single address,
// one-shot init flag) so this stays inside EvaporScript's structural
// totality contract.

contract MortalMessage {
    state {
        body: string = ""
        // \`address\` fields default to all-zero; explicit hex-literal
        // defaults are not part of the EvaporScript surface. The
        // sealed flag below (\`= false\`) is what distinguishes an
        // unpopulated message from a populated one.
        recipient: address
        sender: address
        sealed: bool = false
        boost_count: u64 = 0
        last_boost_epoch: u64 = 0
    }

    // Populate the message exactly once. After \`sealed\` flips to true, the
    // body and recipient are immutable for the lifetime of the contract.
    // Only the original deployer (i.e. the sender) can seal — \`caller ==
    // owner\` enforces that.
    fn set_payload(payload_body: string, payload_recipient: address) {
        require(caller == owner, "only sender can seal")
        require(self.sealed == false, "message already sealed")
        self.body = payload_body
        self.recipient = payload_recipient
        self.sender = owner
        self.sealed = true
        emit("message sealed")
    }

    // Read the message body. Restricted to sender (audit trail) and
    // recipient (intended audience). \`caller == owner\` covers the sender.
    fn read() -> string {
        require(self.sealed == true, "message not yet sealed")
        require(
            caller == self.recipient || caller == owner,
            "not authorized"
        )
        return self.body
    }

    // Boost handler — called by \`on_refresh\` and tracks how many times
    // the message has been kept alive past its natural decay. The chain
    // applies the actual energy delta; this method is just for telemetry.
    fn record_boost() {
        require(self.sealed == true, "message not yet sealed")
        self.boost_count += 1
        self.last_boost_epoch = epoch
        emit("message boosted")
    }

    // Inspect lifecycle state without exposing the body.
    fn inspect() -> u64 {
        return self.boost_count
    }

    on_grace() {
        emit("message energy low — boost to keep alive")
    }

    on_refresh() {
        self.boost_count += 1
        self.last_boost_epoch = epoch
        emit("message boosted")
    }

    on_evaporate() {
        emit("message evaporated")
    }
}
`;
