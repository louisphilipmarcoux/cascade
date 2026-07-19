# Committed data fixtures

Slices of Binance USD-M BTCUSDT for 2024-01-07 (hour 00), cut deterministically from the pinned raw
archives below. Rebuild with `uv run quantsim-data fixture`.

- trades: 41473 rows (1 hour of aggTrades)
- quotes: 73698 rows (first 10 min of bookTicker L1)

Source archives (see also data/manifest.toml):

- `a326f1f0c4833b35…` https://data.binance.vision/data/futures/um/daily/aggTrades/BTCUSDT/BTCUSDT-aggTrades-2024-01-07.zip
- `4536005a9fa21233…` https://data.binance.vision/data/futures/um/daily/bookTicker/BTCUSDT/BTCUSDT-bookTicker-2024-01-07.zip
