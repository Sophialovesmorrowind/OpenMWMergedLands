#!/usr/bin/env python3
"""Benchmark two release binaries against an existing OpenMW load order.

Game inputs are read in place. Application config, plugin output, conflict PNGs,
and logs stay in a new work directory. Warmups and validation are not timed.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import statistics
import struct
import subprocess
import tomllib

from perf_sweep import positive_int, run_once


def file_hash(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def decoded_hash(path):
    """Normalize the first Header record only, then hash JSON without loading it."""
    digest = hashlib.sha256()
    with path.open("rb") as source:
        prefix = source.read(65536)
        boundary = prefix.find(b'},{"type":')
        if not prefix.startswith(b'[{"type":"Header"') or boundary < 0:
            raise ValueError("expected compact tes3conv output starting with Header")
        header = re.sub(rb"(?<=Generated at )\d+(?= UTC\.)", b"<time>", prefix[:boundary])
        digest.update(header)
        digest.update(prefix[boundary:])
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def record_counts(path):
    counts = {}
    with path.open("rb") as source:
        while header := source.read(16):
            if len(header) != 16:
                raise ValueError("truncated record header")
            tag, size, _, _ = struct.unpack("<4sIII", header)
            tag = tag.decode("ascii")
            counts[tag] = counts.get(tag, 0) + 1
            source.seek(size, 1)
        if source.tell() != path.stat().st_size:
            raise ValueError("truncated record payload")
    return counts


def log_details(path):
    clean = re.sub(r"\x1b\[[0-9;]*m", "", path.read_text(encoding="utf-8"))
    return {
        "warnings_and_errors": [line for line in clean.splitlines() if re.search(r"\[(?:WARN|ERROR)\]", line)],
        "workload_lines": [line for line in clean.splitlines() if re.search(
            r"Parsed \d+ data directories|Found \d+ masters|plugins contain LAND|Saving \d+|unique LTEX", line)],
    }


def compare(args):
    root = args.work_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binaries = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    cfg = args.openmw_cfg.resolve()
    app_cfg = args.app_config.resolve()
    app_settings = tomllib.loads(app_cfg.read_text(encoding="utf-8"))
    # Copying a config with relative exclusions would change their meaning.
    if any(not Path(path).is_absolute() for path in app_settings.get("ignore_plugins_from_path", [])):
        raise ValueError("use absolute ignore_plugins_from_path entries for this benchmark")
    original_hashes = {str(path): file_hash(path) for path in (cfg, app_cfg)}
    warmups = {}
    samples = {name: [] for name in binaries}
    for name, binary in binaries.items():
        run_dir = root / name
        (run_dir / "config").mkdir(parents=True)
        (run_dir / "output").mkdir()
        (run_dir / "Conflicts").mkdir()
        shutil.copyfile(app_cfg, run_dir / "config" / "merged_lands.toml")
        print(f"Starting {name} warmup: {cfg}", flush=True)
        warmups[name] = run_once(binary, cfg, run_dir, "openmw")
        print(f"{name} warmup: {warmups[name]['seconds']:.3f} s, "
              f"{warmups[name]['peak_rss_kib']} KiB peak RSS", flush=True)
    for iteration in range(args.repeats):
        order = list(binaries) if iteration % 2 == 0 else list(reversed(binaries))
        for name in order:
            print(f"Starting {name} sample {iteration + 1}/{args.repeats}", flush=True)
            sample = run_once(binaries[name], cfg, root / name, "openmw")
            if sample["output"] != warmups[name]["output"]:
                raise RuntimeError(f"{name} output changed between runs: {root}")
            samples[name].append(sample)
            print(f"{name}: {sample['seconds']:.3f} s", flush=True)
    summaries = {}
    for name, runs in samples.items():
        phases = set.intersection(*(set(run["phases_seconds"]) for run in runs))
        rss = [run["peak_rss_kib"] for run in runs if run["peak_rss_kib"] is not None]
        summaries[name] = {
            "median_seconds": statistics.median(run["seconds"] for run in runs),
            "median_peak_rss_kib": statistics.median(rss) if rss else None,
            "median_phases_seconds": {phase: statistics.median(run["phases_seconds"][phase] for run in runs)
                                      for phase in sorted(phases)},
            "log_details": log_details(root / name / "run.log"),
            "output_record_counts": record_counts(root / name / "output" / "Merged Lands.omwaddon"),
        }
    decoded = None
    if args.tes3conv:
        hashes = {}
        for name in binaries:
            print(f"Decoding {name} output with tes3conv", flush=True)
            output = root / name / "output.json"
            subprocess.run([args.tes3conv, "--compact", "--overwrite",
                            str(root / name / "output" / "Merged Lands.omwaddon"), str(output)], check=True)
            hashes[name] = decoded_hash(output)
        decoded = {"tool": args.tes3conv, "equal_except_generation_time": hashes["baseline"] == hashes["candidate"],
                   "decoded_sha256": hashes}
    for path, expected in original_hashes.items():
        if file_hash(Path(path)) != expected:
            raise RuntimeError(f"input config changed during benchmark: {path}")
    before = summaries["baseline"]["median_seconds"]
    after = summaries["candidate"]["median_seconds"]
    return {
        "workload": {"openmw_cfg": str(cfg), "app_config": str(app_cfg), "repeats": args.repeats,
                     "warmups_per_binary": 1, "cache": "warm", "mode": "openmw",
                     "application_settings": app_settings, "config_sha256": original_hashes},
        "work_dir": str(root),
        "binaries": {name: {"path": str(path), "sha256": file_hash(path)} for name, path in binaries.items()},
        "output_equal_except_generation_time": warmups["baseline"]["output"] == warmups["candidate"]["output"],
        "tes3conv": decoded, "summary": summaries,
        "seconds_saved": before - after, "time_reduction_percent": (before - after) / before * 100,
        "speedup": before / after, "warmups": warmups, "samples": samples,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--openmw-cfg", required=True, type=Path)
    parser.add_argument("--app-config", required=True, type=Path)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--repeats", type=positive_int, default=3)
    parser.add_argument("--json", required=True, type=Path)
    parser.add_argument("--tes3conv", default=shutil.which("tes3conv"))
    args = parser.parse_args()
    report = compare(args)
    args.json.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"summary": report["summary"], "seconds_saved": report["seconds_saved"],
                      "time_reduction_percent": report["time_reduction_percent"],
                      "output_equal": report["output_equal_except_generation_time"],
                      "tes3conv": report["tes3conv"]}, indent=2), flush=True)


if __name__ == "__main__":
    main()
