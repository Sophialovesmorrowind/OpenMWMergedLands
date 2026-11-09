use crate::LandmassDiff;
use crate::land::grid_access::Index2D;
use crate::land::terrain_map::Vec2;
use crate::repair::seam_detection::repair_landmass_seams;
use log::warn;
use std::collections::VecDeque;

/// Global coordinates identify all copies of a shared border or corner vertex.
#[derive(Clone, Copy)]
struct Vertex {
    x: i64,
    y: i64,
}

impl Vertex {
    fn new(cell: Vec2<i32>, x: usize, y: usize) -> Self {
        Self {
            x: i64::from(cell.x) * 64 + i64::try_from(x).expect("vertex index fits in i64"),
            y: i64::from(cell.y) * 64 + i64::try_from(y).expect("vertex index fits in i64"),
        }
    }

    fn copies(self) -> impl Iterator<Item = (Vec2<i32>, Index2D)> {
        let cell_x = self.x.div_euclid(64);
        let cell_y = self.y.div_euclid(64);
        let x = usize::try_from(self.x.rem_euclid(64)).expect("nonnegative remainder");
        let y = usize::try_from(self.y.rem_euclid(64)).expect("nonnegative remainder");

        [(cell_x, x), (cell_x - 1, 64)]
            .into_iter()
            .take(if x == 0 { 2 } else { 1 })
            .flat_map(move |(cx, vx)| {
                [(cell_y, y), (cell_y - 1, 64)]
                    .into_iter()
                    .take(if y == 0 { 2 } else { 1 })
                    .filter_map(move |(cy, vy)| {
                        Some((
                            Vec2::new(i32::try_from(cx).ok()?, i32::try_from(cy).ok()?),
                            Index2D::new(vx, vy),
                        ))
                    })
            })
    }

    fn lower(self, landmass: &mut LandmassDiff, height: i32) {
        for (cell, coords) in self.copies() {
            if let Some(map) = landmass
                .land
                .get_mut(&cell)
                .and_then(|land| land.height_map.as_mut())
            {
                map.set_value(coords, height);
            }
        }
    }
}

/// Lower the higher endpoint only when a VHGT edge exceeds its signed-byte bound.
fn constrain_edge(
    landmass: &mut LandmassDiff,
    cell: Vec2<i32>,
    from: Index2D,
    to: Index2D,
    pending: &mut VecDeque<Vertex>,
) {
    let map = landmass.land[&cell]
        .height_map
        .as_ref()
        .expect("height-bearing cell");
    let a = i64::from(map.get_value(from));
    let b = i64::from(map.get_value(to));

    let (vertex, height) = if b - a > 127 * 8 {
        (Vertex::new(cell, to.x, to.y), a + 127 * 8)
    } else if a - b > 128 * 8 {
        (Vertex::new(cell, from.x, from.y), b + 128 * 8)
    } else {
        return;
    };

    // The new value lies between two existing i32 heights.
    vertex.lower(landmass, i32::try_from(height).expect("bounded height"));
    pending.push_back(vertex);
}

/// Repair seams, quantize shared vertices together, and make stored VHGT steps representable.
///
/// Only horizontal edges and first-column vertical edges are stored in VHGT. Lowering
/// the high endpoint propagates through those edges until all constraints hold. Heights
/// decrease monotonically and remain bounded by the initial minimum, so no iteration
/// limit or encoder clamp is needed. Every copy of a shared vertex changes together.
pub fn repair_height_encoding(landmass: &mut LandmassDiff) {
    repair_landmass_seams(landmass);

    let cells: Vec<_> = landmass
        .sorted()
        .into_iter()
        .filter(|(_, land)| land.height_map.is_some())
        .map(|(cell, _)| *cell)
        .collect();

    for cell in &cells {
        let map = landmass
            .land
            .get_mut(cell)
            .and_then(|land| land.height_map.as_mut())
            .expect("height-bearing cell");
        for y in 0..65 {
            for x in 0..65 {
                let coords = Index2D::new(x, y);
                let height = map.get_value(coords);
                map.set_value(coords, height / 8 * 8);
            }
        }
    }

    let mut pending = VecDeque::new();
    for cell in cells {
        for y in 0..65 {
            if y > 0 {
                constrain_edge(
                    landmass,
                    cell,
                    Index2D::new(0, y - 1),
                    Index2D::new(0, y),
                    &mut pending,
                );
            }
            for x in 1..65 {
                constrain_edge(
                    landmass,
                    cell,
                    Index2D::new(x - 1, y),
                    Index2D::new(x, y),
                    &mut pending,
                );
            }
        }
    }

    let mut repairs = 0;
    while let Some(vertex) = pending.pop_front() {
        repairs += 1;
        for (cell, coords) in vertex.copies() {
            if landmass
                .land
                .get(&cell)
                .is_none_or(|land| land.height_map.is_none())
            {
                continue;
            }

            if coords.x > 0 {
                constrain_edge(
                    landmass,
                    cell,
                    Index2D::new(coords.x - 1, coords.y),
                    coords,
                    &mut pending,
                );
            }
            if coords.x < 64 {
                constrain_edge(
                    landmass,
                    cell,
                    coords,
                    Index2D::new(coords.x + 1, coords.y),
                    &mut pending,
                );
            }
            if coords.x == 0 {
                if coords.y > 0 {
                    constrain_edge(
                        landmass,
                        cell,
                        Index2D::new(0, coords.y - 1),
                        coords,
                        &mut pending,
                    );
                }
                if coords.y < 64 {
                    constrain_edge(
                        landmass,
                        cell,
                        coords,
                        Index2D::new(0, coords.y + 1),
                        &mut pending,
                    );
                }
            }
        }
    }

    if repairs > 0 {
        warn!("Applied {repairs} terrain adjustments to fit VHGT height deltas");
    }
}

