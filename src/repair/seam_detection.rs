use crate::LandmassDiff;
use crate::land::grid_access::Index2D;
use crate::land::landscape_diff::LandscapeDiff;
use crate::land::terrain_map::Vec2;
use crate::merge::relative_terrain_map::RelativeTerrainMap;
use log::{debug, trace};
use std::cmp::Ordering;
use std::collections::{HashSet, VecDeque};

/// Calculates new coordinates by adding the `offset` to the `coords`.
fn coords_with_offset(coords: Vec2<i32>, offset: [i32; 2]) -> Vec2<i32> {
    Vec2::new(coords.x + offset[0], coords.y + offset[1])
}

/// Given a `coords`, adds the four (N, W, S, E) adjacent sides to the
/// list of `possible_seams` if they are not already `visited`.
fn push_back_neighbors(
    possible_seams: &mut VecDeque<(Vec2<i32>, Vec2<i32>)>,
    visited: &mut HashSet<(Vec2<i32>, Vec2<i32>)>,
    coords: Vec2<i32>,
) {
    /// Sorts a pair of `Vec2` coordinates by `x` and then `y`.
    fn sort_pair(lhs: Vec2<i32>, rhs: Vec2<i32>) -> (Vec2<i32>, Vec2<i32>) {
        assert_ne!(lhs, rhs);
        match lhs.x.cmp(&rhs.x) {
            Ordering::Greater => (rhs, lhs),
            Ordering::Less => (lhs, rhs),
            Ordering::Equal => match lhs.y.cmp(&rhs.y) {
                Ordering::Greater => (rhs, lhs),
                Ordering::Less => (lhs, rhs),
                Ordering::Equal => unreachable!(),
            },
        }
    }

    for offset in [[-1, 0], [1, 0], [0, 1], [0, -1]] {
        let neighbor = coords_with_offset(coords, offset);
        let pair = sort_pair(coords, neighbor);
        if visited.insert(pair) {
            possible_seams.push_back(pair);
        }
    }
}

/// A corner of a landscape.
struct Corner {
    coords: Index2D,
    cell_offset: [i32; 2],
}

/// A set of 4 [Corner] relative to the current land.
/// Corner seams are repaired by inspecting all 4 vertices
/// meeting at the same corner.
struct CornerCase {
    corners: [Corner; 4],
}

/// Repairs corner seams by averaging their values together.
fn repair_corner_seams(
    merged: &mut LandmassDiff,
    coords: Vec2<i32>,
    num_seams_repaired: &mut usize,
) {
    let cases = [
        CornerCase {
            corners: [
                Corner {
                    coords: Index2D::new(0, 0),
                    cell_offset: [0, 0],
                },
                Corner {
                    coords: Index2D::new(0, 64),
                    cell_offset: [0, -1],
                },
                Corner {
                    coords: Index2D::new(64, 0),
                    cell_offset: [-1, 0],
                },
                Corner {
                    coords: Index2D::new(64, 64),
                    cell_offset: [-1, -1],
                },
            ],
        },
        CornerCase {
            corners: [
                Corner {
                    coords: Index2D::new(64, 0),
                    cell_offset: [0, 0],
                },
                Corner {
                    coords: Index2D::new(64, 64),
                    cell_offset: [0, -1],
                },
                Corner {
                    coords: Index2D::new(0, 0),
                    cell_offset: [1, 0],
                },
                Corner {
                    coords: Index2D::new(0, 64),
                    cell_offset: [1, -1],
                },
            ],
        },
        CornerCase {
            corners: [
                Corner {
                    coords: Index2D::new(64, 64),
                    cell_offset: [0, 0],
                },
                Corner {
                    coords: Index2D::new(64, 0),
                    cell_offset: [0, 1],
                },
                Corner {
                    coords: Index2D::new(0, 64),
                    cell_offset: [1, 0],
                },
                Corner {
                    coords: Index2D::new(0, 0),
                    cell_offset: [1, 1],
                },
            ],
        },
        CornerCase {
            corners: [
                Corner {
                    coords: Index2D::new(0, 64),
                    cell_offset: [0, 0],
                },
                Corner {
                    coords: Index2D::new(0, 0),
                    cell_offset: [0, 1],
                },
                Corner {
                    coords: Index2D::new(64, 64),
                    cell_offset: [-1, 0],
                },
                Corner {
                    coords: Index2D::new(64, 0),
                    cell_offset: [-1, 1],
                },
            ],
        },
    ];

    for case in &cases {
        let average = {
            let adjacent_values = case.corners.iter().map(|corner| {
                merged
                    .land
                    .get(&coords_with_offset(coords, corner.cell_offset))
                    .and_then(|land| land.height_map.as_ref())
                    .map(|height_map| height_map.get_value(corner.coords))
            });

            let mut average = 0i64;
            let mut num_values = 0i64;
            for value in adjacent_values.flatten() {
                average += i64::from(value);
                num_values += 1;
            }

            if num_values > 0 {
                Some((average / num_values) as i32)
            } else {
                None
            }
        };

        let Some(average) = average else {
            continue;
        };

        for corner in &case.corners {
            let Some(land) = merged
                .land
                .get_mut(&coords_with_offset(coords, corner.cell_offset))
            else {
                continue;
            };

            let Some(height_map) = land.height_map.as_mut() else {
                continue;
            };

            if height_map.get_value(corner.coords) != average {
                height_map.set_value(corner.coords, average);
                *num_seams_repaired += 1;
            }
        }
    }
}

