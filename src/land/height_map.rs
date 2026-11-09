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
    let get_pixel = |y: usize, x: usize| height_map[y][x] / HEIGHT_MAP_SCALE_FACTOR;
    let offset = get_pixel(0, 0).to_f32().expect("i32 fits in f32");
    ensure!(
        offset.to_i32() == Some(get_pixel(0, 0)),
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
            let difference = i64::from(get_pixel(y, x)) - i64::from(previous);
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
    let mut height = vertex_heights
        .offset
        .to_i32()
        .expect("vertex height offset should convert to i32");

    for y in 0..T {
        for x in 0..T {
            let coords = Index2D::new(x, y);
            height += i32::from(vertex_heights.data.get(coords));
            *grid_height.get_mut(coords) = height;
        }

        height = grid_height.get(Index2D::new(0, y));
    }

    for coords in grid_height.iter_grid() {
        *grid_height.get_mut(coords) *= HEIGHT_MAP_SCALE_FACTOR;
    }

    grid_height
}

/// Calculates the normal at one vertex, reusing the last row or column on cell edges.
pub(crate) fn calculate_vertex_normal<const T: usize>(
    height_map: &TerrainMap<i32, T>,
    coords: Index2D,
) -> Vec3<i8> {
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
