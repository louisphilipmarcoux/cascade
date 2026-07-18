# quant-sim task runner. Recipes are one-line cargo/uv delegations so they run
# identically on Linux and Windows (Git Bash provides sh on Windows).

set shell := ["sh", "-cu"]
set windows-shell := ["sh", "-cu"]

default: test

# One-time developer setup (tools are optional; recipes degrade gracefully).
setup:
    cargo install cargo-nextest cargo-deny cargo-mutants --locked

fmt:
    cargo fmt --all

# Full lint gate, identical to CI.
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    @if command -v cargo-nextest >/dev/null 2>&1; then cargo nextest run --workspace --all-features; else cargo test --workspace --all-features; fi

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

deny:
    cargo deny check advisories licenses sources bans

# Kani formal proofs (Linux/WSL only; on Windows run `wsl -- just kani` or rely on CI).
kani:
    @if [ "$(uname -s)" = "Linux" ]; then cargo kani -p matching-engine; else echo "kani requires Linux (use WSL or CI)"; exit 1; fi

bench:
    cargo bench --workspace

mutants:
    cargo mutants -p matching-engine -p backtester --timeout 300

# Python research layer (uv-managed).
research-sync:
    cd research && uv sync --locked

research-test:
    cd research && uv run ruff check && uv run pytest -m "not slow and not network"

# Deterministic demo: run twice, verify identical event-stream hashes.
demo:
    cargo run --release -p quant-sim -- run scenarios/hawkes_demo.toml --seed 42

# Local aggregate gate, mirrors CI.
ci: lint test doc
