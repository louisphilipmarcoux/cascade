"""`quantsim-data`: download / verify / normalize / fixture / record."""

from __future__ import annotations

import argparse
import sys

from quantsim_research.config import STUDY_MARKET, STUDY_SYMBOL


def main() -> None:
    parser = argparse.ArgumentParser(prog="quantsim-data")
    sub = parser.add_subparsers(dest="command", required=True)

    p_download = sub.add_parser("download", help="fetch + checksum-pin Binance Vision archives")
    p_download.add_argument("--market", default=STUDY_MARKET, choices=["spot", "um"])
    p_download.add_argument("--dataset", required=True)
    p_download.add_argument("--symbol", default=STUDY_SYMBOL)
    p_download.add_argument("--start", required=True, help="YYYY-MM-DD")
    p_download.add_argument("--end", required=True, help="YYYY-MM-DD (inclusive)")

    sub.add_parser("verify", help="re-hash raw archives against data/manifest.toml")

    p_norm = sub.add_parser("normalize", help="raw zips → normalized parquet + rust csv")
    p_norm.add_argument("--market", default=STUDY_MARKET, choices=["spot", "um"])
    p_norm.add_argument("--dataset", required=True, choices=["aggTrades", "bookTicker"])
    p_norm.add_argument("--symbol", default=STUDY_SYMBOL)
    p_norm.add_argument("--start", required=True)
    p_norm.add_argument("--end", required=True)

    sub.add_parser("fixture", help="rebuild the committed data/fixtures deterministically")

    p_record = sub.add_parser("record", help="live L2 depth-diff + trades recorder (runs forever)")
    p_record.add_argument("--symbol", default=STUDY_SYMBOL)

    args = parser.parse_args()

    if args.command == "download":
        from quantsim_research.data.binance_vision import Archive
        from quantsim_research.data.download import date_range, fetch

        for date in date_range(args.start, args.end):
            fetch(Archive(args.market, args.dataset, args.symbol, date))
    elif args.command == "verify":
        from quantsim_research.data import manifest

        errors = manifest.verify()
        for error in errors:
            print(f"MISMATCH {error}", file=sys.stderr)
        if errors:
            sys.exit(1)
        print(f"ok: {len(manifest.load())} pinned archives verified")
    elif args.command == "normalize":
        from quantsim_research.data import normalize as norm
        from quantsim_research.data.binance_vision import Archive
        from quantsim_research.data.download import date_range, fetch

        for date in date_range(args.start, args.end):
            archive = Archive(args.market, args.dataset, args.symbol, date)
            zip_path = fetch(archive)
            if args.dataset == "aggTrades":
                frame = norm.normalize_agg_trades(zip_path, archive)
                out = norm.write_partition(frame, "trades", archive)
            else:
                frame = norm.normalize_book_ticker(zip_path, archive)
                out = norm.write_partition(frame, "quotes", archive)
            print(f"{date}: {len(frame)} rows → {out}")
    elif args.command == "fixture":
        from quantsim_research.data import fixtures

        fixtures.build()
        print("fixtures rebuilt")
    elif args.command == "record":
        from quantsim_research.data import recorder

        recorder.main(args.symbol)
