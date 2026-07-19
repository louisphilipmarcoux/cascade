"""Manifest pinning: round-trip, conflict tripwire, verify."""

import pytest

from quantsim_research.data import manifest


@pytest.fixture
def isolated_repo(tmp_path, monkeypatch):
    (tmp_path / "docs").mkdir()
    (tmp_path / "Cargo.toml").write_text("[workspace]\n")
    monkeypatch.setenv("QUANTSIM_ROOT", str(tmp_path))
    return tmp_path


def entry(url: str, sha: str) -> manifest.Entry:
    return manifest.Entry(
        url=url,
        path="x/y.zip",
        sha256=sha,
        size_bytes=1,
        binance_checksum_verified=True,
        downloaded_at="2026-07-18T00:00:00+00:00",
    )


def test_round_trip_and_conflict(isolated_repo):
    manifest.record(entry("https://e/a.zip", "aa" * 32))
    manifest.record(entry("https://e/b.zip", "bb" * 32))
    loaded = manifest.load()
    assert set(loaded) == {"https://e/a.zip", "https://e/b.zip"}

    # Same URL, same hash: idempotent.
    manifest.record(entry("https://e/a.zip", "aa" * 32))
    # Same URL, different hash: hard error.
    with pytest.raises(manifest.PinConflict):
        manifest.record(entry("https://e/a.zip", "cc" * 32))


def test_verify_detects_corruption(isolated_repo):
    payload = isolated_repo / "data" / "raw" / "x" / "y.zip"
    payload.parent.mkdir(parents=True)
    payload.write_bytes(b"hello")
    good = manifest.sha256_of(payload)
    manifest.record(entry("https://e/y.zip", good))
    assert manifest.verify() == []
    payload.write_bytes(b"tampered")
    errors = manifest.verify()
    assert len(errors) == 1 and "y.zip" in errors[0]
