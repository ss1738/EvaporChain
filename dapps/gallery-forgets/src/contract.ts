// Single source of truth: `contracts/evaporscript/gallery_forgets.es`.
// Byte-stable inline copy.

export const GALLERY_FORGETS_SOURCE = `contract GalleryThatForgets {
    state {
        gallery_name: string = ""
        opened_at_epoch: u64 = 0
        sealed: bool = false

        piece_active: map[u64 -> u64]
        piece_hash: map[u64 -> string]
        piece_added_epoch: map[u64 -> u64]

        next_piece_id: u64 = 1

        active_count: u64 = 0
        total_added: u64 = 0
        total_removed: u64 = 0

        closed_early: bool = false
    }

    fn open(name: string) {
        require(caller == owner, "only curator opens")
        require(self.sealed == false, "gallery already opened")
        self.gallery_name = name
        self.opened_at_epoch = epoch
        self.sealed = true
        emit("gallery opened")
    }

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

    fn remove_piece(piece_id: u64) {
        require(caller == owner, "only curator removes pieces")
        require(self.piece_active[piece_id] == 1, "piece not active")
        self.piece_active[piece_id] = 0
        self.piece_hash[piece_id] = ""
        self.active_count -= 1
        self.total_removed += 1
        emit("piece removed")
    }

    fn close_early() {
        require(caller == owner, "only curator closes")
        require(self.sealed == true, "gallery not yet opened")
        require(self.closed_early == false, "already closed")
        self.closed_early = true
        emit("gallery closed early")
    }

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

    on_evaporate() {
        emit("gallery evaporated — every piece is now memory")
    }
}
`;
