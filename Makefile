.PHONY: build test test-compile lint lint-strict fmt fmt-check bench check

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

# Pre-PR gate. Excludes lint-strict until the backlog is cleared.
check: fmt-check lint build test-compile