/// Repairs a seam shared by two cells along a side.
fn try_repair_seam<const T: usize>(
    lhs_coord: Index2D,
    rhs_coord: Index2D,
    lhs_map: &mut RelativeTerrainMap<i32, T>,
    rhs_map: &mut RelativeTerrainMap<i32, T>,
    index: usize,
) -> usize {
    let lhs_value = lhs_map.get_value(lhs_coord);
    let rhs_value = rhs_map.get_value(rhs_coord);
    if lhs_value == rhs_value {
        0
    } else {
        assert!(
            index != 0 && index != 64,
            "corners should have been fixed first"
        );

        // TODO(dvd): #feature Should this use the ConflictResolver instead?
        let average = i32::midpoint(lhs_value, rhs_value);
        let lhs_diff = (average - lhs_value).abs();
        let rhs_diff = (average - rhs_value).abs();
        lhs_map.set_value(lhs_coord, average);
        rhs_map.set_value(rhs_coord, average);
        usize::try_from(lhs_diff.max(rhs_diff)).expect("difference should not be negative")
    }
}

/// Repairs landmass seams by a two-step algorithm. First, the algorithm repairs any
/// corner seams by averaging the values of all vertices shared by 4 cells. Then, the
/// algorithm will repair seams on the sides between cells by picking the average value
/// of both sides. For performance, only seams adjacent to coordinates in the `possible_seams`
/// field of the [`LandmassDiff`] will be visited.
pub fn repair_landmass_seams(merged: &mut LandmassDiff) -> usize {
    let mut possible_seams = VecDeque::new();
    let mut visited = HashSet::new();
    let mut repaired = HashSet::new();

    let mut num_seams_repaired = 0;

    let coords_to_visit: Vec<_> = merged.sorted().into_iter().map(|pair| *pair.0).collect();
    for coords in coords_to_visit {
        repair_corner_seams(merged, coords, &mut num_seams_repaired);
        push_back_neighbors(&mut possible_seams, &mut visited, coords);
    }

    while !possible_seams.is_empty() {
        let next = possible_seams.pop_front().expect("safe");

        let Some(mut rhs) = merged.land.remove(&next.1) else {
            continue;
        };

        let Some(lhs) = merged.land.get_mut(&next.0) else {
            merged.land.insert(next.1, rhs);
            continue;
        };

        let Some(lhs_height_map) = lhs.height_map.as_mut() else {
            merged.land.insert(next.1, rhs);
            continue;
        };

        let Some(rhs_height_map) = rhs.height_map.as_mut() else {
            merged.land.insert(next.1, rhs);
            continue;
        };

        let is_top_seam = if lhs.coords.x == rhs.coords.x {
            assert!(lhs.coords.y < rhs.coords.y);
            true
        } else {
            assert!(lhs.coords.x < rhs.coords.x);
            false
        };

        let mut seam_size = 0;
        let mut sum = 0;
        let mut max_delta = usize::MIN;
        let mut min_delta = usize::MAX;
        if is_top_seam {
            for x in 0..65 {
                let lhs_coord = Index2D::new(x, 64);
                let rhs_coord = Index2D::new(x, 0);
                let delta =
                    try_repair_seam(lhs_coord, rhs_coord, lhs_height_map, rhs_height_map, x);
                if delta > 0 {
                    num_seams_repaired += 1;
                    seam_size += 1;
                    sum += delta;
                    max_delta = max_delta.max(delta);
                    min_delta = min_delta.min(delta);
                }
            }
        } else {
            for y in 0..65 {
                let lhs_coord = Index2D::new(64, y);
                let rhs_coord = Index2D::new(0, y);
                let delta =
                    try_repair_seam(lhs_coord, rhs_coord, lhs_height_map, rhs_height_map, y);
                if delta > 0 {
                    num_seams_repaired += 1;
                    seam_size += 1;
                    sum += delta;
                    max_delta = max_delta.max(delta);
                    min_delta = min_delta.min(delta);
                }
            }
        }

        if let Some(average) = sum.checked_div(seam_size) {
            repaired.insert((next, seam_size, max_delta, min_delta, average));
        }

        merged.land.insert(next.1, rhs);
    }

    if num_seams_repaired > 0 {
        debug!("Repaired {num_seams_repaired} seams");
        let mut repaired: Vec<_> = repaired.iter().collect();
        repaired.sort_by_key(|a| std::cmp::Reverse(a.1));
        for seam in repaired {
            trace!(
                " - ({:>4}, {:>4}) | ({:>4}, {:>4}) | # of Seams = {:<3} | Max = {:<3} | Min = {:<3} | Avg = {}",
                seam.0.0.x, seam.0.0.y, seam.0.1.x, seam.0.1.y, seam.1, seam.2, seam.3, seam.4
            );
        }
    }

    for land in merged.land.values_mut() {
        if let Some(vertex_normals) = land.vertex_normals.as_ref() {
            land.vertex_normals = Some(LandscapeDiff::apply_mask(
                vertex_normals,
                land.height_map
                    .as_ref()
                    .map(RelativeTerrainMap::differences),
            ));
        }
    }

    num_seams_repaired
}

