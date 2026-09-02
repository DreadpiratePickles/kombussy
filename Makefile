# Entry points for the whole workspace. Every target is safe to re-run.

FONTTOOLS := uv run --with "fonttools[woff]" --with brotli python3

.PHONY: all test fixtures interop wasm web cli lint clean

all: test wasm

## Rust unit and corpus tests.
test:
	cargo test --workspace

## Regenerate the synthetic font corpus (requires uv).
fixtures:
	$(FONTTOOLS) fixtures/generate.py

## Cross-check against fontTools in both directions. Builds the CLI first.
interop: cli
	$(FONTTOOLS) fixtures/verify_interop.py

## Native command line converter.
cli:
	cargo build --release -p kombussy-cli

## WebAssembly module, emitted into the web app's source tree.
wasm:
	wasm-pack build crates/kombussy-wasm --target web --out-dir ../../web/src/wasm --release

## Production web build. Depends on the wasm module existing.
web: wasm
	cd web && npm install && npm run build

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

clean:
	cargo clean
	rm -rf web/dist web/src/wasm web/node_modules
