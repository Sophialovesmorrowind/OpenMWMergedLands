use crate::land::grid_access::{GridAccessor2D, GridIterator2D, Index2D, SquareGridIterator};
use crate::land::height_map::calculate_vertex_normals_map;
use crate::land::terrain_map::{TerrainMap, Vec3};
use crate::merge::relative_to::RelativeTo;
use const_default::ConstDefault;

#[derive(Clone)]
/// A [RelativeTerrainMap] is a set of 3 [TerrainMap] representing the original terrain,
/// any differences from that original terrain as a delta, and a boolean grid of values
/// where `true` indicates tht the difference from the original terrain is not zero.
pub struct RelativeTerrainMap<U: RelativeTo, const T: usize> {
    reference: TerrainMap<U, T>,
    relative: TerrainMap<<U as RelativeTo>::Delta, T>,
    has_difference: TerrainMap<bool, T>,
}

/// Type-erased struct for holding default [RelativeTerrainMap] constants.
pub struct DefaultRelativeTerrainMap {}

impl DefaultRelativeTerrainMap {
    /// A blank [RelativeTerrainMap] representing an empty height map.
    pub const HEIGHT_MAP: RelativeTerrainMap<i32, 65> = RelativeTerrainMap::default();

    /// A blank [RelativeTerrainMap] representing an empty vertex normals map.
    pub const VERTEX_NORMALS: RelativeTerrainMap<Vec3<i8>, 65> = RelativeTerrainMap::default();
}

impl<U: RelativeTo, const T: usize> SquareGridIterator<T> for RelativeTerrainMap<U, T> {
    fn iter_grid(&self) -> GridIterator2D<T, T> {
        Default::default()
    }
}

impl<U: RelativeTo, const T: usize> RelativeTerrainMap<U, T> {
    /// Creates a [RelativeTerrainMap] with defaults.
    pub const fn default() -> Self {
        let reference = [[<U as ConstDefault>::DEFAULT; T]; T];
        let relative = [[<<U as RelativeTo>::Delta as ConstDefault>::DEFAULT; T]; T];
        let has_difference = [[false; T]; T];
        Self {
            reference,
            relative,
            has_difference,
        }
    }

    /// Creates a [RelativeTerrainMap] from an existing reference [TerrainMap] without any
    /// differences from the reference.
    pub const fn empty(reference: TerrainMap<U, T>) -> Self {
        let relative = [[<<U as RelativeTo>::Delta as ConstDefault>::DEFAULT; T]; T];
        let has_difference = [[false; T]; T];
        Self {
            reference,
            relative,
            has_difference,
        }
    }

    /// Given a reference [TerrainMap] and a plugin [TerrainMap], calculates the
    /// [RelativeTerrainMap] of the plugin with respect to the reference.
    pub fn from_difference(
        reference: &TerrainMap<U, T>,
        plugin: &TerrainMap<U, T>,
    ) -> RelativeTerrainMap<U, T> {
        let mut output = RelativeTerrainMap::empty(*reference);

        for coords in reference.iter_grid() {
            output.set_value(coords, plugin.get(coords));
        }

        output
    }

    /// Read-only access to the `true` or `false` differences grid.
    pub fn differences(&self) -> &TerrainMap<bool, T> {
        &self.has_difference
    }

    /// Get the value at `coords` by adding the difference to the reference.
    pub fn get_value(&self, coords: Index2D) -> U {
        <U as RelativeTo>::add(self.reference.get(coords), self.relative.get(coords))
    }

    /// Set the value at `coords` by calculating a new difference from the reference.
    pub fn set_value(&mut self, coords: Index2D, value: U) {
        let difference = U::subtract(value, self.reference.get(coords));
        *self.relative.get_mut(coords) = difference;
        *self.has_difference.get_mut(coords) = difference != Default::default();
    }

    /// Get the difference at `coords`.
    pub fn get_difference(&self, coords: Index2D) -> <U as RelativeTo>::Delta {
        let delta = self.relative.get(coords);
        if delta == Default::default() {
            assert!(!self.has_difference.get(coords));
        } else {
            assert!(self.has_difference.get(coords));
        }
        delta
    }

    /// Set the difference at `coords`.
    pub fn set_difference(&mut self, coords: Index2D, difference: <U as RelativeTo>::Delta) {
        *self.relative.get_mut(coords) = difference;
        *self.has_difference.get_mut(coords) = difference != Default::default();
    }

    /// Returns `true` if there is a difference at `coords` with respect to the reference.
    pub fn has_difference(&self, coords: Index2D) -> bool {
        if self.has_difference.get(coords) {
            assert_ne!(self.relative.get(coords), Default::default());
            true
        } else {
            assert_eq!(self.relative.get(coords), Default::default());
            false
        }
    }

    /// Remove all differences.
    pub fn clean_all(&mut self) {
        for v in self.has_difference.as_flattened_mut() {
            *v = false;
        }

        for v in self.relative.as_flattened_mut() {
            *v = Default::default();
        }
    }

    /// Remove differences from all coordinates passed via `iter`.
    pub fn clean_some(&mut self, iter: impl Iterator<Item = Index2D>) {
        for coords in iter {
            *self.has_difference.get_mut(coords) = false;
            *self.relative.get_mut(coords) = Default::default();
        }
    }

