use crate::ParsedPlugin;
use crate::land::conversions::{
    coordinates, landscape_flags, texture_indices, vertex_colors, vertex_normals, world_map_data,
};
use crate::land::grid_access::{GridAccessor2D, SquareGridIterator};
use crate::land::height_map::{NormalSource, try_calculate_height_map};
use crate::land::terrain_map::{LandData, TerrainMap, Vec2, Vec3};
use crate::land::textures::IndexVTEX;
use crate::merge::relative_terrain_map::{IsModified, OptionalTerrainMap, RelativeTerrainMap};
use crate::merge::relative_to::RelativeTo;
use std::sync::Arc;
use tes3::esp::{Landscape, LandscapeFlags, ObjectFlags};

#[derive(Clone)]
/// A [`LandscapeDiff`] is all of the [`OptionalTerrainMap`] to describe the changes
/// between some reference [Landscape] and successive changes by plugin [Landscape].
pub struct LandscapeDiff {
    pub coords: Vec2<i32>,
    pub flags: ObjectFlags,
    pub height_map: OptionalTerrainMap<i32, 65>,
    pub vertex_normals: OptionalTerrainMap<Vec3<i8>, 65>,
    pub world_map_data: OptionalTerrainMap<u8, 9>,
    pub vertex_colors: OptionalTerrainMap<Vec3<u8>, 65>,
    pub texture_indices: OptionalTerrainMap<IndexVTEX, 16>,
    pub normal_sources: Vec<Arc<NormalSource>>,
    pub plugins: Vec<(Arc<ParsedPlugin>, LandData)>,
}

impl LandscapeDiff {
    /// Pair newly synthesized Resolve vectors with the geometry chosen at that stage.
    pub fn retain_resolved_normal_source(&mut self, old: &Self, new: &Self) {
        let (Some(heights), Some(normals), Some(old_normals), Some(new_normals)) = (
            self.height_map.as_ref(),
            self.vertex_normals.as_ref(),
            old.vertex_normals.as_ref(),
            new.vertex_normals.as_ref(),
        ) else {
            return;
        };

        let mut selected = Box::new([[false; 65]; 65]);
        let mut synthesized = false;
        for coords in normals.iter_grid() {
            let chosen = normals.get_value(coords);
            if old_normals.has_difference(coords)
                && new_normals.has_difference(coords)
                && chosen != old_normals.get_value(coords)
                && chosen != new_normals.get_value(coords)
            {
                *selected.get_mut(coords) = true;
                synthesized = true;
            }
        }
        if !synthesized {
            return;
        }

        // Other vertices in this cell must still validate against their original source.
        self.normal_sources.push(Arc::new(NormalSource {
            heights: heights.to_terrain(),
            normals: normals.to_terrain(),
            selected: Some(selected),
        }));
    }

    /// An available zero-valued layer still supplies data for later partial records.
    pub fn has_terrain_data(&self) -> bool {
        self.height_map.is_some()
            || self.vertex_normals.is_some()
            || self.world_map_data.is_some()
            || self.vertex_colors.is_some()
            || self.texture_indices.is_some()
    }

    /// Returns `true` if the [Landscape] is modified.
    pub fn is_modified(&self) -> bool {
        self.height_map.is_modified()
            || self.vertex_normals.is_modified()
            || self.world_map_data.is_modified()
            || self.vertex_colors.is_modified()
            || self.texture_indices.is_modified()
    }

    /// Returns [`LandData`] representing which portions of the [Landscape] are modified.
    pub fn modified_data(&self) -> LandData {
        let mut modified = LandData::default();

        if self.height_map.is_modified() {
            modified |= LandData::VERTEX_HEIGHTS;
        }

        if self.vertex_normals.is_modified() {
            modified |= LandData::VERTEX_NORMALS;
        }

        if self.world_map_data.is_modified() {
            modified |= LandData::WORLD_MAP;
        }

        if self.vertex_colors.is_modified() {
            modified |= LandData::VERTEX_COLORS;
        }

        if self.texture_indices.is_modified() {
            modified |= LandData::TEXTURES;
        }

        modified
    }

