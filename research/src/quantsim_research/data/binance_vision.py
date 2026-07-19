"""Binance Vision URL construction and raw-schema registry.

Facts this module encodes (verified against the official
binance-public-data README):

- Base ``https://data.binance.vision/data/``; every archive has a sibling
  ``<url>.CHECKSUM`` containing its SHA-256.
- ``spot`` offers aggTrades/trades/klines only — **no book data at all**.
- ``futures/um`` adds bookTicker (tick-level L1) and bookDepth (sampled
  ±1..5% depth bands).
- Spot timestamps switch from milliseconds to microseconds at 2025-01-01;
  normalization sniffs units by magnitude rather than trusting dates.
"""

from __future__ import annotations

from dataclasses import dataclass

BASE = "https://data.binance.vision/data"

MARKETS = {"spot": "spot", "um": "futures/um", "cm": "futures/cm"}

# dataset name -> available in spot?
DATASETS = {
    "aggTrades": True,
    "trades": True,
    "klines": True,
    "bookTicker": False,
    "bookDepth": False,
}


@dataclass(frozen=True)
class Archive:
    market: str
    dataset: str
    symbol: str
    date: str  # YYYY-MM-DD (daily) or YYYY-MM (monthly)
    monthly: bool = False
    interval: str = "1m"  # klines only

    def url(self) -> str:
        market_path = MARKETS[self.market]
        period = "monthly" if self.monthly else "daily"
        if self.dataset == "klines":
            return (
                f"{BASE}/{market_path}/{period}/klines/{self.symbol}/{self.interval}/"
                f"{self.symbol}-{self.interval}-{self.date}.zip"
            )
        return (
            f"{BASE}/{market_path}/{period}/{self.dataset}/{self.symbol}/"
            f"{self.symbol}-{self.dataset}-{self.date}.zip"
        )

    def checksum_url(self) -> str:
        return self.url() + ".CHECKSUM"

    def validate(self) -> None:
        if self.market not in MARKETS:
            msg = f"unknown market {self.market!r}"
            raise ValueError(msg)
        if self.dataset not in DATASETS:
            msg = f"unknown dataset {self.dataset!r}"
            raise ValueError(msg)
        if self.market == "spot" and not DATASETS[self.dataset]:
            msg = f"{self.dataset} does not exist for spot (no book data on spot)"
            raise ValueError(msg)


# Raw CSV columns per (market, dataset). Futures files carry a header row;
# spot files historically do not — the normalizer sniffs either way.
RAW_COLUMNS = {
    ("um", "aggTrades"): [
        "agg_trade_id",
        "price",
        "quantity",
        "first_trade_id",
        "last_trade_id",
        "transact_time",
        "is_buyer_maker",
    ],
    ("spot", "aggTrades"): [
        "agg_trade_id",
        "price",
        "quantity",
        "first_trade_id",
        "last_trade_id",
        "transact_time",
        "is_buyer_maker",
        "is_best_match",
    ],
    ("um", "bookTicker"): [
        "update_id",
        "best_bid_price",
        "best_bid_qty",
        "best_ask_price",
        "best_ask_qty",
        "transaction_time",
        "event_time",
    ],
    ("um", "bookDepth"): ["timestamp", "percentage", "depth", "notional"],
}