#[cfg(test)]
mod tests {
    use super::repair_landmass_seams;
    use crate::LandmassDiff;
    use crate::io::parsed_plugins::ParsedPlugin;
    use crate::land::grid_access::Index2D;
    use crate::land::landscape_diff::LandscapeDiff;
    use crate::land::terrain_map::Vec2;
    use crate::merge::relative_terrain_map::RelativeTerrainMap;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tes3::esp::ObjectFlags;

    fn run_with_large_stack<F>(f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(f)
            .expect("spawn test thread")
            .join()
            .expect("test thread panicked");
    }

    fn plugin() -> Arc<ParsedPlugin> {
        Arc::new(ParsedPlugin::empty("plugin.esp"))
    }

    fn landscape(coords: Vec2<i32>) -> LandscapeDiff {
        let height_map: Box<[[i32; 65]; 65]> = vec![[0i32; 65]; 65]
            .into_boxed_slice()
            .try_into()
            .expect("valid 65x65 height map");
        LandscapeDiff {
            coords,
            flags: ObjectFlags::default(),
            height_map: Some(RelativeTerrainMap::empty(*height_map)),
            vertex_normals: None,
            world_map_data: None,
            vertex_colors: None,
            texture_indices: None,
            plugins: vec![],
        }
    }

    fn empty_landmass_diff() -> LandmassDiff {
        LandmassDiff {
            plugin: plugin(),
            land: HashMap::new(),
        }
    }

    #[test]
    fn no_seams_means_no_repairs() {
        run_with_large_stack(|| {
            let mut merged = empty_landmass_diff();
            merged
                .land
                .insert(Vec2::new(0, 0), landscape(Vec2::new(0, 0)));
            merged
                .land
                .insert(Vec2::new(1, 0), landscape(Vec2::new(1, 0)));

            let repaired = repair_landmass_seams(&mut merged);
            assert_eq!(repaired, 0);
        });
    }

    #[test]
    fn repairs_single_side_seam_by_averaging() {
        run_with_large_stack(|| {
            let mut left = landscape(Vec2::new(0, 0));
            left.height_map
                .as_mut()
                .expect("height map")
                .set_value(Index2D::new(64, 10), 0);

            let mut right = landscape(Vec2::new(1, 0));
            right
                .height_map
                .as_mut()
                .expect("height map")
                .set_value(Index2D::new(0, 10), 10);

            let mut merged = empty_landmass_diff();
            merged.land.insert(Vec2::new(0, 0), left);
            merged.land.insert(Vec2::new(1, 0), right);

            let repaired = repair_landmass_seams(&mut merged);
            assert_eq!(repaired, 1);

            let left_value = merged
                .land
                .get(&Vec2::new(0, 0))
                .expect("left cell")
                .height_map
                .as_ref()
                .expect("height map")
                .get_value(Index2D::new(64, 10));
            let right_value = merged
                .land
                .get(&Vec2::new(1, 0))
                .expect("right cell")
                .height_map
                .as_ref()
                .expect("height map")
                .get_value(Index2D::new(0, 10));

            assert_eq!(left_value, 5);
            assert_eq!(right_value, 5);
        });
    }

    #[test]
    fn seam_repair_is_idempotent_after_first_pass() {
        run_with_large_stack(|| {
            let mut left = landscape(Vec2::new(0, 0));
            left.height_map
                .as_mut()
                .expect("height map")
                .set_value(Index2D::new(64, 10), 0);

            let mut right = landscape(Vec2::new(1, 0));
            right
                .height_map
                .as_mut()
                .expect("height map")
                .set_value(Index2D::new(0, 10), 10);

            let mut merged = empty_landmass_diff();
            merged.land.insert(Vec2::new(0, 0), left);
            merged.land.insert(Vec2::new(1, 0), right);

            let first = repair_landmass_seams(&mut merged);
            let second = repair_landmass_seams(&mut merged);

            assert!(first > 0);
            assert_eq!(second, 0);
        });
    }
}
