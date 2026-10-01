# Performance sweep

The sweep compared release builds against commit `8f38ca8` on Linux x86_64 with
Rust 1.94.0. Both generated fixtures and the active full modlist are measured;
game files and saved configuration are not modified. The complete-pipeline benchmark measures parsing,
merging, seam repair, conflict images, cleanup, conversion, and output together.

## Active full modlist

The saved application config selects `~/.config/openmw/openmw.cfg`: 144 content
entries and 191 configured data directories. Its exclusions remove three
generated addons, leaving 141 inputs. Ten plugins contain 16,075 input LAND
records. Both builds use the same saved merge settings and plugin sidecars, with
all writable config, plugin output, PNGs, and logs isolated under `/tmp`.

| Full pipeline, warm cache | Prior build (`8f38ca8`) | Updated build | Saved per run |
| --- | ---: | ---: | ---: |
| Median of three measured runs | 6.175 s | 4.565 s | **1.610 s (26.1%)** |

Each binary has one untimed warmup; measured runs alternate binary order. The
baseline samples are 6.462, 6.124, and 6.175 seconds; updated samples are 4.654,
4.565, and 4.439 seconds. This is a 1.35× speedup on this machine with warm
filesystem caches. It does not measure cold startup or build time.

| Median phase | Prior | Updated | Saved |
| --- | ---: | ---: | ---: |
| Parse plugins and discover inputs | 0.404 s | 0.120 s | 0.284 s |
| Build references and plugin diffs | 1.489 s | 1.447 s | 0.042 s |
| Merge and repair seams | 1.831 s | 1.275 s | 0.556 s |
| Write conflict PNGs | 1.122 s | 1.056 s | 0.066 s |
| Clean merged terrain | 0.845 s | 0.296 s | 0.549 s |
| Convert LAND records | 0.316 s | 0.189 s | 0.127 s |

Independent phase medians need not add up to the median complete run. Median
peak RSS is 7,771,548 → 7,736,364 KiB, a reduction of about 34 MiB.

All 1,293 conflict PNGs and the generated plugin match byte-for-byte after
normalizing only the header timestamp. Independent `tes3conv` decoding also
matches: one Header, 282 LTEX records, and 2,262 LAND records. No warnings or
errors occur during successful runs. [Raw full-modlist results](perf-modlist-results.json)
include all samples, hashes, settings, and the exact benchmark content order.

