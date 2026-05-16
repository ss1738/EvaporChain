.PHONY: build test test-compile lint lint-strict fmt fmt-check bench check audit-canaries

build:
	cargo build --workspace

# Compile every test binary without running. Catches test-only compile
# regressions much faster than `cargo test`.
test-compile:
	cargo test --workspace --no-run

test:
	cargo test --workspace

# `lint` does not deny warnings — there is a backlog of pre-existing
# rustc-1.94 stylistic lint hits across the research-crate tree that
# need a dedicated sweep. `lint-strict` is the post-cleanup target.
lint:
	cargo clippy --workspace --all-targets

lint-strict:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

bench:
	cd prototypes/fold-a-block && cargo run --release

# Regression-gate for closed audit findings.  Targeted grep over the
# tree to verify previously-shipped fixes haven't been silently
# overwritten.  Motivation: the 2026-05-16 audit round caught 10
# closures (R3/R4/R6/R7 + DRIFT-N3) that a single large merge
# dropped 24 hours after they shipped.  Each canary maps to a known
# closed finding; see `scripts/audit-canaries.sh` for the catalogue.
audit-canaries:
	./scripts/audit-canaries.sh

# Pre-PR gate. Excludes lint-strict until the backlog is cleared.
check: fmt-check lint build test-compile audit-canaries
