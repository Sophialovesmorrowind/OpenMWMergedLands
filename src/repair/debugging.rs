use crate::land::grid_access::SquareGridIterator;
use crate::land::landscape_diff::LandscapeDiff;
use crate::land::terrain_map::Vec3;
use crate::merge::conflict::{ConflictResolver, ConflictType};
use crate::merge::relative_terrain_map::RelativeTerrainMap;
use crate::merge::relative_to::RelativeTo;
use crate::LandmassDiff;

/// Adds any conflicts between the `lhs` [RelativeTerrainMap] and
/// the `rhs` [RelativeTerrainMap] to the `vertex_colors`.
pub fn add_vertex_colors<U: RelativeTo + ConflictResolver, const T: usize>(
    lhs: Option<&RelativeTerrainMap<U, T>>,
    rhs: Option<&RelativeTerrainMap<U, T>>,
    vertex_colors: Option<&mut RelativeTerrainMap<Vec3<u8>, T>>,
) {
    let Some(lhs) = lhs else {
        return;
    };

    let Some(rhs) = rhs else {
        return;
    };

    let Some(vertex_colors) = vertex_colors else {
        return;
    };

    let params = Default::default();

    const MAJOR_COLOR: Vec3<u8> = Vec3::new(255u8, 0, 0);
    const MINOR_COLOR: Vec3<u8> = Vec3::new(255u8, 255u8, 0);
    const MODIFIED_COLOR: Vec3<u8> = Vec3::new(0, 255u8, 0);
    const UNMODIFIED_COLOR: Vec3<u8> = Vec3::new(0, 0, 0);

    for coords in lhs.iter_grid() {
        let actual = lhs.get_value(coords);
        let expected = rhs.get_value(coords);
        let has_difference = rhs.has_difference(coords);

        let debug_color = if has_difference {
            match actual.average(expected, &params) {
                None => MODIFIED_COLOR,
                Some(ConflictType::Minor(_)) => MINOR_COLOR,
                Some(ConflictType::Major(_)) => MAJOR_COLOR,
            }
        } else {
            UNMODIFIED_COLOR
        };

        if debug_color == UNMODIFIED_COLOR {
            continue;
        }

        let current_color = vertex_colors.get_value(coords);
        let can_paint = (debug_color == MAJOR_COLOR)
            || (debug_color == MINOR_COLOR && current_color != MAJOR_COLOR);
        if can_paint {
            vertex_colors.set_value(coords, debug_color);
        }
    }
}

/// Add vertex colors to [LandscapeDiff] `reference` for any conflict found with `plugin`.
fn add_debug_vertex_colors_to_landscape(reference: &mut LandscapeDiff, plugin: &LandscapeDiff) {
    add_vertex_colors(
        reference.height_map.as_ref(),
        plugin.height_map.as_ref(),
        reference.vertex_colors.as_mut(),
    );
}

/// Add vertex colors to [LandmassDiff] `reference` for any conflict found with `plugin`.
pub fn add_debug_vertex_colors_to_landmass(reference: &mut LandmassDiff, plugin: &LandmassDiff) {
    for (coords, land) in plugin.sorted() {
        let merged_land = reference.land.get_mut(coords).expect("safe");
        add_debug_vertex_colors_to_landscape(merged_land, land);
    }
}

#[cfg(test)]
mod tests {
    use super::add_vertex_colors;
    use crate::land::grid_access::Index2D;
    use crate::land::terrain_map::Vec3;
    use crate::merge::relative_terrain_map::RelativeTerrainMap;

    #[test]
    fn add_vertex_colors_marks_major_conflicts_red() {
        let lhs = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);
        let mut rhs = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);
        rhs.set_value(Index2D::new(0, 0), 100);

        let mut vertex_colors =
            RelativeTerrainMap::<Vec3<u8>, 2>::empty([[Vec3::new(0, 0, 0); 2]; 2]);
        add_vertex_colors(Some(&lhs), Some(&rhs), Some(&mut vertex_colors));

        assert_eq!(
            vertex_colors.get_value(Index2D::new(0, 0)),
            Vec3::new(255, 0, 0)
        );
    }

    #[test]
    fn add_vertex_colors_skips_cells_without_rhs_difference() {
        let lhs = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);
        let rhs = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);

        let mut vertex_colors =
            RelativeTerrainMap::<Vec3<u8>, 2>::empty([[Vec3::new(0, 0, 0); 2]; 2]);
        add_vertex_colors(Some(&lhs), Some(&rhs), Some(&mut vertex_colors));

        assert_eq!(
            vertex_colors.get_value(Index2D::new(0, 0)),
            Vec3::new(0, 0, 0)
        );
        assert_eq!(
            vertex_colors.get_value(Index2D::new(1, 1)),
            Vec3::new(0, 0, 0)
        );
    }

    #[test]
    fn add_vertex_colors_does_not_downgrade_existing_major_to_minor() {
        let lhs_major = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);
        let mut rhs_major = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);
        rhs_major.set_value(Index2D::new(1, 1), 100);

        let mut vertex_colors =
            RelativeTerrainMap::<Vec3<u8>, 2>::empty([[Vec3::new(0, 0, 0); 2]; 2]);
        add_vertex_colors(Some(&lhs_major), Some(&rhs_major), Some(&mut vertex_colors));

        let lhs_minor = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);
        let mut rhs_minor = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);
        rhs_minor.set_value(Index2D::new(1, 1), 2);
        add_vertex_colors(Some(&lhs_minor), Some(&rhs_minor), Some(&mut vertex_colors));

        assert_eq!(
            vertex_colors.get_value(Index2D::new(1, 1)),
            Vec3::new(255, 0, 0)
        );
    }
}
