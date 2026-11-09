# Terrain storage audit fixes

This branch fixes terrain storage and cleanup in Merged Lands. The audit used
OpenMW 0.51.0, the installed engine version, and a copied full mod-list config.
All generated plugins, configuration writes, images, and logs stayed under
`/tmp`. The installed tools pack and game output were unchanged.

## Review order

| Commit | Change |
| --- | --- |
| `f3750a9` | Repair shared heights before VHGT encoding; reject unrepresentable deltas. |
| `e73f8bb` | Compare cleanup with the whole winning LAND record. |
| `1f2c48e` | Allocate present relative terrain grids behind a Box. |
| `e417825` | Apply plugin diffs immediately; retain history only for requested diagnostics. |
| `e009dff` | Finalize quantized geometry and normals before cleanup. |
| `f35a2bd` | Preserve LTEX overrides when ID letter case differs. |
| `9ebee72` | Preserve independent authored normal edits and validate their source geometry. |
| `3060bd5` | Carry untouched winning layers in whole LAND overrides; honor layer inclusion bits. |
| `7f7a460` | Honor an LTEX override that clears its filename. |
| `4d95c52` | Retain present zero-valued layers when a later LAND omits them. |
| `274209b` | Make five existing end-to-end tests load their actual fixture files. |
| `0c7e5b1` | Preserve fractional VHGT offsets and a common phase across connected cells. |
| `510702d` | Preserve vectors synthesized by explicit `Resolve` with their geometry. |
| `2434e8d` | Quantize sub-world offsets once, avoiding a panic when heights cross zero. |
| `d8e5d87` | Associate VNML-only patches with the rolling source geometry. |

## Height storage

VHGT stores signed-byte horizontal steps and first-column vertical steps. One
stored unit is eight world units, so the limits are -1,024 and +1,016 world units.
Other vertical neighbors have no stored-step constraint.

The old writer clamped individual steps after seam repair. Because heights are
cumulative, a single clamp could move the rest of a row or cell. The audited
output had 12 affected height maps and 11 border pairs with worsened vertices;
the largest encoding error was 1,576 world units.

The repair now quantizes heights before cleanup. Connected cells share the most
frequent eight-unit phase in their planned heights, with a deterministic tie
break. A source VHGT offset of 0.5 therefore retains heights such as 4 and 516
rather than becoming 0 and 512. The integer terrain model still quantizes
fractions smaller than one world unit. It truncates the scaled offset once,
then adds integer steps, so crossing zero cannot split a cell into incompatible
phases.

After quantization, repair lowers the high endpoint of an invalid stored edge.
Changes propagate across every copy of a shared vertex, including four-cell
corners. Values decrease monotonically and remain bounded by the quantized
minimum, so repair needs no arbitrary iteration limit. Representable terrain
with a common phase stays unchanged. The encoder returns an error with cell
and vertex context if a delta still cannot be represented.

This policy changes unrepresentable planned terrain to fit the file format. It
does not claim to reconstruct an author's intended shape at a conflict. Tests
verify exact height round trips after repair, shared borders, negative cell
coordinates, corners, asymmetric bounds, and valid unstored vertical slopes.

## Whole records and author intent

