use crate::land::conversions::landscape_flags;
use crate::land::grid_access::{GridAccessor2D, Index2D, SquareGridIterator};
use crate::land::terrain_map::{TerrainMap, Vec3};
use crate::term_style::yellow;
use anyhow::{Context, Result, ensure};
use log::warn;
use num_traits::ToPrimitive;
use tes3::esp::{Landscape, LandscapeFlags, VertexHeights};

const CELL_SIZE: usize = 65;
const HEIGHT_MAP_SCALE_FACTOR: i32 = 8;
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

/// Calculates the vertex heights for the [`TerrainMap`] as a [`TerrainMap`] representing
/// a set of x-y differences and an [f32] for an offset. This is the tuple form of
/// the [`VertexHeights`] for any array size `T`.
fn calculate_vertex_heights<const T: usize>(
    height_map: &TerrainMap<i32, T>,
) -> Result<(f32, TerrainMap<i8, T>)> {
    let mut terrain = [[0; T]; T];
    let phase = height_map[0][0].rem_euclid(HEIGHT_MAP_SCALE_FACTOR);
    let get_pixel = |y: usize, x: usize| {
        (i64::from(height_map[y][x]) - i64::from(phase)) / i64::from(HEIGHT_MAP_SCALE_FACTOR)
    };
    let offset = height_map[0][0].to_f32().expect("i32 fits in f32") / HEIGHT_MAP_SCALE_FACTOR_F32;
    ensure!(
        (offset * HEIGHT_MAP_SCALE_FACTOR_F32).to_i32() == Some(height_map[0][0]),
        "VHGT offset loses height precision"
    );

    for (y, row) in terrain.iter_mut().enumerate() {
        for (x, delta) in row.iter_mut().enumerate() {
            let previous = if x > 0 {
                get_pixel(y, x - 1)
            } else if y > 0 {
                get_pixel(y - 1, 0)
            } else {
                get_pixel(0, 0)
            };
            ensure!(
                height_map[y][x].rem_euclid(HEIGHT_MAP_SCALE_FACTOR) == phase,
                "VHGT vertex ({x}, {y}) has a different eight-unit phase from its offset"
            );
            let difference = get_pixel(y, x) - previous;
            *delta = i8::try_from(difference).with_context(|| {
                format!("VHGT delta {difference} at vertex ({x}, {y}) exceeds -128..=127")
            })?;
        }
    }

    Ok((offset, terrain))
}

/// Creates [`VertexHeights`] from the `height_map` [`TerrainMap`].
/// Returns an error when a stored delta cannot represent the terrain.
pub fn calculate_vertex_heights_tes3(
    height_map: &TerrainMap<i32, CELL_SIZE>,
) -> Result<VertexHeights> {
    let (offset, terrain) = calculate_vertex_heights(height_map)?;
    Ok(VertexHeights {
        offset,
        data: Box::new(terrain),
    })
}

/// Creates [`TerrainMap`] from the `vertex_heights` [`VertexHeights`].
fn calculate_height_map<const T: usize>(vertex_heights: &VertexHeights) -> TerrainMap<i32, T> {
    let mut grid_height = [[0; T]; T];
    let mut row_height = vertex_heights.offset;
    for (y, row) in grid_height.iter_mut().enumerate() {
        row_height += f32::from(vertex_heights.data[y][0]);
        let mut height = row_height;
        for (x, value) in row.iter_mut().enumerate() {
            if x > 0 {
                height += f32::from(vertex_heights.data[y][x]);
            }
            *value = (height * HEIGHT_MAP_SCALE_FACTOR_F32)
                .to_i32()
                .expect("vertex height should fit in integer world units");
        }
    }

    grid_height
}

/// The triangle used by the normal calculation, including the reused edge samples.
fn normal_sample_coords<const T: usize>(coords: Index2D) -> [Index2D; 3] {
    let x = coords.x.min(T - 2);
    let y = coords.y.min(T - 2);
    [
        Index2D::new(x, y),
        Index2D::new(x + 1, y),
        Index2D::new(x, y + 1),
    ]
}

/// Calculates the normal at one vertex, reusing the last row or column on cell edges.
pub(crate) fn calculate_vertex_normal<const T: usize>(
    height_map: &TerrainMap<i32, T>,
    coords: Index2D,
) -> Vec3<i8> {
    let [fixed_coords, coords_right, coords_up] = normal_sample_coords::<T>(coords);

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

    Vec3::new(
        f32_to_i8_saturating(normal.x),
        f32_to_i8_saturating(normal.y),
        f32_to_i8_saturating(normal.z),
    )
}

