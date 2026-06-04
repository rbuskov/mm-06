"""CLI: ``python -m analysis <wav> [--features ...]`` (and the ``analyze`` script).

Prints the metrics JSON to stdout so it can be piped into the harness / jq.
"""

from __future__ import annotations

import argparse
import json
import sys

from .core import analyze
from .schema import ALL_FEATURES


def _parse_features(arg):
    if not arg:
        return None
    names = []
    for chunk in arg:
        names.extend(p.strip() for p in chunk.split(",") if p.strip())
    return names or None


def main(argv=None) -> int:
    p = argparse.ArgumentParser(
        prog="analyze",
        description="Analyze an MM-06 render WAV and print a metrics JSON.",
    )
    p.add_argument("wav", help="path to a mono float WAV (e.g. from crates/render)")
    p.add_argument(
        "--features",
        "-f",
        action="append",
        metavar="LIST",
        help=(
            "comma-separated feature names to compute (default: all). "
            "Valid: " + ", ".join(ALL_FEATURES) + ", all"
        ),
    )
    p.add_argument(
        "--indent",
        type=int,
        default=2,
        help="JSON indent (use a negative value for compact single-line output)",
    )
    args = p.parse_args(argv)

    features = _parse_features(args.features)
    try:
        metrics = analyze(args.wav, feature_set=features)
    except Exception as e:  # noqa: BLE001 - surface a clean CLI error
        print(f"analyze: error: {e}", file=sys.stderr)
        return 1

    indent = None if args.indent is not None and args.indent < 0 else args.indent
    json.dump(metrics, sys.stdout, indent=indent)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
