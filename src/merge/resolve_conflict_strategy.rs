use crate::ParsedPlugin;
use crate::land::grid_access::SquareGridIterator;
use crate::land::terrain_map::Vec2;
use crate::merge::conflict::{ConflictResolver, ConflictType};
use crate::merge::merge_strategy::MergeStrategy;
use crate::merge::relative_terrain_map::RelativeTerrainMap;
use crate::merge::relative_to::RelativeTo;

#[derive(Default)]
/// Implements [`MergeStrategy`] to resolve any conflicts by merging changes together.
pub struct ResolveConflictStrategy {}

impl MergeStrategy for ResolveConflictStrategy {
    fn apply<U: RelativeTo + ConflictResolver, const T: usize>(
        &self,
        _coords: Vec2<i32>,
        _plugin: &ParsedPlugin,
        _value: &str,
        lhs: &RelativeTerrainMap<U, T>,
        rhs: &RelativeTerrainMap<U, T>,
    ) -> RelativeTerrainMap<U, T>
    where
        <U as RelativeTo>::Delta: ConflictResolver,
    {
        let mut new = lhs.clone();

        let params = crate::merge::conflict::ConflictParams::default();

        for coords in new.iter_grid() {
            let lhs_diff = lhs.has_difference(coords);
            let rhs_diff = rhs.has_difference(coords);

            if !lhs_diff && rhs_diff {
                new.set_value(coords, rhs.get_value(coords));
            } else if lhs_diff && rhs_diff {
                let lhs_value = lhs.get_value(coords);
                let rhs_value = rhs.get_value(coords);

                match lhs_value.average(rhs_value, &params) {
                    None => {
                        new.set_value(coords, lhs_value);
                    }
                    Some(ConflictType::Minor(value) | ConflictType::Major(value)) => {
                        new.set_value(coords, value);
                    }
                }
            }
        }

        new
    }
}

#[cfg(test)]
mod tests {
    use super::ResolveConflictStrategy;
    use crate::io::parsed_plugins::ParsedPlugin;
    use crate::land::grid_access::Index2D;
    use crate::land::terrain_map::Vec2;
    use crate::merge::merge_strategy::MergeStrategy;
    use crate::merge::relative_terrain_map::RelativeTerrainMap;

    #[test]
    fn resolve_strategy_uses_rhs_absolute_value_when_only_rhs_differs() {
        let plugin = ParsedPlugin::empty("plugin.esp");
        let coords = Vec2::new(0, 0);
        let value_name = "height_map";

        let lhs = RelativeTerrainMap::<i32, 2>::empty([[0; 2]; 2]);

        let mut rhs = RelativeTerrainMap::<i32, 2>::empty([[100; 2]; 2]);
        rhs.set_value(Index2D::new(0, 0), 55);

        let strategy = ResolveConflictStrategy::default();
        let merged = strategy.apply(coords, &plugin, value_name, &lhs, &rhs);

        assert_eq!(merged.get_value(Index2D::new(0, 0)), 55);
    }
}