/// Calculates the vertex normals for the [`TerrainMap`].
pub fn calculate_vertex_normals_map<const T: usize>(
    height_map: &TerrainMap<i32, T>,
) -> TerrainMap<Vec3<i8>, T> {
    let mut terrain = [[Vec3::default(); T]; T];
    for coords in height_map.iter_grid() {
        *terrain.get_mut(coords) = calculate_vertex_normal(height_map, coords);
    }
    terrain
}

/// Keep selected authored vectors wherever an input supplies them for the final geometry.
/// Source grids are loaded lazily: the winning LAND usually validates the whole cell.
/// Recompute only vectors whose source geometry no longer matches after merging or repair.
pub fn recompute_vertex_normals(
    heights: &TerrainMap<i32, 65>,
    authored: Option<&TerrainMap<Vec3<i8>, 65>>,
    sources: impl IntoIterator<Item = (TerrainMap<i32, 65>, TerrainMap<Vec3<i8>, 65>)>,
) -> TerrainMap<Vec3<i8>, 65> {
    let Some(authored) = authored else {
        return calculate_vertex_normals_map(heights);
    };

    let mut normals = *authored;
    let mut pending: Vec<_> = heights.iter_grid().collect();
    for (source_heights, source_normals) in sources {
        pending.retain(|coords| {
            normals.get(*coords) != source_normals.get(*coords)
                || normal_sample_coords::<65>(*coords)
                    .into_iter()
                    .any(|sample| heights.get(sample) != source_heights.get(sample))
        });
        if pending.is_empty() {
            return normals;
        }
    }

    for coords in pending {
        *normals.get_mut(coords) = calculate_vertex_normal(heights, coords);
    }

    normals
}

