"""Deterministic committed fixtures (≤ 8 MB total).

Built from pinned raw archives so anyone can rebuild byte-identical
fixtures with `quantsim-data fixture`. Consumed by Rust replay tests,
Python unit tests, and CI notebook smoke runs.
"""

from __future__ import annotations

import datetime as dt

import polars as pl

from quantsim_research.config import (
    FIXTURE_DATE,
    FIXTURE_HOUR,
    STUDY_MARKET,
    STUDY_SYMBOL,
    fixtures_dir,
)
from quantsim_research.data import manifest
from quantsim_research.data.binance_vision import Archive
from quantsim_research.data.download import fetch
from quantsim_research.data.normalize import (
    normalize_agg_trades,
    normalize_book_ticker,
    trades_to_rust_csv,
)


def _hour_bounds(date: str, hour: int) -> tuple[int, int]:
    start = dt.datetime.fromisoformat(f"{date}T00:00:00+00:00") + dt.timedelta(hours=hour)
    end = start + dt.timedelta(hours=1)
    return int(start.timestamp() * 1e9), int(end.timestamp() * 1e9)


def build() -> None:
    """Download the pinned fixture-day archives and cut the committed slices."""
    root = fixtures_dir() / f"binance-{STUDY_MARKET}"
    lo, hi = _hour_bounds(FIXTURE_DATE, FIXTURE_HOUR)

    trades_archive = Archive(STUDY_MARKET, "aggTrades", STUDY_SYMBOL, FIXTURE_DATE)
    trades_zip = fetch(trades_archive)
    trades = normalize_agg_trades(trades_zip, trades_archive)
    trades_hour = trades.filter((pl.col("ts_ns") >= lo) & (pl.col("ts_ns") < hi))
    trades_out = root / "trades"
    trades_out.mkdir(parents=True, exist_ok=True)
    trades_hour.write_parquet(
        trades_out / f"{STUDY_SYMBOL}-{FIXTURE_DATE}-h{FIXTURE_HOUR:02}.parquet",
        compression="zstd",
    )
    trades_to_rust_csv(
        trades_hour, trades_out / f"{STUDY_SYMBOL}-{FIXTURE_DATE}-h{FIXTURE_HOUR:02}.csv"
    )

    quotes_archive = Archive(STUDY_MARKET, "bookTicker", STUDY_SYMBOL, FIXTURE_DATE)
    quotes_zip = fetch(quotes_archive)
    quotes = normalize_book_ticker(quotes_zip, quotes_archive)
    # First 10 minutes only — bookTicker is enormous.
    q_hi = lo + 10 * 60 * 1_000_000_000
    quotes_slice = quotes.filter((pl.col("ts_ns") >= lo) & (pl.col("ts_ns") < q_hi))
    quotes_out = root / "quotes"
    quotes_out.mkdir(parents=True, exist_ok=True)
    quotes_slice.write_parquet(
        quotes_out / f"{STUDY_SYMBOL}-{FIXTURE_DATE}-h{FIXTURE_HOUR:02}m00-10.parquet",
        compression="zstd",
    )
    # No CSV twin for quotes: it is ~6 MB (blows the ≤8 MB fixture budget)
    # and only Python consumes L1 quotes today. Regenerate on demand with
    # `quotes_to_rust_csv` when the Rust side grows an L1 reader.

    _write_readme(len(trades_hour), len(quotes_slice))


def _write_readme(n_trades: int, n_quotes: int) -> None:
    entries = manifest.load()
    lines = [
        "# Committed data fixtures",
        "",
        f"Slices of Binance USD-M {STUDY_SYMBOL} for {FIXTURE_DATE} "
        f"(hour {FIXTURE_HOUR:02}), cut deterministically from the pinned raw",
        "archives below. Rebuild with `uv run quantsim-data fixture`.",
        "",
        f"- trades: {n_trades} rows (1 hour of aggTrades)",
        f"- quotes: {n_quotes} rows (first 10 min of bookTicker L1)",
        "",
        "Source archives (see also data/manifest.toml):",
        "",
    ]
    for url in sorted(entries):
        entry = entries[url]
        lines.append(f"- `{entry.sha256[:16]}…` {url}")
    lines.append("")
    (fixtures_dir() / "README.md").write_text("\n".join(lines), encoding="utf-8", newline="\n")
