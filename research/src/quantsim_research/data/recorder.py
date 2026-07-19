"""Live L2/trades recorder (forward-looking capture).

Full historical L2 depth is not freely downloadable (see
docs/interchange.md), so this recorder captures Binance USD-M combined
streams — ``<symbol>@depth@100ms`` diffs plus ``<symbol>@aggTrade`` —
into hourly-rotated, line-delimited JSON (gzip) under
``data/recorded/<symbol>/<date>/``.

Run it early, keep it running: Stage 3's exact L2 delta replay and
queue-position models feed on what this accumulates. On restart it simply
opens a new file; gaps are visible as missing `u`/`pu` continuity when
replayed, and the replay builder refuses to bridge them silently.
"""

from __future__ import annotations

import asyncio
import datetime as dt
import gzip
import json
import signal
import sys

import websockets

from quantsim_research.config import recorded_dir

STREAM_URL = "wss://fstream.binance.com/stream?streams={symbol}@depth@100ms/{symbol}@aggTrade"


def _out_path(symbol: str, now: dt.datetime):
    directory = recorded_dir() / symbol.upper() / now.strftime("%Y-%m-%d")
    directory.mkdir(parents=True, exist_ok=True)
    return directory / f"{symbol.upper()}-{now.strftime('%Y%m%dT%H')}.jsonl.gz"


async def record(symbol: str) -> None:
    symbol = symbol.lower()
    url = STREAM_URL.format(symbol=symbol)
    current_hour: str | None = None
    sink = None
    messages = 0
    try:
        async for connection in websockets.connect(url, ping_interval=20, max_size=2**22):
            try:
                async for message in connection:
                    now = dt.datetime.now(dt.UTC)
                    hour = now.strftime("%Y%m%dT%H")
                    if hour != current_hour:
                        if sink is not None:
                            sink.close()
                        # Deliberately not a context manager: the sink outlives
                        # this scope and rotates hourly; closed in `finally`.
                        sink = gzip.open(  # noqa: SIM115
                            _out_path(symbol, now), "at", encoding="utf-8"
                        )
                        current_hour = hour
                        print(f"[recorder] {symbol} → {hour} ({messages} msgs so far)", flush=True)
                    # Wrap with our receive timestamp; payload stays verbatim.
                    sink.write(
                        json.dumps(
                            {"recv_ns": int(now.timestamp() * 1e9), "msg": json.loads(message)},
                            separators=(",", ":"),
                        )
                    )
                    sink.write("\n")
                    messages += 1
            except websockets.ConnectionClosed:
                print("[recorder] reconnecting…", flush=True)
                continue
    finally:
        if sink is not None:
            sink.close()


def main(symbol: str) -> None:
    def _stop(*_args) -> None:
        print(f"[recorder] stopping {symbol}", flush=True)
        sys.exit(0)

    signal.signal(signal.SIGINT, _stop)
    signal.signal(signal.SIGTERM, _stop)
    asyncio.run(record(symbol))
