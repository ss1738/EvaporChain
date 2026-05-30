// GalleryThatForgets — Cultural lane's first backed primitive
// (GALLERY_FORGETS, 0x0001_0401).
//
// Doctrine claim (from the catalogue): "Thermodynamic-closing gallery
// + Mayflies + AI-decay-art seed. 'The first thing humans have made
// that is provably going to die.'"
//
// The contract IS the gallery. Its own energy is the exhibition's
// lifespan — when it evaporates, the gallery is permanently closed,
// every piece becomes memory, no new method calls succeed. The
// closing date is not a metadata field that an operator can postpone
// indefinitely; it's a structural property of the chain's decay.
//
// One contract = one gallery. The curator (owner) opens the gallery
// once, then `add_piece`s freely while alive. Each piece is a
// content-hash record (the cleartext lives off-chain — IPFS / Arweave
// / wherever); the chain holds attestation + lifecycle, not bytes.
// Curator may `remove_piece` (de-accession) and `close_early` (end
// the exhibition before the contract evaporates).
//
// Pairs naturally with `mayfly.es` — a Mayfly NFT can be added as a
// piece via its contract address (use add_piece(mayfly_addr_hex));
// when the Mayfly evaporates, its slot in the gallery becomes a
// pointer to nothing. The gallery's own lifespan bounds the longest
// any piece can be exhibited.

contract GalleryThatForgets {
    state {
        // ── gallery metadata ───────────────────────────────────────
        gallery_name: string = ""
        opened_at_epoch: u64 = 0
        sealed: bool = false

        // ── pieces ─────────────────────────────────────────────────
        // piece_id -> 1 if currently exhibited, 0 if absent/removed.
        piece_active: map[u64 -> u64]
        // piece_id -> off-chain content hash (URI, CID, etc.).
        piece_hash: map[u64 -> string]
        // piece_id -> when added.
        piece_added_epoch: map[u64 -> u64]

        // Monotonic id allocator. Removed pieces' ids never recycle.
        next_piece_id: u64 = 1

        // ── counters ───────────────────────────────────────────────
        active_count: u64 = 0
        total_added: u64 = 0
        total_removed: u64 = 0

        // ── early-close flag ───────────────────────────────────────
        closed_early: bool = false
    }

    // Curator-only, one-shot: open the gallery with a name.
    fn open(name: string) {
        require(caller == owner, "only curator opens")
        require(self.sealed == false, "gallery already opened")
        self.gallery_name = name
        self.opened_at_epoch = epoch
        self.sealed = true
        emit("gallery opened")
    }

    // Curator-only: add a new piece (auto-assigned id). The id can
    // be retrieved off-chain by reading `next_piece_id` before the
    // call (the assigned id == `next_piece_id` at call-time).
    fn add_piece(content_hash: string) {
        require(caller == owner, "only curator adds pieces")
        require(self.sealed == true, "gallery not yet opened")
        require(self.closed_early == false, "gallery already closed")
        self.piece_active[self.next_piece_id] = 1
        self.piece_hash[self.next_piece_id] = content_hash
        self.piece_added_epoch[self.next_piece_id] = epoch
        self.next_piece_id += 1
        self.active_count += 1
        self.total_added += 1
        emit("piece added")
    }

    // Curator-only: de-accession a piece. Slot id stays reserved
    // (never recycles) so any downstream reference to that id
    // reliably reads "not active."
    fn remove_piece(piece_id: u64) {
        require(caller == owner, "only curator removes pieces")
        require(self.piece_active[piece_id] == 1, "piece not active")
        self.piece_active[piece_id] = 0
        self.piece_hash[piece_id] = ""
        self.active_count -= 1
        self.total_removed += 1
        emit("piece removed")
    }

    // Curator-only: end the exhibition before the contract evaporates.
    // Soft equivalent of letting the contract die — useful when the
    // curator wants a specific closing date earlier than evaporation.
    fn close_early() {
        require(caller == owner, "only curator closes")
        require(self.sealed == true, "gallery not yet opened")
        require(self.closed_early == false, "already closed")
        self.closed_early = true
        emit("gallery closed early")
    }

    // ── Views ──────────────────────────────────────────────────────
    fn is_open() -> bool {
        if self.sealed == false {
            return false
        }
        if self.closed_early == true {
            return false
        }
        return true
    }

    fn is_piece_active(piece_id: u64) -> bool {
        return self.piece_active[piece_id] == 1
    }

    fn piece_hash_view(piece_id: u64) -> string {
        require(self.piece_active[piece_id] == 1, "piece not active")
        return self.piece_hash[piece_id]
    }

    fn piece_added(piece_id: u64) -> u64 {
        return self.piece_added_epoch[piece_id]
    }

    fn age_since_open() -> u64 {
        if self.sealed == false {
            return 0
        }
        return epoch - self.opened_at_epoch
    }

    fn gallery_name_view() -> string {
        require(self.sealed == true, "gallery not yet opened")
        return self.gallery_name
    }

    fn opened_at() -> u64 {
        return self.opened_at_epoch
    }

    fn active_pieces() -> u64 {
        return self.active_count
    }

    fn pieces_ever_added() -> u64 {
        return self.total_added
    }

    fn pieces_ever_removed() -> u64 {
        return self.total_removed
    }

    fn next_id() -> u64 {
        return self.next_piece_id
    }

    fn is_sealed() -> bool {
        return self.sealed
    }

    fn is_closed_early() -> bool {
        return self.closed_early
    }

    on_grace() {
        emit("gallery fading — refresh to extend the exhibition")
    }

    on_refresh() {
        emit("gallery refreshed — extending the exhibition past nature")
    }

    // The doctrine point. Once this fires, the gallery is permanently
    // closed; no method can be called; every piece is now memory.
    on_evaporate() {
        emit("gallery evaporated — every piece is now memory")
    }
}