OpenMW [replaces the entire LAND at a coordinate](https://github.com/OpenMW/openmw/blob/openmw-0.51.0/apps/openmw/mwworld/store.cpp#L455).
A later LAND missing a layer does not inherit that layer from an earlier LAND.
The raw winner used by cleanup now follows that rule. Terrain layer composition
remains a separate merge policy, so cleanup can retain an override needed to
restore a selected older layer.

Present layers remain available even if every value is zero. Treating an
unchanged zero grid as absent discarded predecessor terrain under a later
partial LAND. This distinction applies to heights, black colors, default
texture indices, and world-map data.

Before seam repair, cells also receive any untouched layers from the actual
winning LAND. A normal-only override must carry unchanged zero heights, black
vertex colors, and other enabled layers. Omitting those payloads would replace
them with engine defaults. Payloads disabled by DATA inclusion bits stay disabled.
Regressions compare complete LAND records and cover a flat zero-height cell
participating in seam repair.

Normals are independent authored data. They participate in the selected merge
rule even when heights do not change. Before cleanup, the tool checks each
selected vector against source records whose sampled triangle heights match the
final geometry. It checks the winning LAND first and loads earlier contributors
only if needed. A vector without a matching source is recalculated from the
final heights. This preserves authored lighting where its source geometry still
applies and updates normals invalidated by merging, quantization, or seam repair.

An explicit `Resolve` can synthesize a vector that occurs in no input record.
Those vectors retain a geometry snapshot and a mask of the vertices actually
synthesized. VNML-only patches retain the rolling reference geometry available
when they are read. Both kinds of snapshot undergo the same final geometry
check. They do not validate unrelated vertices or assume that the final merged
geometry was the source geometry.

A genuine normal-only override can remain. Tests cover retaining an earlier
normal edit with `Ignore`, accepting a later edit with `Auto`, neighboring normal
samples, and emitting VNML without inventing a missing VHGT payload. Zero
normal-only records in the full-list output is an observed result, not a pruning
rule.

LTEX uses [a plugin-local index mapping to a globally winning texture ID](https://github.com/OpenMW/openmw/blob/openmw-0.51.0/apps/openmw/mwworld/store.cpp#L383).
Texture IDs now use ASCII-insensitive identity throughout palette deduplication
and filename updates. A later LTEX-only patch changing `Grass01` to a new texture
file must win even if it spells the ID `gRaSs01` and uses another local index.
The end-to-end regression forces LAND output and checks both the emitted texture
filename and the texture patch's master dependency. An override with no filename
also clears the older filename, matching OpenMW's global ID replacement.

Pruning affects the generated patch. Source plugins remain active and continue
to supply their unchanged terrain and texture definitions. Unused LTEX records
are removed from the generated palette after the remaining LAND records are
known. Authored WNAM data keeps its existing merge policy; this work does not
regenerate every world-map height grid.

## Memory

A LAND diff previously reserved 175,544 bytes inline, including arrays for absent
optional layers. Each present relative terrain map now owns one allocation;
absent maps cost one pointer. Grid ownership stays independent when maps clone.

Normal runs apply and release each plugin diff immediately. Conflict images and
debug colors retain the history they need. The two memory commits were checked
against their preceding correctness build: the complete LAND/LTEX byte hash was
identical. Single-run peak RSS fell from 7.78 GiB to 2.14 GiB after allocation
changes, then to 1.69 GiB after streaming.

## Verification

The source passes 155 Rust tests, formatting, and strict pedantic Clippy on the
repository's pinned Rust 1.99.0 toolchain. The official stable release was checked
and was [1.99.1](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/); the repository
pin was kept unchanged for this work.

An independent parser decoded the output records and compared them with the
12,656 winning input LAND cells. It resolved texture IDs, checked subrecord
sizes and header counts, verified dependency file sizes, and compared borders
after placing the patch last in load order.

| Full-list output | Released 1.2 | Fixed |
| --- | ---: | ---: |
| LAND | 2,437 | 298 |
| LTEX | 289 | 202 |
| Master dependencies | 61 | 25 |
| Redundant LAND | 20 | 0 |
| Border pairs with worsened vertices | 11 | 0 |

The fixed output has no malformed payloads or invalid texture references.
Source files and scripts for the independent inspection are under
`/tmp/merged-lands-vs-wraithguard`; stage results are under
`/tmp/merged-lands-fixes/10-reviewed-final`. No in-game visual pass was performed.

## Adversarial review

A separate reviewer checked the code and built binary fixtures independently.
The first pass found five issues: lost explicit `Resolve` normals, lost
zero-valued predecessor layers, truncated fractional VHGT offsets, restored
LTEX filenames after a clearing override, and end-to-end tests reading empty
fixture directories. Each has a separate fix above.

The follow-up caught a new crossing-zero panic in the fractional decoder and
confirmed that VNML-only patches lost their authored vectors after inheriting
heights. The last two source commits fix those cases. Regression coverage also
checks that subsequent geometry edits invalidate affected resolved normals and
that an ignored height edit does not cause a partial normal source to validate
against the wrong terrain.

The final independent pass at `d8e5d87` found no remaining confirmed issues
within that scope. All asserted CLI reproductions passed.

The review fixtures and assertions are under
`/tmp/merged-lands-adversarial-review`. The review is bounded by those source
paths and reproductions; it is not proof that every possible terrain conflict
matches an author's intent.

## Ten-run comparison

Two warmups per binary were excluded. Ten measured runs per binary then
alternated order, using the same copied normal config, active plugin sidecars,
and release builds. Conflict images and debug colors were off. Elapsed time and
peak RSS came from each child process's `wait4` result. LAND/LTEX hashes were
stable across all ten measured runs for each binary.

| Mean over ten runs | Released 1.2 | Fixed |
| --- | ---: | ---: |
| Complete process time | 4.519 s | 2.890 s |
| Peak RSS | 7,753 MiB | 1,643 MiB |
| Output size | 73,070,204 bytes | 8,827,153 bytes |

The fixed build uses about 36% less elapsed time and 79% less peak memory on this
workload. Output semantics differ because the fixes remove redundant terrain and
repair encoding damage. These whole-process timings include the smaller output;
the separate memory commits were checked for identical terrain bytes before
cleanup policy changed.

[Raw samples and validation summary](terrain-storage-benchmark.json) include the
source commit, binary hashes, all measured samples, and output terrain hashes.
These are warm-cache measurements on this machine. They do not measure cold
startup or in-game visual quality.
