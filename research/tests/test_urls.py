"""URL construction and dataset-availability rules."""

import pytest

from quantsim_research.data.binance_vision import Archive


def test_um_agg_trades_daily_url():
    a = Archive("um", "aggTrades", "BTCUSDT", "2024-06-02")
    assert a.url() == (
        "https://data.binance.vision/data/futures/um/daily/aggTrades/BTCUSDT/"
        "BTCUSDT-aggTrades-2024-06-02.zip"
    )
    assert a.checksum_url().endswith(".zip.CHECKSUM")


def test_spot_monthly_klines_url():
    a = Archive("spot", "klines", "BTCUSDT", "2024-06", monthly=True, interval="1m")
    assert a.url() == (
        "https://data.binance.vision/data/spot/monthly/klines/BTCUSDT/1m/BTCUSDT-1m-2024-06.zip"
    )


def test_spot_has_no_book_data():
    with pytest.raises(ValueError, match="no book data"):
        Archive("spot", "bookTicker", "BTCUSDT", "2024-06-02").validate()
