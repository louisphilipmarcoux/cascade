"""Coinbase Exchange candles — the cross-venue sanity gate.

Not a data source for modeling; a tripwire: if Binance and Coinbase hourly
closes diverge materially over the study window, either a venue had an
anomaly or our pipeline corrupted something. Alert thresholds are
preregistered here rather than tuned after the fact.
"""

from __future__ import annotations

import datetime as dt

import httpx
import polars as pl

BASE = "https://api.exchange.coinbase.com"
MAX_CANDLES = 300

# Preregistered gate thresholds.
MAX_MEDIAN_CLOSE_DEVIATION = 0.005  # 0.5%
MIN_HOURLY_RETURN_CORRELATION = 0.98


def hourly_candles(product: str, start: str, end: str) -> pl.DataFrame:
    """Fetch hourly candles [start, end) in ≤300-candle pages."""
    client = httpx.Client(timeout=30.0, headers={"User-Agent": "quant-sim-research"})
    rows: list[tuple[int, float, float]] = []
    cursor = dt.datetime.fromisoformat(f"{start}T00:00:00+00:00")
    stop = dt.datetime.fromisoformat(f"{end}T00:00:00+00:00")
    step = dt.timedelta(hours=MAX_CANDLES)
    try:
        while cursor < stop:
            window_end = min(cursor + step, stop)
            response = client.get(
                f"{BASE}/products/{product}/candles",
                params={
                    "granularity": 3600,
                    "start": cursor.isoformat(),
                    "end": window_end.isoformat(),
                },
            )
            response.raise_for_status()
            for t, _low, _high, _open, close, volume in response.json():
                rows.append((int(t) * 1_000_000_000, float(close), float(volume)))
            cursor = window_end
    finally:
        client.close()
    return (
        pl.DataFrame(rows, schema=["ts_ns", "close", "volume"], orient="row")
        .unique(subset="ts_ns")
        .sort("ts_ns")
    )


def sanity_gate(binance_hourly: pl.DataFrame, coinbase_hourly: pl.DataFrame) -> dict:
    """Join on the hour and evaluate the preregistered thresholds."""
    joined = binance_hourly.join(coinbase_hourly, on="ts_ns", suffix="_cb").drop_nulls()
    deviation = (
        ((joined["close"] - joined["close_cb"]).abs() / joined["close_cb"]).median()
        if len(joined)
        else None
    )
    returns = joined.select(
        (pl.col("close").log().diff()).alias("r_b"),
        (pl.col("close_cb").log().diff()).alias("r_c"),
    ).drop_nulls()
    correlation = float(returns.select(pl.corr("r_b", "r_c")).item()) if len(returns) > 2 else None
    passed = (
        deviation is not None
        and correlation is not None
        and deviation < MAX_MEDIAN_CLOSE_DEVIATION
        and correlation > MIN_HOURLY_RETURN_CORRELATION
    )
    return {
        "hours_joined": len(joined),
        "median_close_deviation": deviation,
        "hourly_return_correlation": correlation,
        "passed": passed,
    }
