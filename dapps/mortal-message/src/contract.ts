// Single source of truth: `contracts/evaporscript/mortal_message.es`.
// The canonical EvaporScript pilot contract. Cargo pilot lives at
// `crates/evaporchain-script/tests/mortal_message_pilot.rs`.

export const MORTAL_MESSAGE_SOURCE = `contract MortalMessage {
    state {
        body: string = ""
        recipient: address
        sender: address
        sealed: bool = false
        boost_count: u64 = 0
        last_boost_epoch: u64 = 0
    }

    fn set_payload(payload_body: string, payload_recipient: address) {
        require(caller == owner, "only sender can seal")
        require(self.sealed == false, "message already sealed")
        self.body = payload_body
        self.recipient = payload_recipient
        self.sender = owner
        self.sealed = true
        emit("message sealed")
    }

    fn read() -> string {
        require(self.sealed == true, "message not yet sealed")
        require(
            caller == self.recipient || caller == owner,
            "not authorized"
        )
        return self.body
    }

    fn record_boost() {
        require(self.sealed == true, "message not yet sealed")
        self.boost_count += 1
        self.last_boost_epoch = epoch
        emit("message boosted")
    }

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
}`;