/// Calculate a [`TerrainMap`] of the [Landscape]'s height map by converting the
/// [`VertexHeights`] if present.
pub fn try_calculate_height_map(land: &Landscape) -> Option<TerrainMap<i32, 65>> {
    let included_data = landscape_flags(land);
    if !included_data.contains(LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS) {
        return None;
    }

    let Some(grid_height) = land.vertex_heights.as_ref().map(calculate_height_map) else {
        warn!(
            "{}",
            yellow(format!(
                "({:>4}, {:>4}) {:<15} | missing vertex_heights",
                land.grid.0, land.grid.1, "height_map"
            ))
        );
        return None;
    };

    // IMPORTANT(dvd): Sanity check that this conversion works.
    let vertex_heights_into =
        calculate_vertex_heights_tes3(&grid_height).expect("decoded VHGT has representable deltas");
    let grid_heights_return = calculate_height_map::<65>(&vertex_heights_into);
    for coords in grid_height.iter_grid() {
        let lhs = grid_height.get(coords);
        let rhs = grid_heights_return.get(coords);
        assert_eq!(
            lhs,
            rhs,
            "delta did not match at {:?} (possibly due to height? {} {})",
            coords,
            land.vertex_heights.as_ref().expect("safe").offset,
            vertex_heights_into.offset
        );
    }

    Some(grid_height)
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_height_map, calculate_vertex_heights, calculate_vertex_heights_tes3,
        try_calculate_height_map,
    };
    use tes3::esp::{Landscape, LandscapeFlags};

    fn flat_heights() -> Box<[[i32; 65]; 65]> {
        vec![[0; 65]; 65]
            .into_boxed_slice()
            .try_into()
            .expect("65 rows")
    }

    #[test]
    fn normals_preserve_the_winning_authored_values_for_unchanged_geometry() {
        let heights = flat_heights();
        let mut authored = [[super::Vec3::new(7, -3, 2); 65]; 65];
        authored[10][10] = super::Vec3::default();
        assert_eq!(
            super::recompute_vertex_normals(
                &heights,
                Some(&authored),
                std::iter::once((*heights, authored))
            ),
            authored
        );
    }

    #[test]
    fn normals_update_neighboring_samples_and_reused_cell_edges() {
        let loaded = flat_heights();
        let mut heights = loaded.clone();
        heights[32][32] = 256;
        heights[32][64] = 128;
        heights[64][32] = 128;
        let authored = [[super::Vec3::new(7, -3, 2); 65]; 65];
        let normals = super::recompute_vertex_normals(
            &heights,
            Some(&authored),
            std::iter::once((*loaded, authored)),
        );
        let generated = super::calculate_vertex_normals_map(&heights);

        for (x, y) in [
            (32, 32),
            (31, 32),
            (32, 31),
            (63, 32),
            (64, 32),
            (32, 63),
            (32, 64),
        ] {
            assert_eq!(normals[y][x], generated[y][x]);
            assert_ne!(normals[y][x], authored[y][x]);
        }
        assert_eq!(normals[10][10], authored[10][10]);
    }

    #[test]
    fn normals_validate_earlier_authored_sources_without_loading_unneeded_ones() {
        let heights = flat_heights();
        let authored = [[super::Vec3::new(64, 0, 110); 65]; 65];
        let winning = [[super::Vec3::new(-64, 0, 110); 65]; 65];
        let sources = std::iter::once((*heights, winning))
            .chain(std::iter::once((*heights, authored)))
            .chain(std::iter::from_fn(|| {
                panic!("later sources are unnecessary")
            }));
        assert_eq!(
            super::recompute_vertex_normals(&heights, Some(&authored), sources),
            authored
        );
    }

    #[test]
    fn normals_are_generated_when_the_winner_has_no_normal_payload() {
        let heights = flat_heights();
        assert_eq!(
            super::recompute_vertex_normals(&heights, None, []),
            super::calculate_vertex_normals_map(&heights)
        );
    }

    #[test]
    fn fractional_offsets_roundtrip_in_world_units() {
        for base in [4, -4, 1, -1] {
            let mut heights = flat_heights();
            for (y, row) in heights.iter_mut().enumerate() {
                for (x, value) in row.iter_mut().enumerate() {
                    *value = base + i32::try_from(x + y).expect("index") * 8;
                }
            }
            let encoded = calculate_vertex_heights_tes3(&heights).expect("fractional offset");
            assert_eq!(calculate_height_map::<65>(&encoded), *heights);
        }
    }

    #[test]
    fn vertex_heights_reject_unrepresentable_deltas_instead_of_clamping() {
        let mut map = [[0; 4]; 4];
        map[1][0] = -2600;
        let error = calculate_vertex_heights(&map).unwrap_err();
        assert!(error.to_string().contains("delta -325 at vertex (0, 1)"));

        map = [[0; 4]; 4];
        map[2][3] = 1024;
        assert!(calculate_vertex_heights(&map).is_err());
    }

    #[test]
    fn vertex_heights_accept_asymmetric_bounds_and_unstored_vertical_steps() {
        let map = [[0, 1016], [-1024, -8]];
        let (_, deltas) = calculate_vertex_heights(&map).expect("valid deltas");
        assert_eq!(deltas, [[0, 127], [-128, 127]]);

        let map = [[0, 1016], [1016, 2032]];
        assert!(calculate_vertex_heights(&map).is_ok());
    }

    #[test]
    fn vertex_heights_roundtrip_restores_original_height_map() {
        let mut map: Box<[[i32; 65]; 65]> = vec![[0i32; 65]; 65]
            .into_boxed_slice()
            .try_into()
            .expect("valid 65x65 map");
        for (y, row) in map.iter_mut().enumerate() {
            for (x, cell) in row.iter_mut().enumerate() {
                *cell = i32::try_from(x + y).expect("safe") * 8;
            }
        }

        let vertex_heights = calculate_vertex_heights_tes3(&map).expect("valid heights");
        let restored = calculate_height_map::<65>(&vertex_heights);
        assert_eq!(restored, *map);
    }

    #[test]
    fn calculate_vertex_heights_returns_expected_offset_for_uniform_map() {
        let map = [[80i32; 4]; 4];
        let (offset, terrain) = calculate_vertex_heights(&map).expect("valid heights");
        assert!((offset - 10.0).abs() < f32::EPSILON);
        assert!(terrain.into_iter().flatten().all(|value| value == 0));
    }

    #[test]
    fn try_calculate_height_map_requires_vertex_height_flag() {
        let land = Landscape {
            landscape_flags: LandscapeFlags::USES_VERTEX_COLORS,
            ..Landscape::default()
        };
        assert!(try_calculate_height_map(&land).is_none());
    }

    #[test]
    fn try_calculate_height_map_reads_vertex_heights_when_flagged() {
        let map: Box<[[i32; 65]; 65]> = vec![[16i32; 65]; 65]
            .into_boxed_slice()
            .try_into()
            .expect("valid 65x65 map");
        let land = Landscape {
            landscape_flags: LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS,
            vertex_heights: Some(calculate_vertex_heights_tes3(&map).expect("valid heights")),
            ..Landscape::default()
        };

        let parsed = try_calculate_height_map(&land).expect("height map should be present");
        assert_eq!(parsed, *map);
    }
}
