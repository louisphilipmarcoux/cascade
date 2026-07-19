"""Normalization on the committed fixture (no network)."""

import polars as pl
import pytest

from quantsim_research.config import (
    FIXTURE_DATE,
    FIXTURE_HOUR,
    STUDY_MARKET,
    STUDY_SYMBOL,
    fixtures_dir,
)

TRADES = (
    fixtures_dir()
    / f"binance-{STUDY_MARKET}"
    / "trades"
    / f"{STUDY_SYMBOL}-{FIXTURE_DATE}-h{FIXTURE_HOUR:02}.parquet"
)
TRADES_CSV = TRADES.with_suffix(".csv")

pytestmark = pytest.mark.skipif(
    not TRADES.exists(), reason="fixtures not built (run `quantsim-data fixture`)"
)


def test_fixture_trades_shape_and_ordering():
    trades = pl.read_parquet(TRADES)
    assert trades.columns == ["ts_ns", "side", "price", "qty", "agg_trade_id"]
    assert len(trades) > 1_000
    assert trades["ts_ns"].is_sorted()
    assert set(trades["side"].unique().to_list()) <= {-1, 1}
    assert (trades["qty"] > 0).all()
    assert (trades["price"] > 0).all()
    # All timestamps inside the fixture hour.
    span_ns = trades["ts_ns"].max() - trades["ts_ns"].min()
    assert span_ns < 3_600_000_000_000


def test_rust_csv_twin_matches_parquet():
    trades = pl.read_parquet(TRADES)
    csv = pl.read_csv(TRADES_CSV)
    assert csv.columns == ["ts_ns", "side", "price_e8", "qty_e8", "agg_trade_id"]
    assert len(csv) == len(trades)
    assert (csv["ts_ns"] == trades["ts_ns"]).all()
    # e8 scaling round-trips within one ulp of the float representation.
    reconstructed = csv["price_e8"].cast(pl.Float64) / 1e8
    assert ((reconstructed - trades["price"]).abs() < 1e-7).all()
    # Data reality (measured, not assumed): although the *nominal* BTCUSDT
    # um tick filter was 0.10 in Jan 2024, a handful of prints (8/41473 on
    # the fixture day — liquidations / legacy-precision artifacts) land on
    # the 0.01 grid. The replay instrument therefore uses tick 0.01; every
    # print must be exact on it.
    assert (csv["price_e8"] % 1_000_000 == 0).all()
    # Quantities are exact on the 0.001 step grid.
    assert (csv["qty_e8"] % 100_000 == 0).all()
