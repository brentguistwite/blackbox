.PHONY: fmt check clippy test precommit ci run build

fmt:
	cargo fmt

check:
	cargo fmt --check
	cargo check

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test

precommit: check clippy

ci: check clippy test

run:
	cargo run --

build:
	cargo build --release
