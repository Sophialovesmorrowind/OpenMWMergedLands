use crate::land::grid_access::SquareGridIterator;
use crate::land::terrain_map::Vec2;
use crate::merge::conflict::ConflictResolver;
use crate::merge::merge_strategy::MergeStrategy;
use crate::merge::relative_terrain_map::RelativeTerrainMap;
use crate::merge::relative_to::RelativeTo;
use crate::ParsedPlugin;

#[derive(Default)]
/// Implements [MergeStrategy] to overwrite any conflicts with the newest change.
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
            let lhs_diff = lhs.has_difference(coords);
            let rhs_diff = rhs.has_difference(coords);

            let mut diff = Default::default();
            if lhs_diff && !rhs_diff {
                diff = lhs.get_difference(coords);
            } else if !lhs_diff && rhs_diff {
                diff = rhs.get_difference(coords);
            } else if !lhs_diff && !rhs_diff {
                // NOP.
            } else {
                // Conflict -- choose rhs.
                diff = rhs.get_difference(coords);
            }

            new.set_difference(coords, diff);
        }

        new
    }
}

#[cfg(test)]
mod tests {
    use super::OverwriteStrategy;
    use crate::io::parsed_plugins::ParsedPlugin;
    use crate::land::grid_access::Index2D;
    use crate::land::terrain_map::Vec2;
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
}
