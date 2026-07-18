# Linux reproduction image for the full quant-sim pipeline (Rust + Python).
# Build:  docker build -t quant-sim .
# Verify: docker run --rm quant-sim just ci
# Demo:   docker run --rm quant-sim cargo run --release -p quant-sim -- run scenarios/hawkes_demo.toml --seed 42

FROM rust:1.96-bookworm

# uv (Python toolchain) + just (task runner) + nextest (test runner)
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/local/bin/
RUN cargo install just cargo-nextest --locked

WORKDIR /quant-sim
COPY . .

RUN cargo build --workspace --release --locked

CMD ["just", "ci"]
