.PHONY: check
check:
	nix fmt -- --ci
	ruff check tools/parity
	python3 -m unittest discover -s tools/parity -p 'test_*.py'
	cargo clippy --all-targets -- -D warnings
	cargo test -p accesskit_atspi_common
	cargo test

.PHONY: integration
integration:
	export T3_BIN="$$(command -v t3)" && cargo test -p t3-client --test live_server -- --ignored
	export T3_BIN="$$(command -v t3)" && cargo test -p t3-terminal --test live_session -- --ignored

.PHONY: fmt
fmt:
	nix fmt

.PHONY: build
build:
	cargo build --release
