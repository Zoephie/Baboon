use super::super::level::{IDENTITY, compose};
use super::*;

#[test]
fn a_prim_name_is_an_identifier() {
    assert_eq!(prim_name("SM_Tree_Mangrove_A"), "SM_Tree_Mangrove_A");
    assert_eq!(prim_name("SM-Tree.01"), "SM_Tree_01");
    // USD will not accept a name that leads with a digit, and Campaign
    // Evolved's generated cells are named exactly that way.
    assert_eq!(prim_name("043ATWPYEEJ"), "_043ATWPYEEJ");
    assert_eq!(prim_name(""), "_");
}

#[test]
fn colliding_leaves_get_distinct_prims() {
    // Two packages can end in the same name, and a prototype that silently
    // replaced another would place the wrong mesh everywhere it is used.
    let mut taken = HashSet::new();
    assert_eq!(unique_prim_name("SM_Rock", &mut taken), "SM_Rock");
    assert_eq!(unique_prim_name("SM_Rock", &mut taken), "SM_Rock_1");
    assert_eq!(unique_prim_name("SM_Rock", &mut taken), "SM_Rock_2");
    assert_eq!(unique_prim_name("SM-Rock", &mut taken), "SM_Rock_3");
}

/// One placement, as the text the exporter writes for it.
fn instance_text(index: usize, prototype: &str, world: &WorldMatrix) -> String {
    let mut usd: Vec<u8> = Vec::new();
    write_instance(&mut usd, index, prototype, world);
    String::from_utf8(usd).expect("the writer emits ASCII")
}

#[test]
fn a_placement_references_the_prototype_rather_than_copying_it() {
    let usd = instance_text(7, "SM_Rock", &IDENTITY);
    assert!(usd.contains("def Xform \"inst_7\""));
    assert!(usd.contains("instanceable = true"));
    assert!(usd.contains("references = </World/Prototypes/SM_Rock>"));
    // Geometry must never appear beside a placement.
    assert!(!usd.contains("points"));
}

#[test]
fn a_placement_is_written_as_unreals_own_matrix() {
    // USD and Unreal agree on layout — row-major, translation last — so a
    // transposed write would be silently wrong rather than rejected.
    let usd = instance_text(
        0,
        "SM_Rock",
        &compose([100.0, -200.0, 50.0], [0.0; 3], [1.0; 3]),
    );
    assert!(
        usd.contains("(100, -200, 50, 1)"),
        "the translation must be the last row: {usd}"
    );
}

#[test]
fn a_mirrored_placement_needs_no_special_case() {
    // The reason for USD over a quaternion-and-one-scale format: a
    // reflection is just a matrix, so no placement needs baked geometry and
    // the instancing survives.
    let usd = instance_text(0, "SM_Rock", &compose([0.0; 3], [0.0; 3], [-1.3, 1.3, 1.3]));
    assert!(usd.contains("(-1.3, 0, 0, 0)"), "{usd}");
    assert!(usd.contains("instanceable = true"));
}

#[test]
fn an_identity_placement_writes_the_identity() {
    let usd = instance_text(0, "SM_Rock", &IDENTITY);
    assert!(usd.contains("( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )"));
}
