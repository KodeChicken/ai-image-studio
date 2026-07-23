.PHONY: check test build dev migrate

check:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cd frontend && pnpm lint && pnpm typecheck

test:
	cargo test --workspace
	cd frontend && pnpm test

build:
	cd frontend && pnpm build
	cargo build --workspace --release

dev:
	cargo run --package ai-image-studio -- serve

migrate:
	cargo run --package ai-image-studio -- migrate

