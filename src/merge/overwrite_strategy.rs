use crate::ParsedPlugin;
use crate::land::grid_access::SquareGridIterator;
use crate::land::terrain_map::Vec2;
use crate::merge::conflict::ConflictResolver;
use crate::merge::merge_strategy::MergeStrategy;
use crate::merge::relative_terrain_map::RelativeTerrainMap;
use crate::merge::relative_to::RelativeTo;

#[derive(Default)]
/// Implements [`MergeStrategy`] to overwrite any conflicts with the newest change.
pub struct OverwriteStrategy {}

impl MergeStrategy for OverwriteStrategy {
    fn apply<U: RelativeTo + ConflictResolver, const T: usize>(
        &self,
        _coords: Vec2<i32>,
        _plugin: &ParsedPlugin,
        _value: &str,
        lhs: &RelativeTerrainMap<U, T>,
        rhs: &RelativeTerrainMap<U, T>,
    ) -> RelativeTerrainMap<U, T> {
        let mut new = lhs.clone();

        for coords in new.iter_grid() {
            if rhs.has_difference(coords) {
                new.set_value(coords, rhs.get_value(coords));
            }
        }

        new
    }
}

#[cfg(test)]
mod tests {
    use super::OverwriteStrategy;
    use crate::io::parsed_plugins::ParsedPlugin;
    use crate::land::grid_access::Index2D;
    use crate::land::terrain_map::{Vec2, Vec3};
    use crate::merge::merge_strategy::MergeStrategy;
    use crate::merge::relative_terrain_map::RelativeTerrainMap;

    #[test]
    fn overwrite_strategy_chooses_rhs_on_conflict() {
        let plugin = ParsedPlugin::empty("plugin.esp");
        let coords = Vec2::new(0, 0);
        let value_name = "height_map";
        let base = [[0i32, 0i32], [0i32, 0i32]];

        let mut lhs = RelativeTerrainMap::<i32, 2>::empty(base);
        lhs.set_value(Index2D::new(0, 0), 5);

        let mut rhs = RelativeTerrainMap::<i32, 2>::empty(base);
        rhs.set_value(Index2D::new(0, 0), 9);

        let strategy = OverwriteStrategy::default();
        let merged = strategy.apply(coords, &plugin, value_name, &lhs, &rhs);

        assert_eq!(merged.get_value(Index2D::new(0, 0)), 9);
    }

    #[test]
    fn overwrite_strategy_uses_rhs_absolute_value_when_references_differ() {
        let plugin = ParsedPlugin::empty("plugin.esp");
        let coords = Vec2::new(0, 0);
        let value_name = "vertex_colors";

        let lhs = RelativeTerrainMap::<Vec3<u8>, 2>::empty([[Vec3::new(0, 0, 0); 2]; 2]);

        let mut rhs = RelativeTerrainMap::<Vec3<u8>, 2>::empty([[Vec3::new(115, 110, 96); 2]; 2]);
        rhs.set_value(Index2D::new(0, 0), Vec3::new(55, 54, 53));

        let strategy = OverwriteStrategy::default();
        let merged = strategy.apply(coords, &plugin, value_name, &lhs, &rhs);

        assert_eq!(merged.get_value(Index2D::new(0, 0)), Vec3::new(55, 54, 53));
    }
}
