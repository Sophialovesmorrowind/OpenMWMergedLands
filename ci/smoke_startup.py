#!/usr/bin/env python3
"""Verify first-run OpenMW discovery completes with terminal stdin and no input.

Run on a POSIX host after building: python3 ci/smoke_startup.py target/debug/merged_lands
Uses temporary generated plugins and an OPENMW_CONFIG override.
"""

import argparse
import os
from pathlib import Path
import pty
import subprocess
import tempfile

from perf_sweep import make_fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="merged_lands_startup_") as directory:
        root = Path(directory)
        cfg = make_fixture(root, 2, 1, 3, 1)
        work = root / "work"
        work.mkdir()
        output = root / "output"
        output.mkdir()
        master, slave = pty.openpty()
        try:
            assert os.isatty(slave)
            command = [
                str(binary), "--config-dir", str(root / "config"),
                "--merged-lands-dir", str(work), "--output-file-dir", str(output),
            ]
            with subprocess.Popen(
                command, stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, cwd=work, env={**os.environ, "OPENMW_CONFIG": str(cfg)},
            ) as process:
                try:
                    log, _ = process.communicate(timeout=30)
                except subprocess.TimeoutExpired:
                    process.kill()
                    log, _ = process.communicate()
                    raise AssertionError(f"Startup blocked on terminal input:\n{log}") from None
                assert process.returncode == 0, log
            assert "First run setup" not in log, log
            assert "Enter 1 or 2" not in log, log
            assert (output / "Merged Lands.omwaddon").is_file(), log
            print("PASS: first run auto-detects and merges with terminal stdin without input")
        finally:
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    main()
