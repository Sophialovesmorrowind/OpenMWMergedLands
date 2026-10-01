#!/usr/bin/env python3
"""Compare release binaries on generated OpenMW terrain; no game files required.

Example: python3 ci/perf_sweep.py --baseline /tmp/old/merged_lands \
    --candidate target/release/merged_lands --json /tmp/perf.json
Fixture construction and warmup are outside the measured region. Measured runs
alternate binary order, include PNG/plugin output, and verify output bytes except
the header's generation timestamp. Optionally verify decoded records with tes3conv.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import struct
import subprocess
import sys
import tempfile
import time


def subrecord(tag, payload):
    return tag + struct.pack("<I", len(payload)) + payload


def record(tag, payload):
    return tag + struct.pack("<III", len(payload), 0, 0) + payload


def texture(index):
    return record(
        b"LTEX",
        subrecord(b"NAME", f"perf_texture_{index}\0".encode())
        + subrecord(b"INTV", struct.pack("<I", index))
        + subrecord(b"DATA", f"perf/texture_{index}.dds\0".encode()),
    )


def landscape(x, y, stage, texture_count, height_step):
    # Most normals can be reused. Sparse border changes require real seam repair;
    # changing the center at every stage exercises rolling-reference merging.
    heights = [[0] * 65 for _ in range(65)]
    heights[32][32] = stage * height_step
    if x % 3 == 0:
        heights[1:64] = [row[:64] + [stage * height_step] for row in heights[1:64]]
    deltas = bytearray(65 * 65)
    for row in range(65):
        deltas[row * 65] = (heights[row][0] - heights[max(row - 1, 0)][0]) % 256
        for col in range(1, 65):
            deltas[row * 65 + col] = (heights[row][col] - heights[row][col - 1]) % 256
    payload = (
        subrecord(b"INTV", struct.pack("<ii", x, y))
        + subrecord(b"DATA", struct.pack("<I", 15))
        + subrecord(b"VNML", bytes([0, 0, 127]) * (65 * 65))
        + subrecord(b"VHGT", struct.pack("<f", 0.0) + deltas + bytes(3))
        + subrecord(b"WNAM", bytes(81))
        + subrecord(b"VCLR", bytes([80, 100, 120]) * (65 * 65))
        + subrecord(b"VTEX", struct.pack("<H", 1 + (x + y + stage) % texture_count) * 256)
    )
    return record(b"LAND", payload)


def write_plugin(path, grid, stage, texture_count, height_step, master=None):
    header = subrecord(
        b"HEDR",
        struct.pack(
            "<fI32s256sI", 1.3, int(master is None), b"perf sweep",
            b"synthetic terrain benchmark", grid * grid + texture_count,
        ),
    )
    if master is not None:
        header += subrecord(b"MAST", master.name.encode() + b"\0")
        header += subrecord(b"DATA", struct.pack("<Q", master.stat().st_size))
    with path.open("wb") as output:
        output.write(record(b"TES3", header))
        for index in range(texture_count):
            output.write(texture(index))
        for x in range(grid):
            for y in range(grid):
                output.write(landscape(x, y, stage, texture_count, height_step))


def make_fixture(root, grid, plugins, textures, height_step):
    data = root / "data"
    data.mkdir()
    master = data / "PerfBase.esm"
    write_plugin(master, grid, 0, textures, height_step)
    names = [master.name]
    for stage in range(1, plugins + 1):
        path = data / f"PerfMod{stage}.esp"
        write_plugin(path, grid, stage, textures, height_step, master)
        names.append(path.name)
    data_local = root / "data-local"
    data_local.mkdir()
    cfg = root / "openmw.cfg"
    cfg.write_text(
        f'data="{data}"\ndata-local="{data_local}"\n'
        + "".join(f"content={name}\n" for name in names), encoding="utf-8",
    )
    return cfg


def output_digest(run_dir, output_name="Merged Lands.omwaddon"):
    files = sorted((run_dir / "Conflicts").rglob("*.png"))
    files.append(run_dir / "output" / output_name)
    digest = hashlib.sha256()
    for path in files:
        digest.update(str(path.relative_to(run_dir)).encode())
        digest.update(b"\0")
        data = path.read_bytes()
        if path.suffix in {".omwaddon", ".esp"}:
            # The writer emits TES3/HEDR first; normalize only its fixed-width
            # 256-byte description, leaving every other field byte-exact.
            if data[:4] != b"TES3" or data[16:24] != b"HEDR" + struct.pack("<I", 300):
                raise ValueError("expected a TES3 header with a 300-byte HEDR")
            description, terminator, padding = data[64:320].partition(b"\0")
            description = re.sub(
                rb"(?<=Generated at )\d+(?= UTC\.)", lambda match: b"0" * len(match[0]), description,
            )
            data = data[:64] + description + terminator + padding + data[320:]
        digest.update(data)
    return {"sha256_except_generation_time": digest.hexdigest(), "files": len(files)}


def parse_phases(log_path):
    phases = {}
    for line in log_path.read_text(encoding="utf-8").splitlines():
        match = re.search(r"\[DEBUG\]\s+(.+?) in ([\d.]+)(ns|µs|ms|s)$", line)
        if match:
            label, duration, unit = match.groups()
            phases[label] = float(duration) * {"ns": 1e-9, "µs": 1e-6, "ms": 1e-3, "s": 1}[unit]
    return phases


def run_once(binary, cfg, run_dir, mode):
    command = [
        str(binary), "--config-dir", str(run_dir / "config"),
        "--merged-lands-dir", str(run_dir), "--output-file-dir", str(run_dir / "output"),
        "--log-level", "off",
    ]
    if mode == "openmw":
        command.extend(["--openmw-cfg", str(cfg)])
    else:
        command.extend(["--vanilla", "--data-files-dir", str(cfg.parent / "data")])
        command.extend(line.removeprefix("content=") for line in cfg.read_text().splitlines()
                       if line.startswith("content="))
    log_path = run_dir / "run.log"
    peak_rss_kib = None
    with log_path.open("wb") as log:
        start = time.perf_counter()
        with subprocess.Popen(command, cwd=run_dir, stdout=log, stderr=subprocess.STDOUT) as process:
            if hasattr(os, "wait4"):
                _, status, usage = os.wait4(process.pid, 0)
                process.returncode = os.waitstatus_to_exitcode(status)
                peak_rss_kib = usage.ru_maxrss / 1024 if sys.platform == "darwin" else usage.ru_maxrss
            else:
                process.wait()
            if process.returncode:
                raise subprocess.CalledProcessError(process.returncode, command)
        elapsed = time.perf_counter() - start
    return {
        "seconds": elapsed,
        "peak_rss_kib": peak_rss_kib,
        "phases_seconds": parse_phases(log_path),
        "output": output_digest(run_dir, "Merged Lands.esp" if mode == "vanilla" else "Merged Lands.omwaddon"),
    }


def compare(args, root):
    cfg = make_fixture(root, args.grid, args.plugins, args.textures, args.height_step)
    binaries = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    samples = {name: [] for name in binaries}
    expected = None
    for name, binary in binaries.items():
        run_dir = root / name
        (run_dir / "config").mkdir(parents=True)
        (run_dir / "output").mkdir()
        (run_dir / "Conflicts").mkdir()
        warmup = run_once(binary, cfg, run_dir, args.mode)
        if expected is None:
            expected = warmup["output"]
        if warmup["output"] != expected:
            raise RuntimeError(f"{name} warmup output differs from baseline: {root}")
    for iteration in range(args.repeats):
        order = list(binaries) if iteration % 2 == 0 else list(reversed(binaries))
        for name in order:
            sample = run_once(binaries[name], cfg, root / name, args.mode)
            if sample["output"] != expected:
                raise RuntimeError(f"{name} output differs from baseline: {root}")
            samples[name].append(sample)
    summaries = {}
    for name, runs in samples.items():
        phases = runs[0]["phases_seconds"]
        rss = [run["peak_rss_kib"] for run in runs if run["peak_rss_kib"] is not None]
        summaries[name] = {
            "median_seconds": statistics.median(run["seconds"] for run in runs),
            "median_peak_rss_kib": statistics.median(rss) if rss else None,
            "median_phases_seconds": {
                phase: statistics.median(run["phases_seconds"][phase] for run in runs)
                for phase in phases
            },
        }
    decoded_comparison = None
    if args.tes3conv is not None:
        decoded = {}
        for name in binaries:
            output = root / name / "output.json"
            subprocess.run([
                args.tes3conv, "--compact", "--overwrite",
                str(root / name / "output" / ("Merged Lands.esp" if args.mode == "vanilla" else "Merged Lands.omwaddon")), str(output),
            ], check=True)
            records = json.loads(output.read_text(encoding="utf-8"))
            for entry in records:
                if entry["type"] == "Header":
                    entry["description"] = re.sub(
                        r"Generated at \d+ UTC\.", "Generated at <time> UTC.", entry["description"],
                    )
            decoded[name] = records
        if decoded["baseline"] != decoded["candidate"]:
            raise RuntimeError(f"tes3conv records differ: {root}")
        counts = {}
        for entry in decoded["candidate"]:
            counts[entry["type"]] = counts.get(entry["type"], 0) + 1
        decoded_comparison = {"tool": args.tes3conv, "equal_except_generation_time": True,
                              "record_counts": counts}
    return {
        "workload": {"grid": args.grid, "cells": args.grid**2, "plugins": args.plugins,
                     "textures_per_plugin": args.textures, "repeats": args.repeats,
                     "mode": args.mode, "height_step": args.height_step},
        "binaries": {name: str(path) for name, path in binaries.items()},
        "output": expected,
        "tes3conv": decoded_comparison,
        "summary": summaries,
        "speedup": summaries["baseline"]["median_seconds"] / summaries["candidate"]["median_seconds"],
        "samples": samples,
    }


def positive_int(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--grid", type=positive_int, default=12, help="cells per side")
    parser.add_argument("--plugins", type=positive_int, default=6)
    parser.add_argument("--textures", type=positive_int, default=64)
    parser.add_argument("--repeats", type=positive_int, default=5)
    parser.add_argument("--height-step", type=positive_int, default=8,
                        help="VHGT increment per plugin")
    parser.add_argument("--mode", choices=["openmw", "vanilla"], default="openmw")
    parser.add_argument("--work-dir", type=Path, help="retain fixtures/results in a new directory")
    parser.add_argument("--json", type=Path, help="also write the report to this file")
    parser.add_argument("--tes3conv", default=shutil.which("tes3conv"),
                        help="decode and compare plugin records (defaults to tes3conv on PATH)")
    args = parser.parse_args()
    if args.plugins * args.height_step > 127 or args.textures > 65534:
        parser.error("plugins * height-step must be <= 127 and textures <= 65534 for this fixture")
    if args.work_dir is not None:
        args.work_dir = args.work_dir.resolve()
        args.work_dir.mkdir(parents=True, exist_ok=False)
        report = compare(args, args.work_dir)
    else:
        with tempfile.TemporaryDirectory(prefix="merged_lands_perf_") as directory:
            report = compare(args, Path(directory))
    output = json.dumps(report, indent=2) + "\n"
    if args.json is not None:
        args.json.write_text(output, encoding="utf-8")
    print(output, end="")


if __name__ == "__main__":
    main()
