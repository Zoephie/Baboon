//! Selection policy for tab groups whose active child is closed or moved.
use egui_tiles::{Container, Tile, Tree};

/// Keep each source group's nearest surviving left tab selected when its
/// active pane leaves. New selections and reorders within a group stay intact.
/// Returns whether selection changed, so callers drawing the tree can repaint.
pub(in crate::app) fn with_tab_neighbour_selection<Pane>(
    tree: &mut Tree<Pane>, edit: impl FnOnce(&mut Tree<Pane>),
) -> bool {
    let before: Vec<_> = tree.tiles.iter().filter_map(|(id, tile)| {
        let Tile::Container(Container::Tabs(tabs)) = tile else { return None };
        let active = tabs.active?;
        // Container tabs can be replaced during layout simplification without
        // a user removing a pane. Only track actual document/tool tabs.
        if !matches!(tree.tiles.get(active), Some(Tile::Pane(_))) { return None; }
        let index = tabs.children.iter().position(|child| *child == active)?;
        Some((*id, active, index, tabs.children.clone()))
    }).collect();
    edit(tree);
    let mut changed = false;
    for (group, departed, index, children) in before {
        let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get(group) else { continue };
        if tabs.children.contains(&departed) { continue; }
        let next = children[..index].iter().rev().chain(children[index + 1..].iter())
            .copied().find(|child| tabs.children.contains(child)
                && tree.tiles.is_visible(*child) && tree.tiles.get(*child).is_some());
        if tabs.active != next {
            if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get_mut(group) {
                tabs.active = next;
                changed = true;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_tiles::{Container, Tile, TileId, Tiles};

    fn active(tree: &Tree<usize>, group: TileId) -> Option<TileId> {
        match tree.tiles.get(group) {
            Some(Tile::Container(Container::Tabs(tabs))) => tabs.active,
            _ => None,
        }
    }

    #[test]
    fn moving_an_active_tab_selects_its_left_neighbour_or_the_right_fallback() {
        for split in [false, true] {
            for moved_index in [0usize, 1, 3, 4] {
                let mut tiles = Tiles::default();
                let panes: Vec<_> = (0..5).map(|n| tiles.insert_pane(n)).collect();
                let source = tiles.insert_tab_tile(panes.clone());
                let other = tiles.insert_pane(5);
                let destination = tiles.insert_tab_tile(vec![other]);
                let root = tiles.insert_horizontal_tile(vec![source, destination]);
                let mut tree = Tree::new(eframe::egui::Id::unique("neighbour_move"), root, tiles);
                tree.make_active(|id, _| id == panes[moved_index]);
                with_tab_neighbour_selection(&mut tree, |tree| {
                    tree.move_tile_to_container(panes[moved_index], if split { root } else { destination }, 1, false);
                });
                let expected = panes[if moved_index == 0 { 1 } else { moved_index - 1 }];
                assert_eq!(active(&tree, source), Some(expected), "split={split}, moved={moved_index}");
                if !split {
                    assert_eq!(active(&tree, destination), Some(panes[moved_index]));
                }
            }
        }
    }

    #[test]
    fn moving_an_inactive_tab_or_reordering_keeps_the_intended_active_tab() {
        let mut tiles = Tiles::default();
        let panes: Vec<_> = (0..5).map(|n| tiles.insert_pane(n)).collect();
        let source = tiles.insert_tab_tile(panes[..4].to_vec());
        let destination = tiles.insert_tab_tile(vec![panes[4]]);
        let root = tiles.insert_horizontal_tile(vec![source, destination]);
        let mut tree = Tree::new(eframe::egui::Id::unique("neighbour_reorder"), root, tiles);
        tree.make_active(|id, _| id == panes[2]);
        with_tab_neighbour_selection(&mut tree, |tree| tree.move_tile_to_container(panes[0], destination, 1, false));
        assert_eq!(active(&tree, source), Some(panes[2]));
        with_tab_neighbour_selection(&mut tree, |tree| tree.move_tile_to_container(panes[2], source, 0, false));
        assert_eq!(active(&tree, source), Some(panes[2]));
        assert_eq!(active(&tree, destination), Some(panes[0]));
    }
}