#[cfg(test)]
mod tests {
    use super::repair_height_encoding;
    use crate::LandmassDiff;
    use crate::io::parsed_plugins::ParsedPlugin;
    use crate::land::grid_access::Index2D;
    use crate::land::height_map::{calculate_vertex_heights_tes3, try_calculate_height_map};
    use crate::land::landscape_diff::LandscapeDiff;
    use crate::land::terrain_map::{LandData, Vec2};
    use crate::merge::relative_terrain_map::RelativeTerrainMap;
    use crate::repair::seam_detection::repair_landmass_seams;
    use std::sync::Arc;
    use tes3::esp::{Landscape, LandscapeFlags};

    fn insert_cell(merged: &mut LandmassDiff, cell: Vec2<i32>, heights: &[[i32; 65]; 65]) {
        let source = Landscape {
            grid: (cell.x, cell.y),
            ..Landscape::default()
        };
        let mut land =
            LandscapeDiff::from_reference(merged.plugin.clone(), &source, LandData::all());
        land.height_map = Some(RelativeTerrainMap::empty(*heights));
        merged.land.insert(cell, land);
    }

    #[test]
    fn repairs_survive_encoding_at_edges_and_four_cell_corners() {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let mut merged = LandmassDiff::new(Arc::new(ParsedPlugin::empty("test.esp")));
                for cell in [
                    Vec2::new(-1, -1),
                    Vec2::new(0, -1),
                    Vec2::new(-1, 0),
                    Vec2::new(0, 0),
                ] {
                    let mut heights: Box<[[i32; 65]; 65]> = vec![[2603; 65]; 65]
                        .into_boxed_slice()
                        .try_into()
                        .expect("65 rows");
                    heights[32][0] = 0;
                    heights[0][0] = 0;
                    insert_cell(&mut merged, cell, &heights);
                }
                repair_landmass_seams(&mut merged);
                repair_height_encoding(&mut merged);
                assert_eq!(repair_landmass_seams(&mut merged), 0);

                for land in merged.land.values() {
                    let planned = land.height_map.as_ref().expect("heights").to_terrain();
                    let encoded = Landscape {
                        landscape_flags: LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS,
                        vertex_heights: Some(
                            calculate_vertex_heights_tes3(&planned).expect("representable"),
                        ),
                        ..Landscape::default()
                    };
                    assert_eq!(
                        try_calculate_height_map(&encoded).expect("decoded"),
                        planned
                    );
                }

                let once = merged.land[&Vec2::new(0, 0)]
                    .height_map
                    .as_ref()
                    .expect("heights")
                    .to_terrain();
                repair_height_encoding(&mut merged);
                assert_eq!(
                    merged.land[&Vec2::new(0, 0)]
                        .height_map
                        .as_ref()
                        .expect("heights")
                        .to_terrain(),
                    once
                );
            })
            .expect("spawn")
            .join()
            .expect("test");
    }

    #[test]
    fn leaves_valid_terrain_and_unstored_vertical_slopes_unchanged() {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let mut merged = LandmassDiff::new(Arc::new(ParsedPlugin::empty("test.esp")));
                let mut heights: Box<[[i32; 65]; 65]> = vec![[0; 65]; 65]
                    .into_boxed_slice()
                    .try_into()
                    .expect("65 rows");
                for (y, row) in heights.iter_mut().enumerate() {
                    if y % 2 == 0 {
                        for (x, value) in row.iter_mut().enumerate() {
                            *value = i32::try_from(x).expect("index") * 1016;
                        }
                    }
                }
                insert_cell(&mut merged, Vec2::new(0, 0), &heights);
                repair_height_encoding(&mut merged);
                let map = merged.land[&Vec2::new(0, 0)]
                    .height_map
                    .as_ref()
                    .expect("heights");
                assert_eq!(map.to_terrain(), *heights);
                assert_eq!(map.get_value(Index2D::new(64, 0)), 65024);
            })
            .expect("spawn")
            .join()
            .expect("test");
    }
}
