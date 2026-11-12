.PHONY: check
check:
	nix fmt -- --ci
	cargo clippy --all-targets -- -D warnings
	cargo test

.PHONY: integration
integration:
	export T3_BIN="$$(command -v t3)" && cargo test -p t3-client --test live_server -- --ignored

.PHONY: fmt
fmt:
	nix fmt

.PHONY: build
build:
	cargo build --release
