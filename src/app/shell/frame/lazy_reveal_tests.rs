//! Revealing a tag inside folders the loose browser has not loaded yet. Each
//! folder loads once the frame that drew it open is over, so a reveal can only
//! open the next folder down a frame later; it has to stay armed until it
//! reaches its tag rather than be spent on the first frame.

use super::perf_baseline_tests::Harness;
use crate::app::loose_fixture::*;

#[test]
fn a_reveal_through_unloaded_folders_reaches_its_tag() {
    let kit = LooseKit::new("lazy-reveal", "halo3_mcc");
    kit.write_mcc("objects/weapons/rifle/assault_rifle", "biped", |_| {});
    let mut h = Harness::new();
    kit.install(&mut h.app);
    for _ in 0..2 {
        h.frame(Vec::new());
    }
    let tree = &h.app.model.kits[0].source.as_ref().unwrap().tree;
    assert!(
        tree.children.iter().all(|node| !node.entries_loaded),
        "no folder is loaded before the reveal"
    );

    let key = kit.key("objects/weapons/rifle/assault_rifle.biped");
    h.app.reveal_in_browser(&key);
    for _ in 0..8 {
        h.frame(Vec::new());
    }
    assert!(
        h.painted.iter().any(|text| text.contains("assault_rifle")),
        "the revealed tag is drawn: {:?}",
        h.painted.iter().filter(|text| text.contains("rifle")).collect::<Vec<_>>()
    );
    assert!(h.app.browser.reveal_target.is_none(), "and the reveal is spent");
}
