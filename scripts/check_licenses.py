#!/usr/bin/env python3
"""Validate Rust dependency licenses against a permissive allowlist."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional, Tuple


ALLOWED_LICENSE_IDS = {
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "0BSD",
    "ISC",
    "Unlicense",
    "Zlib",
    "Unicode-3.0",
}

ALLOWED_EXCEPTIONS = {
    "LLVM-exception",
}

EXPRESSION_NORMALIZATION = {
    "MIT/Apache-2.0": "MIT OR Apache-2.0",
    "Apache-2.0/MIT": "Apache-2.0 OR MIT",
    "MIT/Apache-2.0 OR Apache-2.0/MIT": "MIT OR Apache-2.0",
}

TOKEN_RE = re.compile(r"\(|\)|AND|OR|WITH|[A-Za-z0-9.+-]+")


@dataclass
class LicenseNode:
    kind: str
    value: Optional[str] = None
    left: Optional["LicenseNode"] = None
    right: Optional["LicenseNode"] = None


def normalize_expression(expr: str) -> str:
    normalized = expr.strip()
    for original, replacement in EXPRESSION_NORMALIZATION.items():
        normalized = normalized.replace(original, replacement)
    return normalized


def tokenize(expr: str) -> List[str]:
    return TOKEN_RE.findall(expr)


class Parser:
    def __init__(self, tokens: List[str]) -> None:
        self.tokens = tokens
        self.index = 0

    def parse(self) -> LicenseNode:
        node = self.parse_or()
        if self.index != len(self.tokens):
            raise ValueError(f"unexpected token: {self.tokens[self.index]}")
        return node

    def parse_or(self) -> LicenseNode:
        node = self.parse_and()
        while self.peek() == "OR":
            self.index += 1
            node = LicenseNode("OR", left=node, right=self.parse_and())
        return node

    def parse_and(self) -> LicenseNode:
        node = self.parse_with()
        while self.peek() == "AND":
            self.index += 1
            node = LicenseNode("AND", left=node, right=self.parse_with())
        return node

    def parse_with(self) -> LicenseNode:
        node = self.parse_primary()
        if self.peek() == "WITH":
            self.index += 1
            exception = self.consume_identifier("exception")
            return LicenseNode("WITH", value=exception, left=node)
        return node

    def parse_primary(self) -> LicenseNode:
        token = self.peek()
        if token is None:
            raise ValueError("unexpected end of expression")
        if token == "(":
            self.index += 1
            node = self.parse_or()
            if self.peek() != ")":
                raise ValueError("missing closing ')'")
            self.index += 1
            return node
        if token in {"AND", "OR", "WITH", ")"}:
            raise ValueError(f"unexpected token: {token}")
        self.index += 1
        return LicenseNode("ID", value=token)

    def consume_identifier(self, label: str) -> str:
        token = self.peek()
        if token is None or token in {"AND", "OR", "WITH", "(", ")"}:
            raise ValueError(f"expected {label}")
        self.index += 1
        return token

    def peek(self) -> Optional[str]:
        if self.index >= len(self.tokens):
            return None
        return self.tokens[self.index]


def expression_is_allowed(node: LicenseNode) -> bool:
    if node.kind == "ID":
        return bool(node.value in ALLOWED_LICENSE_IDS)
    if node.kind == "WITH":
        if not node.left or not expression_is_allowed(node.left):
            return False
        return bool(node.value in ALLOWED_EXCEPTIONS)
    if node.kind == "AND":
        return bool(node.left and node.right and expression_is_allowed(node.left)
                    and expression_is_allowed(node.right))
    if node.kind == "OR":
        return bool(node.left and node.right and
                    (expression_is_allowed(node.left) or expression_is_allowed(node.right)))
    return False


def load_metadata(root: Path) -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version=1", "--locked"],
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        stderr = result.stderr.strip() or "unknown error"
        raise RuntimeError(f"cargo metadata failed: {stderr}")
    return json.loads(result.stdout)


def evaluate_license(expr: str) -> Tuple[bool, str]:
    normalized = normalize_expression(expr)
    tokens = tokenize(normalized)
    if not tokens:
        return False, "empty license expression"
    try:
        tree = Parser(tokens).parse()
    except ValueError as exc:
        return False, f"unable to parse SPDX expression '{normalized}': {exc}"
    if expression_is_allowed(tree):
        return True, ""
    return (
        False,
        "no allowed permissive license path in expression "
        f"'{normalized}' (allowed IDs: {sorted(ALLOWED_LICENSE_IDS)})",
    )


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    metadata = load_metadata(root)
    failures: List[str] = []
    checked = 0

    for pkg in metadata.get("packages", []):
        if pkg.get("source") is None:
            continue
        checked += 1
        name = pkg.get("name", "<unknown>")
        version = pkg.get("version", "<unknown>")
        expr = pkg.get("license")
        if not expr:
            failures.append(f"{name} {version}: missing license expression")
            continue
        ok, reason = evaluate_license(expr)
        if not ok:
            failures.append(f"{name} {version}: {reason}")

    if failures:
        print("license policy check failed for Rust dependencies:", file=sys.stderr)
        for item in failures:
            print(f"- {item}", file=sys.stderr)
        return 1

    print(f"license policy check OK ({checked} third-party Rust crates checked)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
