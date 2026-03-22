.PHONY: build test lint fmt bench check

build:
	cargo build --workspace

test:
	cargo test --workspace

lint:
	cargo clippy --workspace -- -D warnings

fmt:
	cargo fmt --all

bench:
	cd prototypes/fold-a-block && cargo run --release

check: fmt lint test
