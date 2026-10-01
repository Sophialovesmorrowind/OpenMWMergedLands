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

fn differs_from_landscape<U: RelativeTo, const T: usize>(
    merged: Option<&RelativeTerrainMap<U, T>>,
    load: impl FnOnce() -> Option<TerrainMap<U, T>>,
) -> bool {
    let Some(merged) = merged else {
        return false;
    };

    let Some(loaded) = load() else {
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
    differs_from_landscape(merged.height_map.as_ref(), || {
        loaded.and_then(try_calculate_height_map)
    }) || differs_from_landscape(merged.vertex_normals.as_ref(), || {
        loaded.and_then(vertex_normals)
    }) || differs_from_landscape(merged.world_map_data.as_ref(), || {
        loaded.and_then(world_map_data)
    }) || differs_from_landscape(merged.vertex_colors.as_ref(), || {
        loaded.and_then(vertex_colors)
    }) || differs_from_landscape(merged.texture_indices.as_ref(), || {
        loaded.and_then(texture_indices)
    })
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

    let before = landmass.land.len();
    landmass.land.retain(|coords, land| {
        has_any_difference_from_loaded_landscape(land, loaded_landmass.land.get(coords))
    });
    let num_unmodified_from_loaded_landscape = before - landmass.land.len();

    debug!(
        "Removing {num_unmodified_from_loaded_landscape} LAND records unmodified from loaded landscape"
    );
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
    use super::differs_from_landscape;
    use crate::land::grid_access::Index2D;
    use crate::merge::relative_terrain_map::RelativeTerrainMap;
    use std::cell::Cell;

    #[test]
    fn absent_merged_map_does_not_load_unneeded_terrain() {
        let loads = Cell::new(0);
        assert!(!differs_from_landscape::<i32, 2>(None, || {
            loads.set(loads.get() + 1);
            Some([[1, 2], [3, 4]])
        }));
        assert_eq!(loads.get(), 0);
    }

    #[test]
    fn present_merged_map_differs_from_absent_loaded_terrain() {
        let merged = RelativeTerrainMap::<i32, 2>::empty([[1, 2], [3, 4]]);
        assert!(differs_from_landscape(Some(&merged), || None));
    }

    #[test]
    fn merged_map_equal_to_loaded_winner_has_no_difference() {
        let mut merged = RelativeTerrainMap::<i32, 2>::empty([[1, 2], [3, 4]]);
        merged.set_value(Index2D::new(1, 0), 20);
        assert!(!differs_from_landscape(Some(&merged), || Some([
            [1, 20],
            [3, 4]
        ])));
    }

    #[test]
    fn merged_map_different_from_loaded_winner_is_retained() {
        let mut merged = RelativeTerrainMap::<i32, 2>::empty([[1, 2], [3, 4]]);
        merged.set_value(Index2D::new(1, 0), 20);
        assert!(differs_from_landscape(Some(&merged), || Some([
            [1, 2],
            [3, 4]
        ])));
    }
}
