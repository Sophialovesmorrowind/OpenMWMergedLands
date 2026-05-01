use crate::io::parsed_plugins::{ParsedPlugin, ParsedPlugins};
use crate::land::conversions::{texture_indices, vertex_colors, vertex_normals, world_map_data};
use crate::land::grid_access::{GridAccessor2D, SquareGridIterator};
use crate::land::height_map::try_calculate_height_map;
use crate::land::landscape_diff::LandscapeDiff;
use crate::land::terrain_map::TerrainMap;
use crate::land::textures::{KnownTextures, RemappedTextures};
use crate::merge::relative_terrain_map::RelativeTerrainMap;
use crate::merge::relative_to::RelativeTo;
use crate::repair::seam_detection::repair_landmass_seams;
use crate::{Landmass, LandmassDiff};
use log::{debug, warn};
use std::sync::Arc;
use tes3::esp::{Landscape, LandscapeTexture};

#[cfg(test)]
fn has_difference<U: RelativeTo, const T: usize>(
    lhs: Option<&RelativeTerrainMap<U, T>>,
    rhs: Option<&RelativeTerrainMap<U, T>>,
) -> bool {
    let Some(lhs) = lhs else {
        return false;
    };

    let Some(rhs) = rhs else {
        return false;
    };

    for coords in lhs.iter_grid() {
        let actual = lhs.get_value(coords);
        let expected = rhs.get_value(coords);
        if actual != expected {
            return true;
        }
    }

    false
}

fn differs_from_landscape<U: RelativeTo, const T: usize>(
    merged: Option<&RelativeTerrainMap<U, T>>,
    loaded: Option<&TerrainMap<U, T>>,
) -> bool {
    let Some(merged) = merged else {
        return false;
    };

    let Some(loaded) = loaded else {
        return true;
    };

    for coords in merged.iter_grid() {
        if merged.get_value(coords) != loaded.get(coords) {
            return true;
        }
    }

    false
}

fn has_any_difference_from_loaded_landscape(
    merged: &LandscapeDiff,
    loaded: Option<&Landscape>,
) -> bool {
    let height_map = loaded.and_then(try_calculate_height_map);
    let vertex_normals = loaded.and_then(vertex_normals);
    let world_map_data = loaded.and_then(world_map_data);
    let vertex_colors = loaded.and_then(vertex_colors);
    let texture_indices = loaded.and_then(texture_indices);

    differs_from_landscape(merged.height_map.as_ref(), height_map.as_ref())
        || differs_from_landscape(merged.vertex_normals.as_ref(), vertex_normals.as_ref())
        || differs_from_landscape(merged.world_map_data.as_ref(), world_map_data.as_ref())
        || differs_from_landscape(merged.vertex_colors.as_ref(), vertex_colors.as_ref())
        || differs_from_landscape(merged.texture_indices.as_ref(), texture_indices.as_ref())
}

fn update_known_textures(plugin: &Arc<ParsedPlugin>, known_textures: &mut KnownTextures) {
    for texture in plugin.records.objects_of_type::<LandscapeTexture>() {
        known_textures.update_texture(plugin, texture);
    }
}

/// Remove any [`crate::LandscapeDiff`] from the [`LandmassDiff`] that would not change the final
/// loaded LAND state.
pub fn clean_landmass_diff(landmass: &mut LandmassDiff, loaded_landmass: &Landmass) {
    assert_eq!(repair_landmass_seams(landmass), 0);

    let mut unmodified = Vec::new();
    let mut num_unmodified_from_reference = 0;
    let mut num_unmodified_from_loaded_landscape = 0;

    for (coords, land) in &mut landmass.land {
        if !land.is_modified() {
            unmodified.push(*coords);
            num_unmodified_from_reference += 1;
            continue;
        }

        if !has_any_difference_from_loaded_landscape(land, loaded_landmass.land.get(coords)) {
            unmodified.push(*coords);
            num_unmodified_from_loaded_landscape += 1;
        }
    }

    debug!("Removing {num_unmodified_from_reference} LAND records unmodified from reference");

    debug!(
        "Removing {num_unmodified_from_loaded_landscape} LAND records unmodified from loaded landscape"
    );

    for coords in unmodified.drain(..) {
        landmass.land.remove(&coords);
    }
}

