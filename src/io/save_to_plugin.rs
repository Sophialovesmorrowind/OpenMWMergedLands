use crate::cli::SortOrder;
use crate::io::meta_schema::{MergeSettings, MetaType, PluginMeta, VersionedPluginMeta};
use crate::io::parsed_plugins::{meta_name, sort_plugins, DataDirs, ParsedPlugin, ParsedPlugins};
use crate::land::conversions::convert_terrain_map;
use crate::land::height_map::calculate_vertex_heights_tes3;
use crate::land::landscape_diff::LandscapeDiff;
use crate::land::terrain_map::Vec3;
use crate::land::textures::{IndexVTEX, KnownTextures, RemappedTextures};
use crate::merge::cells::ModifiedCell;
use crate::merge::relative_terrain_map::{recompute_vertex_normals, DefaultRelativeTerrainMap};
use crate::{Landmass, LandmassDiff, Vec2};
use anyhow::{anyhow, Context, Result};
use log::{debug, trace, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tes3::esp::{
    FixedString, Header, Landscape, LandscapeFlags, Plugin, TES3Object, TextureIndices,
    VertexColors, VertexNormals, WorldMapData,
};

/// Converts a [`LandscapeDiff`] to a [Landscape].
/// The [`RemappedTextures`] is used to update any texture indices.
fn convert_landscape_diff_to_landscape(
    landscape: &LandscapeDiff,
    remapped_textures: &RemappedTextures,
) -> Landscape {
    let mut new_landscape = Landscape::default();

    assert!(!landscape.plugins.is_empty());
    for (plugin, modified_data) in &landscape.plugins {
        if modified_data.is_empty() {
            continue;
        }

        trace!(
            "({:>4}, {:>4}) | {:<50} | {:?}",
            landscape.coords.x,
            landscape.coords.y,
            plugin.name,
            modified_data
        );
    }

    new_landscape.flags = landscape.flags;
    new_landscape.grid = (landscape.coords.x, landscape.coords.y);
    new_landscape.landscape_flags = LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS
        | LandscapeFlags::USES_VERTEX_COLORS
        | LandscapeFlags::USES_TEXTURES
        | LandscapeFlags::UNKNOWN;

    let height_map = landscape
        .height_map
        .as_ref()
        .unwrap_or(&DefaultRelativeTerrainMap::HEIGHT_MAP);
    let vertex_normals = landscape
        .vertex_normals
        .as_ref()
        .unwrap_or(&DefaultRelativeTerrainMap::VERTEX_NORMALS);

    new_landscape.vertex_heights = Some(calculate_vertex_heights_tes3(&height_map.to_terrain()));

    new_landscape.vertex_normals = Some(VertexNormals {
        data: Box::new(convert_terrain_map(
            &recompute_vertex_normals(height_map, Some(vertex_normals)),
            Vec3::into,
        )),
    });

    if let Some(vertex_colors) = landscape.vertex_colors.as_ref() {
        new_landscape.vertex_colors = Some(VertexColors {
            data: Box::new(convert_terrain_map(&vertex_colors.to_terrain(), Vec3::into)),
        });
    }

    if let Some(texture_indices) = landscape.texture_indices.as_ref() {
        let mut texture_indices = texture_indices.to_terrain();
        let mut invalid_texture_indices = 0usize;
        let mut first_invalid_texture_index = None;

        for idx in texture_indices.as_flattened_mut() {
            if let Some(remapped) = remapped_textures.try_remapped_index(*idx) {
                *idx = remapped;
            } else {
                invalid_texture_indices += 1;
                first_invalid_texture_index.get_or_insert(idx.as_u16());
                *idx = IndexVTEX::default();
            }
        }

        if invalid_texture_indices > 0 {
            warn!(
                "({:>4}, {:>4}) | Replaced {} invalid texture indices while converting LAND for output (first VTEX index = {})",
                landscape.coords.x,
                landscape.coords.y,
                invalid_texture_indices,
                first_invalid_texture_index.expect("invalid index count implies first invalid index")
            );
        }

        new_landscape.texture_indices = Some(TextureIndices {
            data: Box::new(convert_terrain_map(
                &texture_indices,
                crate::land::textures::IndexVTEX::as_u16,
            )),
        });
    }

    if let Some(world_map_data) = landscape.world_map_data.as_ref() {
        new_landscape.world_map_data = Some(WorldMapData {
            data: Box::new(world_map_data.to_terrain()),
        });
    }

    new_landscape
}

/// Converts a [`LandmassDiff`] to a [Landmass].
/// The [`RemappedTextures`] is used to update any texture indices.
pub fn convert_landmass_diff_to_landmass(
    landmass: &LandmassDiff,
    remapped_textures: &RemappedTextures,
) -> Landmass {
    let mut new_landmass = Landmass::new(landmass.plugin.clone());

    for (coords, land) in landmass.sorted() {
        let landscape = convert_landscape_diff_to_landscape(land, remapped_textures);
        let last_plugin = land.plugins.last().expect("safe").clone().0;
        new_landmass.insert_land(*coords, &last_plugin, &landscape);
    }

    new_landmass
}

/// Creates a master record for plugin `name` by appending the size
/// of the file in bytes to the tuple `(name, file_size)`.
fn to_master_record(data_dirs: &DataDirs, name: String) -> (String, u64) {
    let file_size = data_dirs
        .resolve(&name)
        .and_then(|path| fs::metadata(path).ok().map(|metadata| metadata.len()))
        .unwrap_or(0);
    (name, file_size)
}

/// Saves the [Landmass] with [`KnownTextures`].
pub fn save_plugin(
    data_dirs: &DataDirs,
    output_file_dir: &Path,
    output_name: &str,
    sort_order: SortOrder,
    landmass: &Landmass,
    known_textures: &KnownTextures,
    cells: Option<&HashMap<Vec2<i32>, ModifiedCell>>,
) -> Result<()> {
    ParsedPlugins::check_dir_exists(output_file_dir)
        .with_context(|| anyhow!("Unable to save file {output_name}"))?;

    let mut plugin = Plugin::new();

    debug!("Determining plugin dependencies");

    let masters: Option<Vec<(String, u64)>> = {
        let mut dependencies = HashSet::new();

        let mut add_dependency =
            |dependency: &Arc<ParsedPlugin>| dependencies.insert(dependency.name.clone());

        // Add plugins that contribute textures.
        for texture in known_textures.sorted() {
            add_dependency(&texture.plugin);
        }

        // Add plugins used for the land.
        for plugin in landmass.plugins.values() {
            add_dependency(plugin);
        }

        if let Some(cells) = cells {
            // Add plugins that modified cells.
            for (coords, _) in landmass.sorted() {
                let cell = cells.get(coords).with_context(|| {
                    anyhow!("Could not find CELL record for LAND with coordinates {coords:?}")
                })?;

                let plugin = cell.plugins.last().expect("safe");
                if add_dependency(plugin) {
                    trace!(
                        "({:>4}, {:>4})   | {:<50} | {}",
                        coords.x,
                        coords.y,
                        plugin.name,
                        if cell.inner.id.is_empty() {
                            cell.inner.region.as_deref().unwrap_or("")
                        } else {
                            cell.inner.id.as_str()
                        }
                    );
                }
            }
        }

        let mut masters: Vec<_> = dependencies.drain().collect();

        sort_plugins(data_dirs, &mut masters, sort_order)
            .with_context(|| anyhow!("Unknown load order for {output_name} dependencies"))?;

        Some(
            masters
                .into_iter()
                .map(|plugin| to_master_record(data_dirs, plugin))
                .collect(),
        )
    };

    for (idx, master) in masters.as_ref().expect("safe").iter().enumerate() {
        trace!("Master  | {:>4} | {:<50} | {:>10}", idx, master.0, master.1);
    }

    let generated_time = SystemTime::now().duration_since(UNIX_EPOCH).map_or_else(
        |_| "unknown".into(),
        |duration| format!("{} UTC", duration.as_secs()),
    );

    let description = format!(
        "Merges landscape changes inside of cells. Place at end of load order. Generated at {generated_time}."
    );

    let author = "Merged Lands by DVD".to_string();

    let header = Header {
        author: FixedString(author),
        description: FixedString(description.clone()),
        masters,
        ..Default::default()
    };

    debug!("Saving 1 TES3 record");
    plugin.objects.push(TES3Object::Header(header));

    debug!("Saving {} LTEX records", known_textures.len());
    for known_texture in known_textures.sorted() {
        trace!(
            "Texture | {:>4} | {:<30} | {}",
            known_texture.index().as_u16(),
            known_texture.id(),
            known_texture.plugin.name
        );
        plugin.objects.push(TES3Object::LandscapeTexture(
            known_texture.clone_landscape_texture(),
        ));
    }

    if cells.is_some() {
        debug!("Saving {} CELL and LAND records", landmass.land.len());
    } else {
        debug!("Saving {} LAND records", landmass.land.len());
    }

    for (coords, land) in landmass.sorted() {
        if let Some(cells) = cells {
            let cell = cells.get(coords).expect("safe");
            plugin.objects.push(TES3Object::Cell(cell.inner.clone()));
        }

        plugin.objects.push(TES3Object::Landscape(land.clone()));
    }

    let meta_name = meta_name(output_name);
    let merged_meta: PathBuf = [output_file_dir, Path::new(&meta_name)].iter().collect();

    let meta = VersionedPluginMeta::V0(PluginMeta {
        meta_type: MetaType::MergedLands,
        height_map: MergeSettings::default(),
        vertex_colors: MergeSettings::default(),
        texture_indices: MergeSettings::default(),
        world_map_data: MergeSettings::default(),
    });

    trace!("Saving meta file {meta_name}");
    fs::write(merged_meta, toml::to_string(&meta).expect("safe"))
        .with_context(|| anyhow!("Unable to save plugin meta {meta_name}"))?;

    let merged_filepath: PathBuf = [output_file_dir, Path::new(output_name)].iter().collect();

    trace!("Saving file {output_name}");
    plugin
        .save_path(&merged_filepath)
        .with_context(|| anyhow!("Unable to save plugin {output_name}"))?;

    trace!(" - Description: {description}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        convert_landmass_diff_to_landmass, convert_landscape_diff_to_landscape, save_plugin,
        to_master_record,
    };
    use crate::cli::SortOrder;
    use crate::io::parsed_plugins::{DataDirs, ParsedPlugin};
    use crate::land::grid_access::Index2D;
    use crate::land::height_map::{calculate_vertex_heights_tes3, try_calculate_height_map};
    use crate::land::landscape_diff::LandscapeDiff;
    use crate::land::terrain_map::{LandData, Vec2};
    use crate::land::textures::{IndexVTEX, KnownTextures, RemappedTextures};
    use crate::merge::cells::ModifiedCell;
    use crate::merge::relative_terrain_map::RelativeTerrainMap;
    use crate::{Landmass, LandmassDiff};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tes3::esp::{
        Cell, Landscape, LandscapeFlags, ObjectFlags, Plugin, TES3Object, VertexNormals,
    };

    fn plugin(name: &str) -> Arc<ParsedPlugin> {
        Arc::new(ParsedPlugin::empty(name))
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = format!(
            "{}_{}_{}",
            name,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before unix epoch")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn fixture_land(coords: (i32, i32), base_height: i32) -> Landscape {
        let mut heights: Box<[[i32; 65]; 65]> = vec![[base_height; 65]; 65]
            .into_boxed_slice()
            .try_into()
            .expect("valid 65x65 height map");
        heights[1][1] = base_height + 16;

        Landscape {
            flags: ObjectFlags::default(),
            grid: coords,
            landscape_flags: LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS
                | LandscapeFlags::UNKNOWN,
            vertex_heights: Some(calculate_vertex_heights_tes3(&heights)),
            vertex_normals: Some(VertexNormals::default()),
            ..Landscape::default()
        }
    }

    fn fixture_cell(coords: (i32, i32), id: &str) -> Cell {
        let mut cell = Cell::default();
        cell.data.grid = coords;
        cell.id = id.to_string();
        cell
    }

    fn load_plugin(path: &std::path::Path) -> Plugin {
        let mut plugin = Plugin::new();
        plugin.load_path(path).expect("load plugin");
        plugin
    }

    fn object_counts(plugin: &Plugin) -> (usize, usize, usize) {
        let mut cells = 0;
        let mut lands = 0;
        let mut ltex = 0;
        for object in &plugin.objects {
            match object {
                TES3Object::Cell(_) => cells += 1,
                TES3Object::Landscape(_) => lands += 1,
                TES3Object::LandscapeTexture(_) => ltex += 1,
                _ => {}
            }
        }
        (cells, lands, ltex)
    }

    fn header_masters(plugin: &Plugin) -> Vec<(String, u64)> {
        plugin
            .objects
            .iter()
            .find_map(|object| match object {
                TES3Object::Header(header) => header.masters.clone(),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn landscape_diff_with_texture(
        plugin: Arc<ParsedPlugin>,
        coords: Vec2<i32>,
        texture_index: u16,
    ) -> LandscapeDiff {
        let mut texture_indices =
            RelativeTerrainMap::<IndexVTEX, 16>::empty([[IndexVTEX::new(0); 16]; 16]);
        texture_indices.set_value(Index2D::new(1, 1), IndexVTEX::new(texture_index));
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
            texture_indices: Some(texture_indices),
            plugins: vec![(plugin, LandData::default())],
        }
    }

    #[test]
    fn to_master_record_returns_file_size_for_existing_plugin() {
        let unique = format!(
            "merged_lands_master_size_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before unix epoch")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("create temp dir");
        let plugin_name = "size_test.esp";
        let file_path = dir.join(plugin_name);
        fs::write(&file_path, [1u8, 2, 3, 4, 5]).expect("write plugin file");

        let record = to_master_record(&DataDirs::single(dir.clone()), plugin_name.to_string());
        assert_eq!(record.0, plugin_name);
        assert_eq!(record.1, 5);

        fs::remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn to_master_record_returns_zero_when_plugin_is_missing() {
        let dir = std::env::temp_dir().join("merged_lands_missing_master");
        fs::create_dir_all(&dir).expect("create temp dir");

        let record = to_master_record(&DataDirs::single(dir.clone()), "missing.esp".to_string());
        assert_eq!(record.1, 0);

        fs::remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn convert_landscape_diff_remaps_invalid_texture_to_default() {
        let plugin = plugin("plugin.esp");
        let landscape = landscape_diff_with_texture(plugin, Vec2::new(0, 0), 1);
        let remapped = RemappedTextures::from(&[true]);

        let converted = convert_landscape_diff_to_landscape(&landscape, &remapped);
        let indices = converted
            .texture_indices
            .expect("texture indices should be present");

        assert_eq!(indices.data[1][1], 0);
        assert!(converted.vertex_heights.is_some());
        assert!(converted.vertex_normals.is_some());
        assert!(converted
            .landscape_flags
            .contains(LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS));
    }

    #[test]
    fn convert_landmass_diff_preserves_land_coordinates() {
        let land_plugin = plugin("land.esp");
        let diff_plugin = plugin("diff.esp");

        let mut landmass_diff = LandmassDiff {
            plugin: diff_plugin,
            land: HashMap::new(),
        };

        let coords = Vec2::new(4, 8);
        let landscape = landscape_diff_with_texture(land_plugin, coords, 0);
        landmass_diff.land.insert(coords, landscape);

        let remapped = RemappedTextures::from(&[true]);
        let converted = convert_landmass_diff_to_landmass(&landmass_diff, &remapped);

        assert!(converted.land.contains_key(&coords));
        assert!(converted.plugins.contains_key(&coords));
    }

    #[test]
    fn save_plugin_writes_non_empty_land_and_cell_content() {
        let root = unique_temp_dir("save_plugin_non_empty");
        let data_dir = root.join("Data Files");
        let output_dir = root.join("Output");
        fs::create_dir_all(&data_dir).expect("create data dir");
        fs::create_dir_all(&output_dir).expect("create output dir");

        let source_name = "Source.esp";
        fs::write(data_dir.join(source_name), [1u8, 2, 3]).expect("write source plugin file");

        let source_plugin = plugin(source_name);
        let coords = Vec2::new(7, 9);

        let mut landmass = Landmass::new(source_plugin.clone());
        let land = fixture_land((coords.x, coords.y), 120);
        landmass.insert_land(coords, &source_plugin, &land);

        let mut cells = HashMap::new();
        cells.insert(
            coords,
            ModifiedCell {
                inner: fixture_cell((coords.x, coords.y), "Output Cell"),
                plugins: vec![source_plugin.clone()],
            },
        );

        save_plugin(
            &DataDirs::single(data_dir.clone()),
            &output_dir,
            "MergedOut.esp",
            SortOrder::None,
            &landmass,
            &KnownTextures::new(),
            Some(&cells),
        )
        .expect("save should succeed");

        let output = load_plugin(&output_dir.join("MergedOut.esp"));
        let (cell_count, land_count, ltex_count) = object_counts(&output);
        assert_eq!(cell_count, 1);
        assert_eq!(land_count, 1);
        assert_eq!(ltex_count, 0);

        let masters = header_masters(&output);
        assert_eq!(masters.len(), 1);
        assert_eq!(masters[0].0, source_name);
        assert_eq!(masters[0].1, 3);

        let out_land = output
            .objects
            .iter()
            .find_map(|object| match object {
                TES3Object::Landscape(land) => Some(land),
                _ => None,
            })
            .expect("LAND should exist");
        assert_eq!(out_land.grid, (7, 9));
        assert!(out_land.vertex_heights.is_some());
        assert!(out_land.vertex_normals.is_some());
        assert!(try_calculate_height_map(out_land).is_some());

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn save_plugin_remove_cells_mode_keeps_same_land_count() {
        let root = unique_temp_dir("save_plugin_remove_cells");
        let data_dir = root.join("Data Files");
        let output_dir = root.join("Output");
        fs::create_dir_all(&data_dir).expect("create data dir");
        fs::create_dir_all(&output_dir).expect("create output dir");

        let source_name = "Source.esp";
        fs::write(data_dir.join(source_name), [4u8, 5, 6]).expect("write source plugin file");
        let source_plugin = plugin(source_name);
        let coords = Vec2::new(3, 5);

        let mut landmass = Landmass::new(source_plugin.clone());
        landmass.insert_land(
            coords,
            &source_plugin,
            &fixture_land((coords.x, coords.y), 200),
        );

        let mut cells = HashMap::new();
        cells.insert(
            coords,
            ModifiedCell {
                inner: fixture_cell((coords.x, coords.y), "Output Cell"),
                plugins: vec![source_plugin.clone()],
            },
        );

        save_plugin(
            &DataDirs::single(data_dir.clone()),
            &output_dir,
            "WithCells.esp",
            SortOrder::None,
            &landmass,
            &KnownTextures::new(),
            Some(&cells),
        )
        .expect("save with cells");

        save_plugin(
            &DataDirs::single(data_dir),
            &output_dir,
            "NoCells.esp",
            SortOrder::None,
            &landmass,
            &KnownTextures::new(),
            None,
        )
        .expect("save without cells");

        let with_cells = load_plugin(&output_dir.join("WithCells.esp"));
        let no_cells = load_plugin(&output_dir.join("NoCells.esp"));

        let with_counts = object_counts(&with_cells);
        let no_counts = object_counts(&no_cells);
        assert_eq!(with_counts.1, 1);
        assert_eq!(no_counts.1, 1);
        assert_eq!(with_counts.0, 1);
        assert_eq!(no_counts.0, 0);

        let with_masters = header_masters(&with_cells);
        let no_masters = header_masters(&no_cells);
        assert_eq!(with_masters, no_masters);
        assert_eq!(with_masters.len(), 1);
        assert_eq!(with_masters[0].0, source_name);
        assert_eq!(with_masters[0].1, 3);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn save_plugin_non_empty_output_is_deterministic() {
        let root = unique_temp_dir("save_plugin_deterministic");
        let data_dir = root.join("Data Files");
        let output_dir = root.join("Output");
        fs::create_dir_all(&data_dir).expect("create data dir");
        fs::create_dir_all(&output_dir).expect("create output dir");

        let source_name = "Source.esp";
        fs::write(data_dir.join(source_name), [9u8, 8, 7]).expect("write source plugin file");
        let source_plugin = plugin(source_name);

        let mut landmass = Landmass::new(source_plugin.clone());
        let coords = Vec2::new(12, 14);
        landmass.insert_land(
            coords,
            &source_plugin,
            &fixture_land((coords.x, coords.y), 88),
        );

        save_plugin(
            &DataDirs::single(data_dir.clone()),
            &output_dir,
            "DetA.esp",
            SortOrder::None,
            &landmass,
            &KnownTextures::new(),
            None,
        )
        .expect("save det A");
        save_plugin(
            &DataDirs::single(data_dir),
            &output_dir,
            "DetB.esp",
            SortOrder::None,
            &landmass,
            &KnownTextures::new(),
            None,
        )
        .expect("save det B");

        let a = load_plugin(&output_dir.join("DetA.esp"));
        let b = load_plugin(&output_dir.join("DetB.esp"));
        assert_eq!(object_counts(&a), object_counts(&b));
        assert_eq!(header_masters(&a), header_masters(&b));

        let a_land = a
            .objects
            .iter()
            .find_map(|object| match object {
                TES3Object::Landscape(land) => Some(land),
                _ => None,
            })
            .expect("LAND in A");
        let b_land = b
            .objects
            .iter()
            .find_map(|object| match object {
                TES3Object::Landscape(land) => Some(land),
                _ => None,
            })
            .expect("LAND in B");

        assert_eq!(a_land.grid, b_land.grid);
        let a_heights = try_calculate_height_map(a_land).expect("heights A");
        let b_heights = try_calculate_height_map(b_land).expect("heights B");
        assert_eq!(a_heights[1][1], b_heights[1][1]);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }
}
