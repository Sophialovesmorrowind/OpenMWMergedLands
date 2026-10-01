#!/usr/bin/env python3
"""Compare frozen pre-sweep terrain algorithms with optimized production code.

Run from any directory: python3 ci/perf_terrain.py
Requires the selected Rust toolchain and the crate's locked dependencies.
The generated harness is optimized with rustc -O; Cargo supplies release deps.
Reports medians of seven alternating runs and checks complete normal-grid equality.
These are synthetic microbenchmarks, not end-to-end plugin merge timings.
"""

import argparse
import json
from pathlib import Path
import subprocess

SOURCE = r"""
#![allow(dead_code)]
use std::hint::black_box;
use std::time::Instant;

mod io {
    pub mod parsed_plugins {
        pub struct ParsedPlugin;
        impl ParsedPlugin {
            pub fn empty(_name: &str) -> Self { Self }
        }
    }
}
mod term_style {
    pub fn yellow(value: String) -> String { value }
}
mod land {
    #[path = "@ROOT@/src/land/grid_access.rs"] pub mod grid_access;
    #[path = "@ROOT@/src/land/terrain_map.rs"] pub mod terrain_map;
    #[path = "@ROOT@/src/land/conversions.rs"] pub mod conversions;
    #[path = "@ROOT@/src/land/height_map.rs"] pub mod height_map;
    #[path = "@ROOT@/src/land/textures.rs"] pub mod textures;
}
mod merge {
    #[path = "@ROOT@/src/merge/relative_to.rs"] pub mod relative_to;
    #[path = "@ROOT@/src/merge/relative_terrain_map.rs"] pub mod relative_terrain_map;
}
mod before_height_map {
    use crate::land::grid_access::{GridAccessor2D, Index2D, SquareGridIterator};
    use crate::land::terrain_map::{TerrainMap, Vec3};
    use num_traits::ToPrimitive;
    const HEIGHT_MAP_SCALE_FACTOR_F32: f32 = 8.0;
fn f32_to_i8_saturating(value: f32) -> i8 {
    if value.is_finite() {
        value
            .clamp(f32::from(i8::MIN), f32::from(i8::MAX))
            .to_i8()
            .expect("bounded f32 should convert to i8")
    } else {
        0
    }
}

pub fn calculate_vertex_normals_map<const T: usize>(
    height_map: &TerrainMap<i32, T>,
) -> TerrainMap<Vec3<i8>, T> {
    /// On the edge of the cell, reuse the last row or column.
    fn fix_coords<const T: usize>(coords: Index2D) -> Index2D {
        let x = if coords.x + 1 == T {
            coords.x - 1
        } else {
            coords.x
        };

        let y = if coords.y + 1 == T {
            coords.y - 1
        } else {
            coords.y
        };

        Index2D::new(x, y)
    }

    let mut terrain = [[Vec3::default(); T]; T];

    for coords in height_map.iter_grid() {
        let fixed_coords = fix_coords::<T>(coords);

        let coords_right = Index2D::new(fixed_coords.x + 1, fixed_coords.y);

        let h = height_map
            .get(fixed_coords)
            .to_f32()
            .expect("height value should convert to f32")
            / HEIGHT_MAP_SCALE_FACTOR_F32;
        let x1 = height_map
            .get(coords_right)
            .to_f32()
            .expect("height value should convert to f32")
            / HEIGHT_MAP_SCALE_FACTOR_F32;
        let v1 = Vec3 {
            x: 128f32 / HEIGHT_MAP_SCALE_FACTOR_F32,
            y: 0f32,
            z: x1 - h,
        };

        let coords_up = Index2D::new(fixed_coords.x, fixed_coords.y + 1);
        let y1 = height_map
            .get(coords_up)
            .to_f32()
            .expect("height value should convert to f32")
            / HEIGHT_MAP_SCALE_FACTOR_F32;
        let v2 = Vec3 {
            x: 0f32,
            y: 128f32 / HEIGHT_MAP_SCALE_FACTOR_F32,
            z: y1 - h,
        };

        let mut normal = Vec3 {
            x: v1.y * v2.z - v1.z * v2.y,
            y: v1.z * v2.x - v1.x * v2.z,
            z: v1.x * v2.y - v1.y * v2.x,
        };

        let squared: f32 = normal.x.powi(2) + normal.y.powi(2) + normal.z.powi(2);
        let hyp: f32 = squared.sqrt() / 127.0f32;

        normal.x /= hyp;
        normal.y /= hyp;
        normal.z /= hyp;

        *terrain.get_mut(coords) = Vec3::new(
            f32_to_i8_saturating(normal.x),
            f32_to_i8_saturating(normal.y),
            f32_to_i8_saturating(normal.z),
        );
    }

    terrain
}

}
mod before_relative {
    use crate::land::grid_access::{GridAccessor2D, SquareGridIterator};
    use crate::land::terrain_map::{TerrainMap, Vec3};
    use crate::before_height_map::calculate_vertex_normals_map;
    pub use crate::merge::relative_terrain_map::RelativeTerrainMap;
pub fn recompute_vertex_normals(
    height_map: &RelativeTerrainMap<i32, 65>,
    vertex_normals: Option<&RelativeTerrainMap<Vec3<i8>, 65>>,
) -> TerrainMap<Vec3<i8>, 65> {
    let height_map_abs = height_map.to_terrain();

    let mut recomputed_vertex_normals = calculate_vertex_normals_map(&height_map_abs);

    if let Some(vertex_normals) = vertex_normals {
        for coords in height_map.iter_grid() {
            if !height_map.has_difference(coords) {
                assert_eq!(vertex_normals.get_difference(coords), Vec3::default());
                let existing = vertex_normals.get_value(coords);
                if existing != Vec3::default() {
                    *recomputed_vertex_normals.get_mut(coords) = existing;
                }
            }
        }
    }

    recomputed_vertex_normals
}

}
mod before_textures {
    use crate::land::textures::IndexVTEX;
    use std::collections::HashMap;
    pub struct RemappedTextures { inner: HashMap<IndexVTEX, IndexVTEX> }
    impl RemappedTextures {
        pub fn from(used_ids: &[bool]) -> Self {
            assert!(used_ids[0]);
            assert!(used_ids.len() < u16::MAX as usize, "exceeded 65535 textures");
            let mut inner = HashMap::with_capacity(used_ids.len());
            for (new_id, (old_id, _)) in used_ids.iter().enumerate().filter(|(_, used)| **used).enumerate() {
                inner.insert(IndexVTEX::new(old_id.try_into().expect("safe")), IndexVTEX::new(new_id.try_into().expect("safe")));
            }
            Self { inner }
        }
        pub fn fallback_texture_index(&self) -> IndexVTEX {
            self.inner.values().copied().filter(|idx| *idx != IndexVTEX::default()).min().unwrap_or_default()
        }
    }
}
use land::grid_access::Index2D;
use merge::relative_terrain_map as after_relative;

fn elapsed(iterations: usize, mut f: impl FnMut()) -> f64 {
    let started = Instant::now();
    for _ in 0..iterations { f(); }
    started.elapsed().as_secs_f64() * 1e6 / iterations as f64
}
fn compare(label: &str, iterations: usize, mut before: impl FnMut(), mut after: impl FnMut()) {
    for _ in 0..32 { before(); after(); }
    let mut before_times = Vec::new();
    let mut after_times = Vec::new();
    for round in 0..7 {
        if round % 2 == 0 {
            before_times.push(elapsed(iterations, &mut before));
            after_times.push(elapsed(iterations, &mut after));
        } else {
            after_times.push(elapsed(iterations, &mut after));
            before_times.push(elapsed(iterations, &mut before));
        }
    }
    before_times.sort_by(f64::total_cmp);
    after_times.sort_by(f64::total_cmp);
    println!("{label}: before {:.6} us, after {:.6} us, {:.2}x", before_times[3], after_times[3], before_times[3] / after_times[3]);
}
fn main() {
    for percent in [0usize, 1, 50, 100] {
        let mut reference = [[0i32; 65]; 65];
        for (y, row) in reference.iter_mut().enumerate() {
            for (x, height) in row.iter_mut().enumerate() {
                *height = ((x * 37 + y * 61 + x * y) % 1024) as i32 * 8 - 4096;
            }
        }
        let existing = land::height_map::calculate_vertex_normals_map(&reference);
        let mut before_height = before_relative::RelativeTerrainMap::empty(reference);
        let mut after_height = after_relative::RelativeTerrainMap::empty(reference);
        for y in 0..65 {
            for x in 0..65 {
                if (x + y * 65) % 100 < percent {
                    let coords = Index2D::new(x, y);
                    before_height.set_value(coords, reference[y][x] + 128);
                    after_height.set_value(coords, reference[y][x] + 128);
                }
            }
        }
        let before_normals = before_relative::RelativeTerrainMap::empty(existing);
        let after_normals = after_relative::RelativeTerrainMap::empty(existing);
        assert_eq!(before_relative::recompute_vertex_normals(&before_height, Some(&before_normals)), after_relative::recompute_vertex_normals(&after_height, Some(&after_normals)));
        compare(&format!("normals {percent}% changed, 65x65"), 2000,
            || { black_box(before_relative::recompute_vertex_normals(black_box(&before_height), Some(black_box(&before_normals)))); },
            || { black_box(after_relative::recompute_vertex_normals(black_box(&after_height), Some(black_box(&after_normals)))); });
        if percent == 100 {
            assert_eq!(before_relative::recompute_vertex_normals(&before_height, None), after_relative::recompute_vertex_normals(&after_height, None));
            compare("normals without existing values, 65x65", 2000,
                || { black_box(before_relative::recompute_vertex_normals(black_box(&before_height), None)); },
                || { black_box(after_relative::recompute_vertex_normals(black_box(&after_height), None)); });
        }
    }
    for textures in [100usize, 1000, 10000] {
        let ids = vec![true; textures + 1];
        let before = before_textures::RemappedTextures::from(&ids);
        let after = land::textures::RemappedTextures::from(&ids);
        assert_eq!(before.fallback_texture_index().as_u16(), after.fallback_texture_index().as_u16());
        compare(&format!("fallback {textures} textures"), 4000,
            || { black_box(black_box(&before).fallback_texture_index()); },
            || { black_box(black_box(&after).fallback_texture_index()); });
    }
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", help="Override the repository's pinned Rust toolchain")
    parser.add_argument("--debug-deps", action="store_true", help="Reuse debug dependencies; harness is still optimized")
    parser.add_argument("--reuse-artifacts", action="store_true", help="Skip Cargo; requires the most recently built artifacts to use the same toolchain")
    args = parser.parse_args()
    toolchain_args = [f"+{args.toolchain}"] if args.toolchain else []
    root = Path(__file__).resolve().parents[1]
    perf = root / "target/perf_sweep/terrain"
    perf.mkdir(parents=True, exist_ok=True)
    deps = root / "target" / ("debug" if args.debug_deps else "release") / "deps"
    libraries = {}
    if args.reuse_artifacts:
        for name in ["anyhow", "bitflags", "const_default", "log", "num_traits", "tes3"]:
            candidates = list(deps.glob(f"lib{name}-*.rlib"))
            if not candidates:
                parser.error(f"Missing {name} artifacts in {deps}; omit --reuse-artifacts")
            libraries[name] = max(candidates, key=lambda path: path.stat().st_mtime)
    else:
        command = ["cargo", *toolchain_args, "build", "--locked", "--message-format=json"]
        if not args.debug_deps:
            command.append("--release")
        with subprocess.Popen(command, cwd=root, stdout=subprocess.PIPE, text=True) as process:
            for line in process.stdout:
                artifact = json.loads(line)
                if artifact.get("reason") == "compiler-artifact":
                    for filename in artifact.get("filenames", []):
                        if filename.endswith(".rlib"):
                            libraries[artifact["target"]["name"]] = Path(filename)
            if process.wait() != 0:
                raise SystemExit(process.returncode)
    source = perf / "bench.rs"
    escaped_root = str(root).replace("\\", "\\\\").replace('"', '\\"')
    source.write_text(SOURCE.replace("@ROOT@", escaped_root), encoding="utf-8")
    command = ["rustc", *toolchain_args, "--edition", "2024", "-C", "opt-level=3", "-L", f"dependency={deps}"]
    for name in ["anyhow", "bitflags", "const_default", "log", "num_traits", "tes3"]:
        command.extend(["--extern", f"{name}={libraries[name]}"])
    binary = perf / "bench"
    command.extend([str(source), "-o", str(binary)])
    subprocess.run(command, cwd=root, check=True)
    subprocess.run(["rustc", *toolchain_args, "--version"], cwd=root, check=True)
    print("Optimized synthetic benchmarks; median of 7 alternating runs; warmed fallback cache.", flush=True)
    subprocess.run([str(binary)], check=True)


if __name__ == "__main__":
    main()