    /// Create a new [TerrainMap] by adding the differences to the reference.
    /// This is the same as calling [RelativeTerrainMap::get_value] in a loop for each coordinate.
    pub fn to_terrain(&self) -> TerrainMap<U, T> {
        let mut terrain = [[Default::default(); T]; T];
        for coords in self.iter_grid() {
            *terrain.get_mut(coords) = self.get_value(coords);
        }
        terrain
    }
}

/// Types implementing [IsModified] report whether they differ from the reference.
pub trait IsModified {
    /// Returns `true` if there are differences from the reference.
    fn is_modified(&self) -> bool;
}

impl<U: RelativeTo, const T: usize> IsModified for RelativeTerrainMap<U, T> {
    fn is_modified(&self) -> bool {
        self.has_difference.as_flattened().contains(&true)
    }
}

/// An [Option] wrapping a [RelativeTerrainMap].
pub type OptionalTerrainMap<U, const T: usize> = Option<RelativeTerrainMap<U, T>>;

impl<U: RelativeTo, const T: usize> IsModified for OptionalTerrainMap<U, T> {
    fn is_modified(&self) -> bool {
        self.as_ref().map(|map| map.is_modified()).unwrap_or(false)
    }
}

/// Creates a [TerrainMap] representing the vertex normals of the `height_map` argument by
/// recalculating the vertex normals from the terrain. If the optional `vertex_normals`
/// is [Some], then the function will reuse those vertex normals on any unmodified coordinate
/// in the `height_map` instead of calculating new normals.
pub fn recompute_vertex_normals(
    height_map: &RelativeTerrainMap<i32, 65>,
    vertex_normals: Option<&RelativeTerrainMap<Vec3<i8>, 65>>,
) -> TerrainMap<Vec3<i8>, 65> {
    let height_map_abs = height_map.to_terrain();

    let mut recomputed_vertex_normals = calculate_vertex_normals_map(&height_map_abs);

    if let Some(vertex_normals) = vertex_normals {
        for coords in height_map.iter_grid() {
            if !height_map.has_difference(coords) {
                assert_eq!(vertex_normals.get_difference(coords), Default::default());
                *recomputed_vertex_normals.get_mut(coords) = vertex_normals.get_value(coords);
            }
        }
    }

    recomputed_vertex_normals
}

#[cfg(test)]
mod tests {
    use super::{recompute_vertex_normals, IsModified, OptionalTerrainMap, RelativeTerrainMap};
    use crate::land::grid_access::{GridAccessor2D, Index2D};
    use crate::land::terrain_map::{TerrainMap, Vec3};

    #[test]
    fn set_value_marks_and_gets_difference() {
        let reference = [[0i32; 2]; 2];
        let mut map = RelativeTerrainMap::<i32, 2>::empty(reference);

        let coords = Index2D::new(1, 0);
        map.set_value(coords, 5);

        assert!(map.has_difference(coords));
        assert_eq!(map.get_difference(coords), 5);
        assert_eq!(map.get_value(coords), 5);
        assert!(map.is_modified());
    }

    #[test]
    fn from_difference_roundtrips_plugin_values() {
        let reference = [[1i32, 2], [3, 4]];
        let plugin = [[1i32, 20], [30, 4]];
        let map = RelativeTerrainMap::<i32, 2>::from_difference(&reference, &plugin);

        assert_eq!(map.to_terrain(), plugin);
        assert!(!map.has_difference(Index2D::new(0, 0)));
        assert!(map.has_difference(Index2D::new(1, 0)));
        assert!(map.has_difference(Index2D::new(0, 1)));
    }

    #[test]
    fn clean_some_and_clean_all_clear_differences() {
        let mut map = RelativeTerrainMap::<i32, 2>::empty([[0i32; 2]; 2]);
        let c0 = Index2D::new(0, 0);
        let c1 = Index2D::new(1, 1);

        map.set_value(c0, 7);
        map.set_value(c1, 9);

        map.clean_some([c0].into_iter());
        assert!(!map.has_difference(c0));
        assert!(map.has_difference(c1));

        map.clean_all();
        assert!(!map.has_difference(c0));
        assert!(!map.has_difference(c1));
        assert!(!map.is_modified());
    }

    #[test]
    fn optional_is_modified_tracks_presence_and_inner_state() {
        let none_map: OptionalTerrainMap<i32, 2> = None;
        assert!(!none_map.is_modified());

        let mut some_map = RelativeTerrainMap::<i32, 2>::empty([[0i32; 2]; 2]);
        assert!(!Some(some_map.clone()).is_modified());

        some_map.set_value(Index2D::new(0, 0), 1);
        assert!(Some(some_map).is_modified());
    }

    #[test]
    fn recompute_vertex_normals_reuses_unmodified_existing_values() {
        let height_map = RelativeTerrainMap::<i32, 65>::empty([[0i32; 65]; 65]);
        let old_normals_reference: TerrainMap<Vec3<i8>, 65> = [[Vec3::new(7, -3, 2); 65]; 65];
        let old_normals = RelativeTerrainMap::<Vec3<i8>, 65>::empty(old_normals_reference);

        let recomputed = recompute_vertex_normals(&height_map, Some(&old_normals));
        assert_eq!(recomputed.get(Index2D::new(10, 10)), Vec3::new(7, -3, 2));
    }
}
