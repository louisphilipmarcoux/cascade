"""Checksum-verified downloads from Binance Vision."""

from __future__ import annotations

import datetime as dt
import hashlib
from pathlib import Path

import httpx
from tqdm import tqdm

from quantsim_research.config import raw_dir
from quantsim_research.data import manifest
from quantsim_research.data.binance_vision import Archive


def _relative_path(archive: Archive) -> str:
    # Mirror the remote path under data/raw for provenance-by-inspection.
    return archive.url().removeprefix("https://data.binance.vision/data/")


def fetch(archive: Archive, *, client: httpx.Client | None = None) -> Path:
    """Download one archive (idempotent), verify Binance's CHECKSUM, pin it."""
    archive.validate()
    own_client = client is None
    client = client or httpx.Client(timeout=60.0, follow_redirects=True)
    try:
        relative = _relative_path(archive)
        target = raw_dir() / relative
        target.parent.mkdir(parents=True, exist_ok=True)

        # Binance's published SHA-256 sidecar.
        checksum_text = client.get(archive.checksum_url()).raise_for_status().text
        expected = checksum_text.split()[0].strip().lower()

        if target.exists() and manifest.sha256_of(target) == expected:
            return target  # already present and valid

        digest = hashlib.sha256()
        tmp = target.with_suffix(".part")
        with client.stream("GET", archive.url()) as response:
            response.raise_for_status()
            total = int(response.headers.get("content-length", 0))
            with (
                tmp.open("wb") as out,
                tqdm(total=total, unit="B", unit_scale=True, desc=target.name) as bar,
            ):
                for chunk in response.iter_bytes(1 << 20):
                    out.write(chunk)
                    digest.update(chunk)
                    bar.update(len(chunk))
        actual = digest.hexdigest()
        if actual != expected:
            tmp.unlink(missing_ok=True)
            msg = f"checksum mismatch for {archive.url()}: {actual} != {expected}"
            raise RuntimeError(msg)
        tmp.replace(target)

        manifest.record(
            manifest.Entry(
                url=archive.url(),
                path=relative.replace("\\", "/"),
                sha256=actual,
                size_bytes=target.stat().st_size,
                binance_checksum_verified=True,
                downloaded_at=dt.datetime.now(dt.UTC).isoformat(timespec="seconds"),
            )
        )
        return target
    finally:
        if own_client:
            client.close()


def date_range(start: str, end: str) -> list[str]:
    """Inclusive list of YYYY-MM-DD dates."""
    first = dt.date.fromisoformat(start)
    last = dt.date.fromisoformat(end)
    out = []
    day = first
    while day <= last:
        out.append(day.isoformat())
        day += dt.timedelta(days=1)
    return out
