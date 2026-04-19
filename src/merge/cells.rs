use crate::io::meta_schema::MetaType;
use crate::io::parsed_plugins::{ParsedPlugin, ParsedPlugins};
use crate::land::terrain_map::Vec2;
use std::collections::HashMap;
use std::sync::Arc;
use tes3::esp::Cell;

pub struct ModifiedCell {
    pub inner: Cell,
    pub plugins: Vec<Arc<ParsedPlugin>>,
}

fn merge_cell_into(lhs: &mut ModifiedCell, rhs: &Cell, plugin: &Arc<ParsedPlugin>) {
    let new = &mut lhs.inner;
    let mut is_modified = false;

    if new.flags != rhs.flags {
        new.flags |= rhs.flags;
        is_modified = true;
    }

    if new.data != rhs.data {
        assert_eq!(new.data.grid, rhs.data.grid);
        new.data.flags |= rhs.data.flags;
        is_modified = true;
    }

    if !rhs.id.is_empty() && new.id != rhs.id {
        new.id = rhs.id.clone();
        is_modified = true;
    }

    if let Some(record) = new.region.as_ref() {
        new.region = Some(record.clone());
        is_modified = true;
    }

    if let Some(record) = new.map_color.as_ref() {
        new.map_color = Some(*record);
        is_modified = true;
    }

    if let Some(record) = new.water_height.as_ref() {
        new.water_height = Some(*record);
        is_modified = true;
    }

    if let Some(record) = new.atmosphere_data.as_ref() {
        new.atmosphere_data = Some(record.clone());
        is_modified = true;
    }

    if is_modified {
        lhs.plugins.push(plugin.clone());
    } else {
        *lhs.plugins.last_mut().expect("safe") = plugin.clone();
    }
}

fn merge_cells_into(cells: &mut HashMap<Vec2<i32>, ModifiedCell>, plugins: &[Arc<ParsedPlugin>]) {
    for plugin in plugins {
        if plugin.meta.meta_type == MetaType::MergedLands {
            continue;
        }

        for cell in plugin.records.objects_of_type::<Cell>() {
            let coords = Vec2::new(cell.data.grid.0, cell.data.grid.1);
            if cells.contains_key(&coords) {
                let prev_cell = cells.get_mut(&coords).expect("safe");
                merge_cell_into(prev_cell, cell, plugin);
            } else {
                let new_cell = ModifiedCell {
                    inner: Cell {
                        flags: cell.flags,
                        id: cell.id.clone(),
                        data: cell.data.clone(),
                        region: cell.region.clone(),
                        map_color: cell.map_color,
                        water_height: cell.water_height,
                        atmosphere_data: cell.atmosphere_data.clone(),
                        references: Default::default(),
                    },
                    plugins: vec![plugin.clone()],
                };

                cells.insert(coords, new_cell);
            };
        }
    }
}

pub fn merge_cells(parsed_plugins: &ParsedPlugins) -> HashMap<Vec2<i32>, ModifiedCell> {
    let mut cells = Default::default();

    merge_cells_into(&mut cells, &parsed_plugins.masters);
    merge_cells_into(&mut cells, &parsed_plugins.plugins);

    cells
}

#[cfg(test)]
mod tests {
    use super::merge_cells;
    use crate::io::meta_schema::{MetaType, PluginMeta};
    use crate::io::parsed_plugins::{ParsedPlugin, ParsedPlugins};
    use crate::land::terrain_map::Vec2;
    use std::sync::Arc;
    use tes3::esp::{Cell, Plugin, TES3Object};

    fn cell_at(x: i32, y: i32, id: &str) -> Cell {
        let mut cell: Cell = Default::default();
        cell.data.grid = (x, y);
        cell.id = id.to_string();
        cell
    }

    fn parsed_plugin(name: &str, cells: Vec<Cell>, meta_type: MetaType) -> Arc<ParsedPlugin> {
        let mut records = Plugin::new();
        for cell in cells {
            records.objects.push(TES3Object::Cell(cell));
        }

        Arc::new(ParsedPlugin {
            name: name.to_string(),
            records,
            meta: PluginMeta {
                meta_type,
                ..Default::default()
            },
        })
    }

    #[test]
    fn merge_cells_applies_master_then_plugin_in_order() {
        let master = parsed_plugin(
            "master.esm",
            vec![cell_at(10, 20, "OldCell")],
            MetaType::Auto,
        );
        let plugin = parsed_plugin("mod.esp", vec![cell_at(10, 20, "NewCell")], MetaType::Auto);

        let parsed_plugins = ParsedPlugins {
            masters: vec![master],
            plugins: vec![plugin],
        };

        let cells = merge_cells(&parsed_plugins);
        let merged = cells
            .get(&Vec2::new(10, 20))
            .expect("merged cell should exist");

        assert_eq!(merged.inner.id, "NewCell");
        assert_eq!(merged.plugins.len(), 2);
        assert_eq!(merged.plugins[0].name, "master.esm");
        assert_eq!(merged.plugins[1].name, "mod.esp");
    }

    #[test]
    fn merge_cells_skips_merged_lands_meta_plugins() {
        let generated = parsed_plugin(
            "Merged Lands.esp",
            vec![cell_at(1, 2, "Generated")],
            MetaType::MergedLands,
        );

        let parsed_plugins = ParsedPlugins {
            masters: vec![],
            plugins: vec![generated],
        };

        let cells = merge_cells(&parsed_plugins);
        assert!(cells.is_empty());
    }

    #[test]
    fn merge_cells_replaces_last_source_when_second_cell_is_identical() {
        let first = parsed_plugin("a.esp", vec![cell_at(3, 4, "")], MetaType::Auto);
        let second = parsed_plugin("b.esp", vec![cell_at(3, 4, "")], MetaType::Auto);

        let parsed_plugins = ParsedPlugins {
            masters: vec![],
            plugins: vec![first, second],
        };

        let cells = merge_cells(&parsed_plugins);
        let merged = cells
            .get(&Vec2::new(3, 4))
            .expect("merged cell should exist");

        assert_eq!(merged.plugins.len(), 1);
        assert_eq!(merged.plugins[0].name, "b.esp");
    }
}
