"""Repository paths and pinned study configuration."""

from __future__ import annotations

import os
from pathlib import Path


def repo_root() -> Path:
    """Locate the repo root (env override, else walk up to `data/`'s parent)."""
    if env := os.environ.get("QUANTSIM_ROOT"):
        return Path(env)
    here = Path(__file__).resolve()
    for parent in here.parents:
        if (parent / "Cargo.toml").exists() and (parent / "docs").exists():
            return parent
    msg = "cannot locate quant-sim repo root; set QUANTSIM_ROOT"
    raise RuntimeError(msg)


def data_dir() -> Path:
    return repo_root() / "data"


def raw_dir() -> Path:
    return data_dir() / "raw"


def normalized_dir() -> Path:
    return data_dir() / "normalized"


def fixtures_dir() -> Path:
    return data_dir() / "fixtures"


def recorded_dir() -> Path:
    return data_dir() / "recorded"


def manifest_path() -> Path:
    return data_dir() / "manifest.toml"


# The pinned study window (spec: docs/interchange.md provenance discipline).
#
# January 2024, not later: Binance Vision's um bookTicker archives — the only
# free tick-level L1 source — were published from ~mid-2023 and discontinued
# between 2024-03 and 2024-04 (verified empirically by HTTP probing; see
# docs/interchange.md). The study window must lie where trades, bookTicker
# and bookDepth all coexist.
STUDY_SYMBOL = "BTCUSDT"
STUDY_MARKET = "um"  # Binance USD-M perpetual
STUDY_START = "2024-01-01"
STUDY_END = "2024-01-31"
# The committed fixture day (low-volume Sunday) and hour.
FIXTURE_DATE = "2024-01-07"
FIXTURE_HOUR = 0
