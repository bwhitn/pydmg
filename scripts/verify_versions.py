#!/usr/bin/env python3
"""Validate release versions and the repository's Rust toolchain policy."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python 3.9/3.10 compatibility
    import tomli as tomllib


PRIMARY_RUST = "1.98.1"
MSRV_RUST = "1.88"


def read_pyproject_version(path: Path) -> str:
    with path.open("rb") as handle:
        data = tomllib.load(handle)
    try:
        return str(data["project"]["version"])
    except KeyError as exc:
        raise RuntimeError(f"missing project.version in {path}") from exc


def read_cargo_package(path: Path) -> tuple[str, str]:
    with path.open("rb") as handle:
        data = tomllib.load(handle)
    try:
        return str(data["package"]["version"]), str(data["package"]["rust-version"])
    except KeyError as exc:
        raise RuntimeError(f"missing package version or rust-version in {path}") from exc


def read_primary_rust(path: Path) -> str:
    with path.open("rb") as handle:
        data = tomllib.load(handle)
    try:
        return str(data["toolchain"]["channel"])
    except KeyError as exc:
        raise RuntimeError(f"missing toolchain.channel in {path}") from exc


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
    rust_toolchain = root / "rust-toolchain.toml"

    py_version = normalize(read_pyproject_version(pyproject))
    cargo_version_raw, msrv = read_cargo_package(cargo)
    cargo_version = normalize(cargo_version_raw)
    primary_rust = read_primary_rust(rust_toolchain)

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

    if primary_rust != PRIMARY_RUST:
        print(
            f"primary Rust mismatch: expected={PRIMARY_RUST} configured={primary_rust}",
            file=sys.stderr,
        )
        return 1

    if msrv != MSRV_RUST:
        print(
            f"Rust MSRV mismatch: expected={MSRV_RUST} configured={msrv}",
            file=sys.stderr,
        )
        return 1

    print(f"version check OK: {py_version}; Rust {primary_rust}; MSRV {msrv}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
