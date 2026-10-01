"""Measure production repair/cleaning code using synthetic in-memory LAND records.

Build dependencies with `cargo build --release --locked` first. To compare revisions,
run this script with --source pointing at each checkout and --dependencies pointing
at the same target/release/deps directory. Copies and fixture construction happen
outside timed regions; no game configuration or plugin files are read.
"""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


HARNESS = r"""
fn main() {
    use std::hint::black_box;
    use std::time::Instant;
    use crate::merge::relative_terrain_map::RelativeTerrainMap;
    use crate::land::grid_access::Index2D;
    use crate::land::terrain_map::Vec3;
    let args: Vec<_> = std::env::args().collect();
    let samples: usize = args[1].parse().unwrap();
    let side: i32 = args[2].parse().unwrap();
    println!("LandscapeDiff bytes: {}", std::mem::size_of::<LandscapeDiff>());
    for (name, sparse, change) in [
        ("dense-clean", false, false),
        ("dense-seams", false, true),
        ("sparse-clean", true, false),
    ] {
        let mut template = LandmassDiff::new(Arc::new(ParsedPlugin::empty("perf.esp")));
        for x in 0..side {
            for y in 0..side {
                let spacing = if sparse {3} else {1};
                let coords = Vec2::new(x * spacing, y * spacing);
                let height = if change {(x+y)%3*80} else {0};
                let mut normals = RelativeTerrainMap::empty([[Vec3::new(0i8, 0, 127);65];65]);
                normals.set_value(Index2D::new(10, 10), Vec3::new(1, 2, 3));
                normals.set_value(Index2D::new(64, 10), Vec3::new(4, 5, 6));
                template.land.insert(coords, LandscapeDiff {
                    coords, flags: ObjectFlags::default(),
                    height_map: Some(RelativeTerrainMap::empty([[height;65];65])),
                    vertex_normals: Some(normals), vertex_colors: None,
                    world_map_data: None, texture_indices: None, plugins: vec![],
                });
            }
        }
        let mut times = vec![];
        let mut count = 0;
        let mut checksum = 0u64;
        for _ in 0..samples {
            let mut input = LandmassDiff {
                plugin: template.plugin.clone(), land: template.land.clone(),
            };
            let start = Instant::now();
            count = black_box(crate::repair::seam_detection::repair_landmass_seams(black_box(&mut input)));
            times.push(start.elapsed().as_secs_f64()*1000.);
            checksum = 0;
            for (coords, land) in &input.land {
                let heights = land.height_map.as_ref().unwrap();
                let normals = land.vertex_normals.as_ref().unwrap();
                let mut cell = (coords.x as u64).wrapping_mul(31).wrapping_add(coords.y as u64);
                for y in 0..65 {
                    for x in 0..65 {
                        let vertex = Index2D::new(x,y);
                        let normal = normals.get_value(vertex);
                        let delta = normals.get_difference(vertex);
                        for value in [
                            heights.get_value(vertex), heights.get_difference(vertex),
                            i32::from(normal.x), i32::from(normal.y), i32::from(normal.z),
                            delta.x, delta.y, delta.z,
                        ] {
                            cell = cell.wrapping_mul(1099511628211).wrapping_add(value as u64);
                        }
                    }
                }
                checksum ^= cell;
            }
            black_box(checksum);
        }
        times.sort_by(f64::total_cmp);
        println!("{} cells={} median_ms={:.3} min_ms={:.3} repairs={} checksum={:016x}",
            name, template.land.len(), times[samples/2], times[0], count, checksum);
    }
    let loaded = Landscape {
        landscape_flags: LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS
            | LandscapeFlags::USES_VERTEX_COLORS | LandscapeFlags::USES_TEXTURES,
        vertex_heights: Some(crate::land::height_map::calculate_vertex_heights_tes3(&[[0;65];65])),
        vertex_normals: Some(tes3::esp::VertexNormals::default()),
        vertex_colors: Some(tes3::esp::VertexColors::default()),
        texture_indices: Some(tes3::esp::TextureIndices::default()),
        world_map_data: Some(tes3::esp::WorldMapData::default()),
        ..Landscape::default()
    };
    let merged = LandscapeDiff {
        coords: Vec2::new(0,0), flags: ObjectFlags::default(),
        height_map: None, vertex_normals: None, vertex_colors: None, world_map_data: None,
        texture_indices: Some(RelativeTerrainMap::empty([[crate::land::textures::IndexVTEX::new(0);16];16])),
        plugins: vec![],
    };
    let mut times = vec![];
    let mut differs = false;
    for _ in 0..samples {
        let start = Instant::now();
        for _ in 0..10000 {
            differs = black_box(crate::repair::cleaning::perf_compare(
                black_box(&merged), black_box(Some(&loaded)),
            ));
        }
        times.push(start.elapsed().as_secs_f64()*1000.);
    }
    times.sort_by(f64::total_cmp);
    println!("loaded-texture-only comparisons=10000 median_ms={:.3} min_ms={:.3} differs={}",
        times[samples/2], times[0], differs);
}
"""


def main():
    repository = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=repository)
    parser.add_argument("--dependencies", type=Path, default=repository / "target/release/deps")
    parser.add_argument("--samples", type=int, default=11)
    parser.add_argument("--side", type=int, default=24)
    args = parser.parse_args()
    if args.samples < 1 or args.side < 1:
        parser.error("--samples and --side must be positive")
    source = args.source.resolve()
    dependencies = args.dependencies.resolve()
    manifest = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))
    command = ["rustc", "--edition=2024", "-O", "-Awarnings", "-L", f"dependency={dependencies}"]
    names = set(manifest["dependencies"])
    for target in manifest.get("target", {}).values():
        names.update(target.get("dependencies", {}))
    for name in sorted(names):
        crate = name.replace("-", "_")
        candidates = list(dependencies.glob(f"lib{crate}-*.rlib"))
        if not candidates and name in manifest["dependencies"]:
            parser.error(f"missing built dependency {name}; run cargo build --release --locked")
        if candidates:
            artifact = max(candidates, key=lambda path: path.stat().st_mtime_ns)
            command.extend(["--extern", f"{crate}={artifact}"])
    with tempfile.TemporaryDirectory(prefix="merged-lands-repair-perf-") as directory:
        work = Path(directory)
        shutil.copytree(source / "src", work / "src")
        shutil.copytree(source / "assets", work / "assets")
        main_source = work / "src/main.rs"
        application = main_source.read_text(encoding="utf-8")
        if "fn main() {" not in application:
            parser.error("cannot locate application main function")
        main_source.write_text(application.replace("fn main() {", "fn application_main() {", 1) + HARNESS,
                               encoding="utf-8")
        cleaning = work / "src/repair/cleaning.rs"
        with cleaning.open("a", encoding="utf-8") as bridge:
            bridge.write("\npub fn perf_compare(merged: &LandscapeDiff, loaded: Option<&Landscape>) -> bool { "
                         "has_any_difference_from_loaded_landscape(merged, loaded) }\n")
        executable = work / ("perf.exe" if os.name == "nt" else "perf")
        environment = os.environ.copy()
        environment["CARGO_PKG_VERSION"] = manifest["package"]["version"]
        subprocess.run(command + [str(main_source), "-o", str(executable)], env=environment, check=True)
        subprocess.run([str(executable), str(args.samples), str(args.side)], check=True)


if __name__ == "__main__":
    main()
