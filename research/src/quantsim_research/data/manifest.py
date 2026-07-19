"""The committed data manifest: SHA-256 pins for every downloaded archive.

Re-downloading a pinned URL that hashes differently is a **hard error** —
the upstream-mutation tripwire. `verify()` re-hashes everything on disk
against the manifest and is CI-usable.
"""

from __future__ import annotations

import hashlib
import tomllib
from dataclasses import asdict, dataclass
from pathlib import Path

import tomli_w

from quantsim_research.config import manifest_path, raw_dir


@dataclass
class Entry:
    url: str
    path: str  # relative to data/raw
    sha256: str
    size_bytes: int
    binance_checksum_verified: bool
    downloaded_at: str  # UTC ISO-8601


class PinConflict(RuntimeError):
    """A pinned URL re-downloaded with a different hash."""


def load() -> dict[str, Entry]:
    path = manifest_path()
    if not path.exists():
        return {}
    with path.open("rb") as fh:
        raw = tomllib.load(fh)
    return {e["url"]: Entry(**e) for e in raw.get("archive", [])}


def save(entries: dict[str, Entry]) -> None:
    path = manifest_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {"archive": [asdict(entries[url]) for url in sorted(entries)]}
    text = tomli_w.dumps(payload)
    path.write_text(text, encoding="utf-8", newline="\n")


def record(entry: Entry) -> None:
    entries = load()
    existing = entries.get(entry.url)
    if existing is not None and existing.sha256 != entry.sha256:
        msg = (
            f"pin conflict for {entry.url}:\n"
            f"  manifest: {existing.sha256}\n"
            f"  download: {entry.sha256}\n"
            "Upstream archive changed or download corrupted; refusing to overwrite."
        )
        raise PinConflict(msg)
    entries[entry.url] = entry
    save(entries)


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        while chunk := fh.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


def verify() -> list[str]:
    """Re-hash every manifest entry present on disk. Returns error strings."""
    errors: list[str] = []
    for entry in load().values():
        path = raw_dir() / entry.path
        if not path.exists():
            continue  # payloads are gitignored; absence is fine
        actual = sha256_of(path)
        if actual != entry.sha256:
            errors.append(f"{entry.path}: manifest {entry.sha256[:16]}… != disk {actual[:16]}…")
    return errors
