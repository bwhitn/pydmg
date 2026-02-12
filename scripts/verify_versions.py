#!/usr/bin/env python3
"""Validate release versions across project metadata files."""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path


def read_pyproject_version(path: Path) -> str:
    with path.open("rb") as handle:
        data = tomllib.load(handle)
    try:
        return str(data["project"]["version"])
    except KeyError as exc:
        raise RuntimeError(f"missing project.version in {path}") from exc


def read_cargo_version(path: Path) -> str:
    with path.open("rb") as handle:
        data = tomllib.load(handle)
    try:
        return str(data["package"]["version"])
    except KeyError as exc:
        raise RuntimeError(f"missing package.version in {path}") from exc


def normalize(version: str) -> str:
    return version.strip().removeprefix("v")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--expected",
        default="",
        help="Expected release version, with or without leading v",
    )
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    pyproject = root / "pyproject.toml"
    cargo = root / "Cargo.toml"

    py_version = normalize(read_pyproject_version(pyproject))
    cargo_version = normalize(read_cargo_version(cargo))

    if py_version != cargo_version:
        print(
            f"version mismatch: pyproject={py_version} cargo={cargo_version}",
            file=sys.stderr,
        )
        return 1

    expected = normalize(args.expected)
    if expected and py_version != expected:
        print(
            f"tag/version mismatch: expected={expected} project={py_version}",
            file=sys.stderr,
        )
        return 1

    print(f"version check OK: {py_version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
