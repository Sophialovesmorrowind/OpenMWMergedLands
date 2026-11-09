use crate::land::grid_access::{GridAccessor2D, GridIterator2D, Index2D, SquareGridIterator};
use crate::land::terrain_map::TerrainMap;
use crate::merge::relative_to::RelativeTo;
use const_default::ConstDefault;

#[derive(Clone)]
/// A [`RelativeTerrainMap`] is a set of 3 [`TerrainMap`] representing the original terrain,
/// any differences from that original terrain as a delta, and a boolean grid of values
/// where `true` indicates tht the difference from the original terrain is not zero.
pub struct RelativeTerrainMap<U: RelativeTo, const T: usize> {
    // Keep absent maps small and move present maps without copying their grids.
    data: Box<RelativeTerrainData<U, T>>,
}

#[derive(Clone)]
struct RelativeTerrainData<U: RelativeTo, const T: usize> {
    reference: TerrainMap<U, T>,
    relative: TerrainMap<<U as RelativeTo>::Delta, T>,
    has_difference: TerrainMap<bool, T>,
}

impl<U: RelativeTo, const T: usize> SquareGridIterator<T> for RelativeTerrainMap<U, T> {
    fn iter_grid(&self) -> GridIterator2D<T, T> {
        GridIterator2D::default()
    }
}

impl<U: RelativeTo, const T: usize> RelativeTerrainMap<U, T> {
    /// Creates a [`RelativeTerrainMap`] from an existing reference [`TerrainMap`] without any
    /// differences from the reference.
    pub fn empty(reference: TerrainMap<U, T>) -> Self {
        let relative = [[<<U as RelativeTo>::Delta as ConstDefault>::DEFAULT; T]; T];
        let has_difference = [[false; T]; T];
        Self {
            data: Box::new(RelativeTerrainData {
                reference,
                relative,
                has_difference,
            }),
        }
    }

    /// Given a reference [`TerrainMap`] and a plugin [`TerrainMap`], calculates the
    /// [`RelativeTerrainMap`] of the plugin with respect to the reference.
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

    /// Get the value at `coords` by adding the difference to the reference.
    pub fn get_value(&self, coords: Index2D) -> U {
        <U as RelativeTo>::add(
            self.data.reference.get(coords),
            self.data.relative.get(coords),
        )
    }

    /// Set the value at `coords` by calculating a new difference from the reference.
    pub fn set_value(&mut self, coords: Index2D, value: U) {
        let difference = U::subtract(value, self.data.reference.get(coords));
        *self.data.relative.get_mut(coords) = difference;
        *self.data.has_difference.get_mut(coords) =
            difference != <U as RelativeTo>::Delta::default();
    }

    /// Returns `true` if there is a difference at `coords` with respect to the reference.
    pub fn has_difference(&self, coords: Index2D) -> bool {
        if self.data.has_difference.get(coords) {
            assert_ne!(
                self.data.relative.get(coords),
                <U as RelativeTo>::Delta::default()
            );
            true
        } else {
            assert_eq!(
                self.data.relative.get(coords),
                <U as RelativeTo>::Delta::default()
            );
            false
        }
    }

    /// Create a new [`TerrainMap`] by adding the differences to the reference.
    /// This is the same as calling [`RelativeTerrainMap::get_value`] in a loop for each coordinate.
    pub fn to_terrain(&self) -> TerrainMap<U, T> {
        let mut terrain = [[U::default(); T]; T];
        for coords in self.iter_grid() {
            *terrain.get_mut(coords) = self.get_value(coords);
        }
        terrain
    }
}

/// Types implementing [`IsModified`] report whether they differ from the reference.
pub trait IsModified {
    /// Returns `true` if there are differences from the reference.
    fn is_modified(&self) -> bool;
}

impl<U: RelativeTo, const T: usize> IsModified for RelativeTerrainMap<U, T> {
    fn is_modified(&self) -> bool {
        self.data.has_difference.as_flattened().contains(&true)
    }
}

/// An [Option] wrapping a [`RelativeTerrainMap`].
pub type OptionalTerrainMap<U, const T: usize> = Option<RelativeTerrainMap<U, T>>;

impl<U: RelativeTo, const T: usize> IsModified for OptionalTerrainMap<U, T> {
    fn is_modified(&self) -> bool {
        self.as_ref().is_some_and(IsModified::is_modified)
    }
}

#[cfg(test)]
mod tests {
    use super::{IsModified, OptionalTerrainMap, RelativeTerrainMap};
    use crate::land::grid_access::Index2D;

    #[test]
    fn absent_terrain_maps_do_not_reserve_inline_grids() {
        use std::mem::size_of;

        assert_eq!(size_of::<OptionalTerrainMap<i32, 65>>(), size_of::<usize>());
        assert_eq!(
            size_of::<OptionalTerrainMap<crate::land::terrain_map::Vec3<i8>, 65>>(),
            size_of::<usize>()
        );
        assert!(size_of::<crate::land::landscape_diff::LandscapeDiff>() < 256);
    }

    #[test]
    fn set_value_marks_and_gets_difference() {
        let reference = [[0i32; 2]; 2];
        let mut map = RelativeTerrainMap::<i32, 2>::empty(reference);

        let coords = Index2D::new(1, 0);
        map.set_value(coords, 5);

        assert!(map.has_difference(coords));
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
    fn optional_is_modified_tracks_presence_and_inner_state() {
        let none_map: OptionalTerrainMap<i32, 2> = None;
        assert!(!none_map.is_modified());

        let mut some_map = RelativeTerrainMap::<i32, 2>::empty([[0i32; 2]; 2]);
        assert!(!Some(some_map.clone()).is_modified());

        some_map.set_value(Index2D::new(0, 0), 1);
        assert!(Some(some_map).is_modified());
    }
}