The saved order initially prevents both builds from merging: `Sotha Sil
Expanded.ESP` declares `OAAB_Data.esm` as a master but precedes it. OpenMW's
[engine loader](https://github.com/OpenMW/openmw/blob/f4bec41444214a7903bebd178389ca22ca13f646/apps/openmw/mwworld/esmloader.cpp#L46-L55)
also rejects a missing or later master. For this comparison only, a temporary
config moves Sotha Sil from content position 6 to position 10, immediately after
OAAB. Every content entry and all other settings are retained. The saved source
config remains unchanged.

To repeat this workload, copy the source config to a temporary path and make
that one order adjustment, then run:

```sh
python3 ci/perf_modlist.py --baseline target/perf-baseline/release/merged_lands --candidate target/release/merged_lands --openmw-cfg /tmp/benchmark-openmw.cfg --app-config "$HOME/.config/openmw/merged_lands.toml" --work-dir /tmp/merged-lands-modlist-repeat --repeats 3 --json /tmp/full-modlist-perf.json
```

The work directory must be new. `tes3conv` defaults to the executable on PATH.
JSON verification hashes the decoded streams without retaining both in memory.

## Complete pipeline

Each comparison uses a 12×12 cell grid, 64 textures per input plugin, one warmup
per binary, and seven measured runs with alternating binary order. Fixture
creation and output validation are outside the measured interval.

| Workload | Baseline median | Updated median | Speedup |
| --- | ---: | ---: | ---: |
| OpenMW, six overlapping plugins, conflict PNGs | 460.8 ms | 434.8 ms | 1.06× |
| Classic mode, six overlapping plugins, conflict PNGs | 475.0 ms | 439.4 ms | 1.08× |
| OpenMW, one plugin, no conflict PNGs | 108.8 ms | 92.1 ms | 1.18× |

The final sweep reduces wall time by 5.7%, 7.5%, and 15.3%, respectively.
[Raw final measurements](perf-results.json) include every sample, phase timing,
peak RSS, binary hashes, and `tes3conv` verification results. Earlier repetitions
showed somewhat larger gains; these final figures use the build after config and
startup changes. Small phase differences are subject to normal timing variation.

The OpenMW conflict workload's median phases changed as follows:

| Phase | Baseline | Updated |
| --- | ---: | ---: |
| Build rolling references and plugin diffs | 165.8 ms | 167.8 ms |
| Merge diffs and repair seams | 49.6 ms | 37.5 ms |
| Write conflict images | 199.3 ms | 195.3 ms |
| Clean output against loaded terrain | 9.8 ms | 4.8 ms |
| Convert output LAND records | 10.8 ms | 7.3 ms |

Peak resident memory changed modestly: 438,988 → 434,648 KiB for the OpenMW
conflict workload. Most of the remaining time is in constructing diffs and
writing PNGs. The component improvements below do not translate directly into
whole-program speedups.

## Changes and isolated measurements

- Seam repair borrows both adjacent cells through `HashMap::get_disjoint_mut`.
  It previously removed and reinserted 175,544-byte inline cells per edge.
  Normal masks now update in place. Across 576-cell dense clean, dense seamed,
  and sparse clean fixtures, medians improved from 21–24 ms to 3–4 ms.
- Cleanup converts only the loaded terrain fields that are needed and stops
  after finding a difference. Ten thousand texture-only comparisons improved
  from 344.9 ms to 6.1 ms. Removal uses `retain` without a temporary key list.
- Normal recomputation reuses unchanged nonzero normals before calculating new
  values. A 1%-changed 65×65 normal grid improved from 93.0 to 26.7 µs with
  Rust 1.94.0. Rust 1.88 also showed sparse gains; dense/no-existing-normal
  results were approximately unchanged, with up to 3% overhead in some samples.
- Texture fallback lookup caches the first real remapped texture and invalidates
  on every insertion/replacement, eliminating a repeated full-map scan.
- LAND insertion moves newly owned buffers. Rolling reference updates mutate
  existing LAND records, and diff merging avoids cloning terrain it replaces.
  Serialization reuses texture ordering and clones dependency names once.
- Discovery indexes directories once. With 128 data directories and 1,000
  plugins, exact plugin resolution improved from 95.1 to 12.0 ms; absent sidecars
  from 997.6 to 19.9 ms; combined resolution from 1,094.6 to 29.8 ms. These
  measurements use fresh resolver caches over a warm filesystem and exclude
  plugin decoding. A regression also fixes higher-priority case-insensitive
  files losing to exact-case lower-priority files.
- Conflict image rendering skips unchanged samples while preserving PNG bytes.

Packaging the actual 3.28 MB release executable took about 107 ms. No packaging
rewrite was justified. CI dependency caching remains an unmeasured follow-up.

## Output verification

Every measured output matches the baseline, including all conflict PNG names
and bytes. Only the generated-at timestamp digits in the fixed-width TES3
header description are normalized; all other header, dependency, LAND, LTEX,
and description padding bytes are preserved. Tests verify changes in those
fields are detected. Same-session timestamps have the same digit width.

`tes3conv` from PATH independently decodes both outputs. Complete JSON record
comparisons match after normalizing the header's generation timestamp. Each
workload writes one Header, 22 LTEX records, 96 LAND records, and zero CELL
records. The conflict workloads also write 240 PNGs.

## Reproduce

Build the baseline in a separate directory, using the same toolchain and locked
dependencies as the candidate:

```sh
mkdir -p /tmp/merged-lands-baseline
git archive 8f38ca8 | tar -x -C /tmp/merged-lands-baseline
cargo build --release --locked --manifest-path /tmp/merged-lands-baseline/Cargo.toml --target-dir target/perf-baseline
cargo build --release --locked
python3 ci/perf_sweep.py --baseline target/perf-baseline/release/merged_lands --candidate target/release/merged_lands --repeats 7 --json /tmp/openmw-perf.json
python3 ci/perf_sweep.py --baseline target/perf-baseline/release/merged_lands --candidate target/release/merged_lands --mode vanilla --repeats 7 --json /tmp/vanilla-perf.json
python3 ci/perf_sweep.py --baseline target/perf-baseline/release/merged_lands --candidate target/release/merged_lands --plugins 1 --height-step 1 --repeats 7 --json /tmp/no-png-perf.json
```

The runner automatically uses `tes3conv` when available on PATH. `--work-dir`
retains input fixtures, decoded outputs, PNGs, and logs in a new directory.
Peak RSS uses Unix `wait4`; other hosts leave that measurement unspecified.

Isolated benchmarks are separate from normal CI tests:

```sh
python3 ci/perf_io.py --baseline-ref 8f38ca8
python3 ci/perf_terrain.py
python3 ci/perf_repair.py --source /tmp/merged-lands-baseline
python3 ci/perf_repair.py
```

The repair benchmark excludes fixture creation and cell cloning and prints
repair counts plus checksums covering every height/normal value and delta.
The terrain benchmark asserts complete normal-grid equality against frozen
pre-sweep calculations and defaults to the repository's pinned toolchain.
Historical Rust 1.88 measurements above predate the current Rust 1.99 requirement.

The startup smoke uses a terminal for stdin without supplying input:

```sh
cargo build --locked
python3 ci/smoke_startup.py target/debug/merged_lands
```

The generated workloads above are synthetic warm-cache results from one machine. Real load orders can
spend different proportions of time in decoding, conflict images, and merging.
The byte/record comparisons cover the generated workloads; semantic regression
tests cover missing/excluded data, normal masks, corners, and cache invalidation.
