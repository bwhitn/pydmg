#!/usr/bin/env python3
"""Validate that Cargo audit ignores are documented, exact, and unexpired."""

from __future__ import annotations

import json
import sys
from datetime import date
from pathlib import Path
from typing import Any, cast

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python 3.9/3.10 compatibility
    import tomli as tomllib


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    with (root / ".cargo" / "audit.toml").open("rb") as handle:
        audit_config = tomllib.load(handle)
    exceptions = cast(
        list[dict[str, Any]],
        json.loads((root / "security" / "audit-exceptions.json").read_text(encoding="utf-8")),
    )

    ignored = set(cast(list[str], audit_config.get("advisories", {}).get("ignore", [])))
    documented_ids = [cast(str, item.get("id")) for item in exceptions]
    documented = set(documented_ids)
    failures: list[str] = []

    if len(documented) != len(documented_ids):
        failures.append("documented advisory IDs must be unique")

    if ignored != documented:
        failures.append(
            f"audit ignore IDs and documented exception IDs differ: ignored={sorted(ignored)}, "
            f"documented={sorted(documented)}"
        )

    today = date.today()
    required_fields = {"id", "package", "dependency_path", "reason", "expires", "tracking"}
    for item in exceptions:
        advisory_id = cast(str, item.get("id", "<missing>"))
        missing = sorted(required_fields - item.keys())
        if missing:
            failures.append(f"{advisory_id}: missing fields {missing}")
            continue
        empty = sorted(
            field
            for field in required_fields
            if not isinstance(item[field], str) or not cast(str, item[field]).strip()
        )
        if empty:
            failures.append(f"{advisory_id}: fields must be non-empty strings: {empty}")
            continue
        try:
            expiry = date.fromisoformat(cast(str, item["expires"]))
        except (TypeError, ValueError):
            failures.append(f"{advisory_id}: invalid ISO expiry date {item['expires']!r}")
            continue
        if expiry < today:
            failures.append(f"{advisory_id}: exception expired on {expiry.isoformat()}")

    if failures:
        print("Cargo audit exception validation failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(f"Cargo audit exceptions OK ({len(exceptions)} documented, none expired)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
