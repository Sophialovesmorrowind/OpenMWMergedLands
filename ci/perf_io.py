#!/usr/bin/env python3
"""Compare DataDirs resolution against a Git revision without building Cargo dependencies."""

from __future__ import annotations

import argparse
import json
import platform
import statistics
import subprocess
import tempfile
from pathlib import Path


REPO = Path(__file__).resolve().parent.parent
SOURCE = "src/io/parsed_plugins.rs"
HARNESS = r"""
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;
type Result<T, E = String> = std::result::Result<T, E>;

__SOURCE__

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let dir_count: usize = args[2].parse().unwrap();
    let plugin_count: usize = args[3].parse().unwrap();
    let trials: usize = args[4].parse().unwrap();
    let dirs: Vec<_> = (0..dir_count).map(|n| root.join(format!("data_{n:04}"))).collect();
    if !root.exists() {
        for directory in &dirs { fs::create_dir_all(directory).unwrap(); }
        for n in 0..plugin_count {
            fs::write(dirs[n % dirs.len()].join(format!("Plugin_{n:06}.esp")), []).unwrap();
        }
    }
    let plugins: Vec<_> = (0..plugin_count).map(|n| format!("Plugin_{n:06}.esp")).collect();
    let sidecars: Vec<_> = (0..plugin_count).map(|n| format!("Plugin_{n:06}.mergedlands.toml")).collect();
    for (workload, resolve_plugins, resolve_sidecars) in [
        ("exact-plugins", true, false),
        ("missing-sidecars", false, true),
        ("plugins-and-sidecars", true, true),
    ] {
        let mut times = Vec::new();
        for _ in 0..trials {
            let data = DataDirs::from_ordered(dirs.clone()).unwrap();
            let started = Instant::now();
            let mut found = 0usize;
            for n in 0..plugin_count {
                if resolve_plugins { found += usize::from(data.resolve(&plugins[n]).is_some()); }
                if resolve_sidecars { assert!(data.resolve(&sidecars[n]).is_none()); }
            }
            let elapsed = started.elapsed();
            assert_eq!(found, if resolve_plugins { plugin_count } else { 0 });
            times.push(elapsed.as_secs_f64() * 1000.0);
        }
        println!("{{\"workload\":\"{workload}\",\"milliseconds\":{times:?}}}");
    }
}
"""


def positive_integer(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return number


def extract_data_dirs(source: str) -> str:
    """Isolate the std-only resolver; replace its one anyhow error with a String."""
    source = source[source.index("/// A set of directories") : source.index("// PluginListSource")]
    return source.replace(
        'bail!("DataDirs must contain at least one directory");',
        'return Err("DataDirs must contain at least one directory".to_string());',
    )


def measure(source: str, directory: Path, variant: str, args: argparse.Namespace) -> dict:
    rust_source = directory / f"{variant}.rs"
    executable = directory / (f"{variant}.exe" if platform.system() == "Windows" else variant)
    rust_source.write_text(HARNESS.replace("__SOURCE__", extract_data_dirs(source)), encoding="utf-8")
    subprocess.run(
        ["rustc", "--edition=2024", "-O", "-A", "dead_code", str(rust_source), "-o", str(executable)],
        check=True,
    )
    output = subprocess.check_output(
        [str(executable), str(directory / "fixture"), str(args.data_dirs), str(args.plugins), str(args.trials)],
        text=True,
    )
    results = {}
    for line in output.splitlines():
        result = json.loads(line)
        samples = result["milliseconds"]
        results[result["workload"]] = {
            "median_ms": statistics.median(samples),
            "min_ms": min(samples),
            "max_ms": max(samples),
            "samples_ms": samples,
        }
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-ref", default="HEAD", help="Git revision to compare with (default: HEAD)")
    parser.add_argument("--data-dirs", type=positive_integer, default=128)
    parser.add_argument("--plugins", type=positive_integer, default=1000)
    parser.add_argument("--trials", type=positive_integer, default=7)
    parser.add_argument("--output", type=Path, help="also write the JSON report to this file")
    args = parser.parse_args()
    baseline = subprocess.check_output(["git", "show", f"{args.baseline_ref}:{SOURCE}"], cwd=REPO, text=True)
    current = (REPO / SOURCE).read_text(encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="merged_lands_perf_io_") as temp:
        directory = Path(temp)
        before = measure(baseline, directory, "before", args)
        after = measure(current, directory, "after", args)
    report = {
        "baseline_ref": args.baseline_ref,
        "platform": platform.platform(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "data_dirs": args.data_dirs,
        "plugins": args.plugins,
        "trials": args.trials,
        "measurement": "Resolution only; fresh DataDirs each trial, warm filesystem, no plugin parsing",
        "before": before,
        "after": after,
        "speedup": {name: before[name]["median_ms"] / after[name]["median_ms"] for name in before},
    }
    output = json.dumps(report, indent=2) + "\n"
    if args.output is not None:
        args.output.write_text(output, encoding="utf-8")
    print(output, end="")


if __name__ == "__main__":
    main()