    /// Creates a new [`LandscapeDiff`] from the provided [Landscape] and allowed [`LandData`].
    pub fn from_reference(
        plugin: Arc<ParsedPlugin>,
        land: &Landscape,
        allowed_data: LandData,
    ) -> Self {
        let included_data = landscape_flags(land);

        let height_map = Self::calculate_reference(
            included_data.contains(LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS)
                && allowed_data.contains(LandData::VERTEX_HEIGHTS),
            try_calculate_height_map(land).as_ref(),
        );

        let vertex_normals = Self::calculate_reference(
            included_data.contains(LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS)
                && allowed_data.contains(LandData::VERTEX_NORMALS),
            vertex_normals(land).as_ref(),
        );

        let world_map_data = Self::calculate_reference(
            included_data.uses_world_map_data() && allowed_data.contains(LandData::WORLD_MAP),
            world_map_data(land).as_ref(),
        );

        let vertex_colors = Self::calculate_reference(
            included_data.contains(LandscapeFlags::USES_VERTEX_COLORS)
                && allowed_data.contains(LandData::VERTEX_COLORS),
            vertex_colors(land).as_ref(),
        );

        let texture_indices = Self::calculate_reference(
            included_data.contains(LandscapeFlags::USES_TEXTURES)
                && allowed_data.contains(LandData::TEXTURES),
            texture_indices(land).as_ref(),
        );

        Self {
            coords: coordinates(land),
            flags: land.flags,
            height_map,
            vertex_normals,
            world_map_data,
            vertex_colors,
            texture_indices,
            normal_sources: Vec::new(),
            plugins: vec![(plugin, LandData::default())],
        }
    }

    /// Creates a new [`LandscapeDiff`] from the provided `land` [Landscape] and allowed [`LandData`].
    /// The differences are computed by comparing `land` to the `reference` [Landscape].
    pub fn from_difference(
        land: &Landscape,
        reference: Option<&Landscape>,
        allowed_data: LandData,
    ) -> Self {
        let included_data = landscape_flags(land);

        let height_map = Self::calculate_differences(
            "height_map",
            included_data.contains(LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS)
                && allowed_data.contains(LandData::VERTEX_HEIGHTS),
            reference.and_then(try_calculate_height_map).as_ref(),
            try_calculate_height_map(land).as_ref(),
        );

        // Normal edits are independent authored data. Keep even an unchanged payload
        // available so a later whole-record override can carry it with the heights.
        let include_normals = included_data
            .contains(LandscapeFlags::USES_VERTEX_HEIGHTS_AND_NORMALS)
            && allowed_data.contains(LandData::VERTEX_NORMALS);
        let vertex_normals = if include_normals {
            let reference = reference.and_then(vertex_normals);
            vertex_normals(land).map(|normals| {
                let reference = reference.unwrap_or([[Vec3::default(); 65]; 65]);
                RelativeTerrainMap::from_difference(&reference, &normals)
            })
        } else {
            None
        };

        let world_map_data = Self::calculate_differences(
            "world_map_data",
            included_data.uses_world_map_data() && allowed_data.contains(LandData::WORLD_MAP),
            reference.and_then(world_map_data).as_ref(),
            world_map_data(land).as_ref(),
        );

        let vertex_colors = Self::calculate_differences(
            "vertex_colors",
            included_data.contains(LandscapeFlags::USES_VERTEX_COLORS)
                && allowed_data.contains(LandData::VERTEX_COLORS),
            reference.and_then(vertex_colors).as_ref(),
            vertex_colors(land).as_ref(),
        );

        let texture_indices = Self::calculate_differences(
            "texture_indices",
            included_data.contains(LandscapeFlags::USES_TEXTURES)
                && allowed_data.contains(LandData::TEXTURES),
            reference.and_then(texture_indices).as_ref(),
            texture_indices(land).as_ref(),
        );

        Self {
            coords: coordinates(land),
            flags: land.flags,
            height_map,
            vertex_normals,
            world_map_data,
            vertex_colors,
            texture_indices,
            normal_sources: Vec::new(),
            plugins: Vec::new(),
        }
    }

    /// Returns the modified terrain, keeping absent or unchanged layers optional.
    fn calculate_differences<U: RelativeTo, const T: usize>(
        _value: &str,
        should_include: bool,
        reference: Option<&TerrainMap<U, T>>,
        plugin: Option<&TerrainMap<U, T>>,
    ) -> OptionalTerrainMap<U, T> {
        if !should_include {
            return None;
        }

        let plugin = plugin?;
        let relative = if let Some(reference) = reference {
            RelativeTerrainMap::from_difference(reference, plugin)
        } else {
            let default = [[Default::default(); T]; T];
            RelativeTerrainMap::from_difference(&default, plugin)
        };

        Some(relative)
    }

    /// Returns [`RelativeTerrainMap::empty`] if `plugin` is [Some] and `should_include`.
    fn calculate_reference<U: RelativeTo, const T: usize>(
        should_include: bool,
        plugin: Option<&TerrainMap<U, T>>,
    ) -> OptionalTerrainMap<U, T> {
        if !should_include {
            return None;
        }

        plugin.map(|plugin| RelativeTerrainMap::empty(*plugin))
    }
}
