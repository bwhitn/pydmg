#!/usr/bin/env python3
"""Build pydoc HTML docs for pydmg."""

from __future__ import annotations

import argparse
import os
import pydoc
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output-dir",
        default=".",
        help="Directory where pydoc HTML will be written (default: current directory)",
    )
    parser.add_argument(
        "--cleanup",
        action="store_true",
        help="Delete the generated HTML after verifying it was created",
    )
    args = parser.parse_args()

    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    output_filename = "pydmg.html"
    old_cwd = Path.cwd()
    try:
        os.chdir(output_dir)
        pydoc.writedoc("pydmg")
    finally:
        os.chdir(old_cwd)

    output_path = output_dir / output_filename
    if not output_path.exists():
        raise RuntimeError(f"pydoc output was not created: {output_path}")

    print(f"pydoc build OK: {output_path}")

    if args.cleanup:
        output_path.unlink()
        print(f"pydoc cleanup OK: {output_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
