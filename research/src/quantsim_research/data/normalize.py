"""Raw archives → normalized interchange data.

Two twins per dataset (docs/interchange.md):

- **Parquet** (Float64 prices, epoch-ns Int64) — the research-side format;
- **Rust CSV** (scaled-i64 ``price_e8``/``qty_e8``) — the deterministic
  cross-boundary format the replay engine reads.

Timestamp units are sniffed by magnitude (spot switched ms→µs at
2025-01-01; futures are ms), and header rows are sniffed rather than
assumed (spot files historically lack them).

e8 scaling note: values are parsed as f64 and scaled with round-to-nearest.
For prices < 1e7 quote with ≤ 8 decimal places this is exact within f64's
15–16 significant digits; the Rust side re-validates tick alignment on
load (`round(price/tick) must round-trip`), so a violation cannot pass
silently.
"""

from __future__ import annotations

import shutil
import tempfile
import zipfile
from pathlib import Path

import polars as pl

from quantsim_research.config import normalized_dir
from quantsim_research.data.binance_vision import RAW_COLUMNS, Archive


def _sniff_ns(ts: int) -> int:
    """Multiplier turning a sniffed epoch magnitude into nanoseconds."""
    if ts < 100_000_000_000_000:  # < 1e14 → milliseconds (~5138 AD in ms)
        return 1_000_000
    if ts < 100_000_000_000_000_000:  # < 1e17 → microseconds
        return 1_000
    return 1


def read_raw_csv(zip_path: Path, columns: list[str]) -> pl.DataFrame:
    """Read the single CSV inside a Binance Vision zip, header-sniffing.

    Streams the decompressed CSV to a temporary file (bookTicker days run to
    gigabytes decompressed — no single in-memory buffer) and lets polars
    read from disk.
    """
    with zipfile.ZipFile(zip_path) as zf:
        inner = zf.namelist()[0]
        with zf.open(inner) as stream:
            head = stream.read(256).split(b"\n", 1)[0]
        has_header = not head.split(b",")[0].strip().isdigit()
        with tempfile.NamedTemporaryFile(suffix=".csv", delete=False) as tmp:
            tmp_path = Path(tmp.name)
            with zf.open(inner) as stream:
                shutil.copyfileobj(stream, tmp, length=1 << 20)
    try:
        return pl.read_csv(
            tmp_path,
            has_header=has_header,
            new_columns=None if has_header else columns,
            infer_schema_length=10_000,
            low_memory=True,
        )
    finally:
        tmp_path.unlink(missing_ok=True)


def normalize_agg_trades(zip_path: Path, archive: Archive) -> pl.DataFrame:
    """Raw aggTrades → the normalized `trades` frame (ts_ns, side, price, qty, id)."""
    columns = RAW_COLUMNS[(archive.market, "aggTrades")]
    df = read_raw_csv(zip_path, columns)
    ts_sample = int(df["transact_time"][0])
    multiplier = _sniff_ns(ts_sample)
    maker_col = df["is_buyer_maker"]
    if maker_col.dtype != pl.Boolean:
        maker = maker_col.cast(pl.Utf8).str.to_lowercase() == "true"
    else:
        maker = maker_col
    return (
        df.with_columns(
            (pl.col("transact_time").cast(pl.Int64) * multiplier).alias("ts_ns"),
            # aggressor bought unless the buyer was the maker
            pl.when(maker).then(-1).otherwise(1).cast(pl.Int8).alias("side"),
            pl.col("price").cast(pl.Float64),
            pl.col("quantity").cast(pl.Float64).alias("qty"),
            pl.col("agg_trade_id").cast(pl.Int64),
        )
        .select("ts_ns", "side", "price", "qty", "agg_trade_id")
        .sort("ts_ns", "agg_trade_id")
    )


def normalize_book_ticker(zip_path: Path, archive: Archive) -> pl.DataFrame:
    """Raw um bookTicker → normalized L1 quotes frame."""
    columns = RAW_COLUMNS[(archive.market, "bookTicker")]
    df = read_raw_csv(zip_path, columns)
    multiplier = _sniff_ns(int(df["transaction_time"][0]))
    return (
        df.with_columns(
            (pl.col("transaction_time").cast(pl.Int64) * multiplier).alias("ts_ns"),
            pl.col("update_id").cast(pl.Int64),
            pl.col("best_bid_price").cast(pl.Float64).alias("bid_px"),
            pl.col("best_bid_qty").cast(pl.Float64).alias("bid_qty"),
            pl.col("best_ask_price").cast(pl.Float64).alias("ask_px"),
            pl.col("best_ask_qty").cast(pl.Float64).alias("ask_qty"),
        )
        .select("ts_ns", "update_id", "bid_px", "bid_qty", "ask_px", "ask_qty")
        .sort("ts_ns", "update_id")
    )


def e8(expr: pl.Expr) -> pl.Expr:
    return (expr * 100_000_000.0).round(0).cast(pl.Int64)


def trades_to_rust_csv(trades: pl.DataFrame, out: Path) -> None:
    """The scaled-i64 `trades` interchange CSV consumed by the Rust replay."""
    out.parent.mkdir(parents=True, exist_ok=True)
    frame = trades.select(
        pl.col("ts_ns"),
        pl.col("side"),
        e8(pl.col("price")).alias("price_e8"),
        e8(pl.col("qty")).alias("qty_e8"),
        pl.col("agg_trade_id"),
    )
    frame.write_csv(out, line_terminator="\n")


def quotes_to_rust_csv(quotes: pl.DataFrame, out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    frame = quotes.select(
        pl.col("ts_ns"),
        pl.col("update_id"),
        e8(pl.col("bid_px")).alias("bid_e8"),
        e8(pl.col("bid_qty")).alias("bid_qty_e8"),
        e8(pl.col("ask_px")).alias("ask_e8"),
        e8(pl.col("ask_qty")).alias("ask_qty_e8"),
    )
    frame.write_csv(out, line_terminator="\n")


def write_partition(frame: pl.DataFrame, table: str, archive: Archive) -> Path:
    """Hive-partitioned Parquet under data/normalized/."""
    out = (
        normalized_dir()
        / table
        / f"exchange=binance-{archive.market}"
        / f"symbol={archive.symbol}"
        / f"date={archive.date}"
        / "part-0.parquet"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    frame.write_parquet(out, compression="zstd", statistics=True)
    return out
