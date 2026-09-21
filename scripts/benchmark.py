#!/usr/bin/env python3
"""Run repeatable release-mode pydmg performance measurements.

The parent process starts a fresh worker for every sample so peak resident memory
and process I/O are isolated. Build the extension with Cargo's ``benchmarking``
feature to include Rust allocation and partition-copy counters.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib
import importlib.metadata
import json
import math
import platform
import shutil
import statistics
import subprocess  # nosec B404
import sys
import tempfile
import time
import zipfile
from collections.abc import Sequence
from pathlib import Path
from typing import Any, Callable, cast

# Subprocesses use resolved executables and argument arrays without a shell.

try:
    import resource
except ImportError:  # pragma: no cover - Windows has no resource module.
    resource = None  # type: ignore[assignment]


ROOT = Path(__file__).resolve().parents[1]
FAT_FIXTURE = ROOT / "tests" / "fixtures" / "fat32-large-sample.dmg"
APFS_FIXTURE = ROOT / "tests" / "fixtures" / "apfs-linearmouse-v0.10.2.dmg"
APPLE_FILE = "/LinearMouse.app/Contents/Info.plist"
APPLE_DIRECTORY = "/LinearMouse.app/Contents"
SCENARIOS = (
    "import",
    "metadata",
    "checksum",
    "compressed_partition",
    "partition_extract",
    "fat_list",
    "fat_extract",
    "apple_list",
    "apple_extract",
)
METRIC_NAMES = (
    "wall_seconds",
    "cpu_seconds",
    "peak_rss_bytes",
    "disk_read_bytes",
    "disk_write_bytes",
    "logical_output_bytes",
    "allocation_calls",
    "allocated_bytes",
    "reallocation_calls",
    "reallocated_bytes",
    "staging_copy_bytes",
    "compressed_input_bytes",
    "decoded_output_bytes",
    "sink_written_bytes",
)


def _run_git(*arguments: str) -> str:
    executable = shutil.which("git")
    if executable is None:
        raise RuntimeError("git is required to identify the benchmark revision")
    # Callers supply only fixed repository-inspection arguments.
    completed = subprocess.run(  # nosec B603
        [executable, *arguments],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return completed.stdout.strip()


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _usage() -> Any:
    if resource is None:
        return None
    return resource.getrusage(resource.RUSAGE_SELF)


def _peak_rss_bytes(usage: Any) -> int | None:
    if usage is None:
        return None
    value = int(usage.ru_maxrss)
    # Darwin reports bytes; Linux and the BSDs report KiB.
    return value if sys.platform == "darwin" else value * 1024


def _block_bytes(after: Any, before: Any, field: str) -> int | None:
    if after is None or before is None:
        return None
    blocks = max(0, int(getattr(after, field)) - int(getattr(before, field)))
    # ru_inblock/ru_oublock count 512-byte blocks on supported Unix hosts.
    return blocks * 512


def _native_metrics_reset(native: Any) -> None:
    reset = getattr(native, "_benchmark_metrics_reset", None)
    if reset is not None:
        reset()


def _native_metrics(native: Any) -> dict[str, int]:
    snapshot = getattr(native, "_benchmark_metrics_json", None)
    if snapshot is None:
        return {}
    value = json.loads(snapshot())
    if not isinstance(value, dict):
        raise TypeError("native benchmark metrics must be a JSON object")
    return {str(key): int(item) for key, item in value.items()}


def _directory_file_bytes(path: Path) -> int:
    return sum(item.stat().st_size for item in path.rglob("*") if item.is_file())


def _scenario_operation(name: str, scratch: Path) -> tuple[Callable[[], Any], Callable[[Any], int]]:
    import pydmg

    if name == "metadata":
        return (
            lambda: pydmg.list_partitions(FAT_FIXTURE),
            lambda value: len(json.dumps(value, sort_keys=True, separators=(",", ":"))),
        )
    if name == "checksum":
        return (lambda: pydmg.compute_data_checksum(APFS_FIXTURE), lambda _value: 4)
    if name == "compressed_partition":
        return (lambda: pydmg.read_partition(APFS_FIXTURE, 4), len)
    if name == "partition_extract":
        destination = scratch / "partition.bin"
        return (
            lambda: pydmg.extract_partition(APFS_FIXTURE, 4, destination),
            lambda _value: destination.stat().st_size,
        )
    if name == "fat_list":
        return (
            lambda: pydmg.list_fat32_entries(FAT_FIXTURE, 1),
            lambda value: len(json.dumps(value, sort_keys=True, separators=(",", ":"))),
        )
    if name == "fat_extract":
        destination = scratch / "fat"
        return (
            lambda: pydmg.extract_fat32(FAT_FIXTURE, destination, 1),
            lambda _value: _directory_file_bytes(destination),
        )
    if name == "apple_list":
        return (
            lambda: pydmg.list_apple_entries(APFS_FIXTURE, APPLE_DIRECTORY),
            lambda value: len(json.dumps(value, sort_keys=True, separators=(",", ":"))),
        )
    if name == "apple_extract":
        destination = scratch / "Info.plist"
        return (
            lambda: pydmg.extract_apple_file(APFS_FIXTURE, APPLE_FILE, destination),
            lambda _value: destination.stat().st_size,
        )
    raise ValueError(f"unknown scenario: {name}")


def _run_import_worker() -> dict[str, Any]:
    usage_before = _usage()
    cpu_before = time.process_time()
    wall_before = time.perf_counter()
    module = importlib.import_module("pydmg")
    wall_seconds = time.perf_counter() - wall_before
    cpu_seconds = time.process_time() - cpu_before
    usage_after = _usage()
    native_path = Path(cast(str, importlib.import_module("pydmg._pydmg").__file__))
    return {
        "scenario": "import",
        "wall_seconds": wall_seconds,
        "cpu_seconds": cpu_seconds,
        "peak_rss_bytes": _peak_rss_bytes(usage_after),
        "disk_read_bytes": _block_bytes(usage_after, usage_before, "ru_inblock"),
        "disk_write_bytes": _block_bytes(usage_after, usage_before, "ru_oublock"),
        "logical_output_bytes": native_path.stat().st_size,
        "module_version": module.__version__,
    }


def _run_operation_worker(name: str) -> dict[str, Any]:
    import pydmg  # noqa: F401 - importing before timing isolates operation cost.
    from pydmg import _pydmg as native

    with tempfile.TemporaryDirectory(prefix=f"pydmg-benchmark-{name}-") as temporary:
        operation, output_size = _scenario_operation(name, Path(temporary))
        _native_metrics_reset(native)
        usage_before = _usage()
        cpu_before = time.process_time()
        wall_before = time.perf_counter()
        result = operation()
        wall_seconds = time.perf_counter() - wall_before
        cpu_seconds = time.process_time() - cpu_before
        usage_after = _usage()
        native_metrics = _native_metrics(native)
        logical_output_bytes = output_size(result)

    sample: dict[str, Any] = {
        "scenario": name,
        "wall_seconds": wall_seconds,
        "cpu_seconds": cpu_seconds,
        "peak_rss_bytes": _peak_rss_bytes(usage_after),
        "disk_read_bytes": _block_bytes(usage_after, usage_before, "ru_inblock"),
        "disk_write_bytes": _block_bytes(usage_after, usage_before, "ru_oublock"),
        "logical_output_bytes": logical_output_bytes,
    }
    sample.update(native_metrics)
    return sample


def _run_worker(name: str) -> int:
    result = _run_import_worker() if name == "import" else _run_operation_worker(name)
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


def _percentile_95(values: Sequence[float]) -> float:
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * 0.95) - 1)]


def _summarize(samples: Sequence[dict[str, Any]]) -> dict[str, dict[str, float]]:
    summary: dict[str, dict[str, float]] = {}
    for metric in METRIC_NAMES:
        values = [float(sample[metric]) for sample in samples if sample.get(metric) is not None]
        if not values:
            continue
        summary[metric] = {
            "min": min(values),
            "median": statistics.median(values),
            "mean": statistics.fmean(values),
            "stdev": statistics.stdev(values) if len(values) > 1 else 0.0,
            "p95": _percentile_95(values),
            "max": max(values),
        }
    return summary


def _run_sample(name: str) -> dict[str, Any]:
    # The interpreter, script, and scenario choice are controlled locally.
    completed = subprocess.run(  # nosec B603
        [sys.executable, str(Path(__file__).resolve()), "--worker", name],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"benchmark worker {name!r} failed with code {completed.returncode}:\n"
            f"{completed.stderr}"
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"benchmark worker {name!r} emitted invalid JSON") from error
    if not isinstance(value, dict):
        raise TypeError(f"benchmark worker {name!r} result must be an object")
    return cast(dict[str, Any], value)


def _artifact_size(path: Path | None) -> int | None:
    if path is None:
        return None
    return path.stat().st_size


def _wheel_native_size(path: Path | None) -> int | None:
    if path is None:
        return None
    with zipfile.ZipFile(path) as archive:
        candidates = [
            item.file_size
            for item in archive.infolist()
            if "_pydmg" in item.filename
            and item.filename.endswith((".so", ".pyd", ".dylib"))
        ]
    if len(candidates) != 1:
        raise RuntimeError(f"expected one native pydmg library in wheel, found {len(candidates)}")
    return candidates[0]


def _command_version(command: Sequence[str]) -> str | None:
    executable = shutil.which(command[0])
    if executable is None:
        return None
    # Version probes use a resolved local executable and fixed arguments.
    completed = subprocess.run(  # nosec B603
        [executable, *command[1:]],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        return None
    return completed.stdout.strip().splitlines()[0]


def _package_version(distribution: str) -> str | None:
    try:
        return importlib.metadata.version(distribution)
    except importlib.metadata.PackageNotFoundError:
        return None


def _run_suite(arguments: argparse.Namespace) -> int:
    revision = _run_git("rev-parse", "HEAD")
    dirty = bool(_run_git("status", "--porcelain"))
    if arguments.publish and dirty:
        raise RuntimeError("refusing to publish benchmark results from a dirty working tree")
    if arguments.publish and arguments.revision and arguments.revision != revision:
        raise RuntimeError("published benchmark revision must equal the checked-out immutable HEAD")

    selected = tuple(arguments.scenario or SCENARIOS)
    scenario_results: dict[str, Any] = {}
    for name in selected:
        for _index in range(arguments.warmups):
            _run_sample(name)
        samples = [_run_sample(name) for _index in range(arguments.samples)]
        scenario_results[name] = {"samples": samples, "summary": _summarize(samples)}
        median = scenario_results[name]["summary"]["wall_seconds"]["median"]
        print(f"{name}: median {median:.6f}s", file=sys.stderr)

    import pydmg._pydmg as native

    benchmark_native_path = Path(cast(str, native.__file__))
    wheel_native_bytes = _wheel_native_size(arguments.wheel)
    if arguments.native_artifact is not None:
        production_native_path: str | None = str(arguments.native_artifact)
        production_native_bytes = arguments.native_artifact.stat().st_size
    elif wheel_native_bytes is not None:
        production_native_path = "native extension inside the recorded wheel"
        production_native_bytes = wheel_native_bytes
    else:
        production_native_path = str(benchmark_native_path)
        production_native_bytes = benchmark_native_path.stat().st_size
    report = {
        "schema_version": 1,
        "label": arguments.label,
        "source_revision": arguments.revision or revision,
        "checked_out_revision": revision,
        "working_tree_dirty": dirty,
        "published": bool(arguments.publish),
        "samples_per_scenario": arguments.samples,
        "warmups_per_scenario": arguments.warmups,
        "environment": {
            "platform": platform.platform(),
            "machine": platform.machine(),
            "python": sys.version,
            "rustc": _command_version(["rustc", "--version"]),
            "cargo": _command_version(["cargo", "--version"]),
            "maturin": _package_version("maturin"),
        },
        "fixtures": {
            str(FAT_FIXTURE.relative_to(ROOT)): {
                "bytes": FAT_FIXTURE.stat().st_size,
                "sha256": _sha256(FAT_FIXTURE),
            },
            str(APFS_FIXTURE.relative_to(ROOT)): {
                "bytes": APFS_FIXTURE.stat().st_size,
                "sha256": _sha256(APFS_FIXTURE),
            },
        },
        "artifacts": {
            "benchmark_native_path": str(benchmark_native_path),
            "benchmark_native_bytes": benchmark_native_path.stat().st_size,
            "native_path": production_native_path,
            "native_bytes": production_native_bytes,
            "wheel_path": str(arguments.wheel) if arguments.wheel else None,
            "wheel_bytes": _artifact_size(arguments.wheel),
            "wheel_native_bytes": wheel_native_bytes,
        },
        "scenarios": scenario_results,
    }
    serialized = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output:
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(serialized, encoding="utf-8")
    else:
        print(serialized, end="")
    return 0


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", choices=SCENARIOS, help=argparse.SUPPRESS)
    parser.add_argument("--label", default="local", help="human-readable result label")
    parser.add_argument("--revision", help="source revision represented by this build")
    parser.add_argument("--samples", type=int, default=9)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--scenario", action="append", choices=SCENARIOS)
    parser.add_argument("--wheel", type=Path, help="production wheel whose size should be recorded")
    parser.add_argument(
        "--native-artifact",
        type=Path,
        help="production native library whose size should be recorded",
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--publish",
        action="store_true",
        help="require a clean immutable checkout and mark the report publishable",
    )
    return parser


def main() -> int:
    arguments = _parser().parse_args()
    if arguments.worker:
        return _run_worker(arguments.worker)
    if arguments.samples < 2:
        raise ValueError("--samples must be at least 2")
    if arguments.warmups < 0:
        raise ValueError("--warmups must not be negative")
    return _run_suite(arguments)


if __name__ == "__main__":
    raise SystemExit(main())