/// Remove any unused [`crate::land::textures::KnownTexture`] from the [`KnownTextures`].
/// Returns [`RemappedTextures`] for anything that was not removed.
pub fn clean_known_textures(
    parsed_plugins: &ParsedPlugins,
    landmass: &LandmassDiff,
    known_textures: &mut KnownTextures,
) -> RemappedTextures {
    assert!(
        known_textures.len() < u16::MAX as usize,
        "exceeded maximum number of textures"
    );

    // Make sure all LTEX records have the correct filenames.

    for master in &parsed_plugins.masters {
        update_known_textures(master, known_textures);
    }

    for plugin in &parsed_plugins.plugins {
        update_known_textures(plugin, known_textures);
    }

    // Determine all LTEX records in use in the final MergedLands.esp.
    // Reserve extra texture index for the default 0th texture.

    let mut used_ids = vec![false; known_textures.len() + 1];
    used_ids[0] = true; // Assume the default texture is in use.
    for (_, land) in landmass.sorted() {
        let Some(texture_indices) = land.texture_indices.as_ref() else {
            continue;
        };

        let mut invalid_texture_indices = 0usize;
        let mut first_invalid_texture_index = None;
        for coords in texture_indices.iter_grid() {
            let key = texture_indices.get_value(coords);
            let idx = usize::from(key.as_u16());
            if idx < used_ids.len() {
                used_ids[idx] = true;
            } else {
                invalid_texture_indices += 1;
                first_invalid_texture_index.get_or_insert(key.as_u16());
            }
        }

        if invalid_texture_indices > 0 {
            warn!(
                "({:>4}, {:>4}) | {} invalid texture indices in merged LAND (first VTEX index = {}) will be replaced with a fallback texture",
                land.coords.x,
                land.coords.y,
                invalid_texture_indices,
                first_invalid_texture_index
                    .expect("invalid index count implies first invalid index")
            );
        }
    }

    // Determine the remapping needed for LTEX records.

    let remapped_textures = RemappedTextures::from(&used_ids);
    let num_removed_ids = known_textures.remove_unused(&remapped_textures);

    debug!("Removing {num_removed_ids} unused LTEX records");
    debug!("Remapping {} LTEX records", known_textures.len());

    remapped_textures
}

#[cfg(test)]
mod tests {
    use super::has_difference;
    use crate::land::grid_access::Index2D;
    use crate::merge::relative_terrain_map::RelativeTerrainMap;

    #[test]
    fn has_difference_is_false_when_maps_are_identical() {
        let lhs = RelativeTerrainMap::<i32, 2>::empty([[1, 2], [3, 4]]);
        let rhs = RelativeTerrainMap::<i32, 2>::empty([[1, 2], [3, 4]]);

        assert!(!has_difference(Some(&lhs), Some(&rhs)));
    }

    #[test]
    fn has_difference_is_true_when_any_cell_differs() {
        let mut lhs = RelativeTerrainMap::<i32, 2>::empty([[1, 2], [3, 4]]);
        let rhs = RelativeTerrainMap::<i32, 2>::empty([[1, 2], [3, 4]]);

        lhs.set_value(Index2D::new(1, 0), 20);
        assert!(has_difference(Some(&lhs), Some(&rhs)));
    }

    #[test]
    fn has_difference_returns_false_when_missing_input_map() {
        let lhs = RelativeTerrainMap::<i32, 2>::empty([[1, 2], [3, 4]]);
        assert!(!has_difference(Some(&lhs), None));
        assert!(!has_difference::<i32, 2>(None, Some(&lhs)));
    }
}
