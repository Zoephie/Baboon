//! Kit tests of every game's feature readers: antennas, cloth, change colours,
//! firing, flags, lights and particle systems, read off real tags.

use super::*;
use crate::app::model_preview::load::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::app::model_preview::harness::kit::{kit_preview, kit_preview_with};
use crate::app::model_preview::common::attached::Attached;
use crate::core::game::GameId;

/// With `BLAM_TEST_H4EK`: the Warthog's antenna sways and the CTF flag
/// hangs as cloth, through Reach's readers. Lasky names a sensor-pack
/// antenna, but his model has no `antenna` marker for it, so he has none,
/// nor has the Chief either.
#[test]
fn halo_4_objects_bring_their_antennas_and_cloth() {
    let Some(root) = kit_root(GameId::Halo4) else { return };
    let warthog = kit_preview(&root, GameId::Halo4, "objects\\vehicles\\human\\storm_warthog\\storm_warthog", "vehicle", b"vehi");
    assert!(!warthog.preview.antennas.is_empty(), "{:?}", warthog.notes);
    let lasky = kit_preview(&root, GameId::Halo4, "objects\\characters\\storm_lasky\\storm_lasky", "biped", b"bipd");
    assert!(lasky.preview.antennas.is_empty());
    let flag = kit_preview(&root, GameId::Halo4, "objects\\weapons\\multiplayer\\flag\\flag", "weapon", b"weap");
    assert!(!flag.preview.cloth.is_empty(), "{:?}", flag.notes);
    let chief = kit_preview(&root, GameId::Halo4, "objects\\characters\\storm_masterchief\\storm_masterchief", "biped", b"bipd");
    assert!(chief.preview.antennas.is_empty() && chief.preview.cloth.is_empty());
}

/// Whether previewing the tag at `path` comes with any change colors.
fn has_colors(root: &std::path::Path, game: GameId, path: &str, extension: &str, group: &[u8; 4]) -> bool {
    let data = kit_preview(root, game, path, extension, group);
    data.change_colors.iter().any(|channel| !channel.is_empty())
}

/// With `BLAM_TEST_HCEEK`: change colors are the object's, so a gbxmodel
/// beside one has none.
#[test]
fn only_a_halo_ce_object_has_change_colors() {
    let Some(root) = kit_root(GameId::HaloCe) else { return };
    let path = "characters\\cyborg\\cyborg";
    assert!(has_colors(&root, GameId::HaloCe, path, "biped", b"bipd"));
    assert!(!has_colors(&root, GameId::HaloCe, path, "gbxmodel", b"mod2"));
}

/// With `BLAM_TEST_H2EK`: the same for a `.model`.
#[test]
fn only_a_halo_2_object_has_change_colors() {
    let Some(root) = kit_root(GameId::Halo2) else { return };
    let path = "objects\\characters\\brute\\brute";
    assert!(has_colors(&root, GameId::Halo2, path, "biped", b"bipd"));
    assert!(!has_colors(&root, GameId::Halo2, path, "model", b"hlmt"));
}

/// With `BLAM_TEST_H3EK`: a vehicle's built-in gun is an `extension of
/// parent`, so the game plays its attachments at the vehicle's markers;
/// the machine-gun turret's is its firing light.
#[test]
fn a_halo_3_vehicle_brings_in_its_extension_weapons_attachments() {
    let Some(root) = kit_root(GameId::Halo3) else { return };
    let path = "objects\\weapons\\turret\\machinegun_turret\\machinegun_turret";
    let data = kit_preview(&root, GameId::Halo3, path, "vehicle", b"vehi");
    // The weapon's `firing` light, attached with the primary scale
    // `illumination`; the vehicle has none of its own.
    let firing = data.preview.h3.lights.iter().filter(|light| light.scale_name() == "illumination");
    assert_eq!(firing.count(), 1, "the gun's firing light");
}

/// With `BLAM_TEST_HREK`: the warthog's one widget is its antenna, on
/// its `antenna` marker, sprung and weighted by its steel. At rest it
/// stands straight: its length takes out what gravity does along it.
#[test]
fn kit_reach_warthog_hangs_its_antenna_straight() {
    let Some(root) = kit_root(GameId::HaloReach) else { return };
    let path = "objects\\vehicles\\human\\warthog\\warthog";
    let data = kit_preview(&root, GameId::HaloReach, path, "vehicle", b"vehi");
    let antennas = &data.preview.antennas;
    assert_eq!(antennas.len(), 1);
    let antenna = &antennas[0];
    assert!(antenna.is_reach() && antenna.h3_bitmap.is_some());
    let spring = antenna.reach_spring().unwrap();
    assert_eq!(spring.strength, 0.1);
    assert!((spring.length - 0.4).abs() < 1e-6);
    assert!(spring.physics.is_some(), "warthog_antenna_steel");
    let (marker, forward) = antenna.marker(None);
    assert!(forward[2] > 0.99, "the marker faces up: {forward:?}");
    let mut field = super::super::antennas::spring::ReachAntennaField::default();
    field.advance(antennas, None, 0.0);
    let tip = field.advance(antennas, None, 20.0)[0].unwrap();
    let rest: [f32; 3] = std::array::from_fn(|axis| marker[axis] + forward[axis] * spring.length);
    // The marker leans a little off up, so gravity bends the tip a hair.
    assert!((0..3).all(|axis| (tip[axis] - rest[axis]).abs() < 1e-3), "{tip:?} against {rest:?}, {forward:?}");
    assert!(tip[0] < rest[0], "toward where it leans: {tip:?} against {rest:?}, {forward:?}");
    let view = super::super::common::view::ViewFrame {
        eye: [0.0, -3.0, 1.0],
        forward: [0.0, 1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
    };
    assert_eq!(antenna.quads(None, &view, Some(tip)).len(), 10, "a sprite a vertex");
}

/// With `BLAM_TEST_HREK`: the scorpion's antennas hang on its cannon, a
/// child object, so they come in through it.
#[test]
fn kit_reach_scorpion_brings_its_cannons_antennas() {
    let Some(root) = kit_root(GameId::HaloReach) else { return };
    let path = "objects\\vehicles\\human\\scorpion\\scorpion";
    let data = kit_preview(&root, GameId::HaloReach, path, "vehicle", b"vehi");
    let antennas = &data.preview.antennas;
    assert_eq!(antennas.len(), 2, "short_antenna and medium_antenna");
    assert!(antennas.iter().all(|antenna| antenna.is_reach()));
    let lengths: Vec<f32> = antennas.iter().map(|antenna| antenna.reach_spring().unwrap().length).collect();
    assert!((lengths[0] - 0.25).abs() < 1e-5 && (lengths[1] - 0.45).abs() < 1e-5, "{lengths:?}");
}

/// With `BLAM_TEST_HREK`: the marine's radio antenna hangs on a marker of
/// its `trooper_radio` backpack alone, and goes with it.
#[test]
fn kit_reach_marine_antenna_goes_with_its_backpack() {
    let Some(root) = kit_root(GameId::HaloReach) else { return };
    let path = "objects\\characters\\marine\\marine";
    let data = kit_preview(&root, GameId::HaloReach, path, "biped", b"bipd");
    let antennas = &data.preview.antennas;
    assert_eq!(antennas.len(), 1);
    let under = antennas[0].only_under().map(|(region, permutation)| (region.as_str(), permutation.as_str()));
    assert_eq!(under, Some(("flair_backpack", "trooper_radio")));
}

/// With `BLAM_TEST_HREK`: Reach's flag hangs from all ten of its
/// `flag_attach` markers, down its first column.
#[test]
fn kit_reach_flag_hangs_from_all_ten_markers() {
    let Some(root) = kit_root(GameId::HaloReach) else { return };
    let path = "objects\\weapons\\multiplayer\\flag\\flag";
    let data = kit_preview(&root, GameId::HaloReach, path, "weapon", b"weap");
    let cloths = &data.preview.cloth;
    assert_eq!(cloths.len(), 1);
    assert_eq!(cloths[0].game(), super::super::cloth::ClothGame::Reach);
    assert_eq!(cloths[0].pinned_vertices(), [0, 10, 20, 30, 40, 50, 60, 70, 80, 90]);
    // The model's variant names a `flag` region its render model lacks;
    // the sheet, listed apart from it, still shows.
    let mut state = ModelPreviewState::default();
    reset_model_preview_selection(&mut state, &data, default_variant_index(&data));
    let visible = visible_batch_indices(&data.preview, &state);
    let sheet = data.preview.batches.iter().position(|batch| batch.region_name == "flag.cloth");
    assert!(sheet.is_some_and(|sheet| visible.contains(&sheet)), "{sheet:?} in {visible:?}");
}

/// With `BLAM_TEST_H3EK`: Halo 3's flag and warthog stay Halo 3's, six
/// pins at most and an antenna at rest.
#[test]
fn kit_halo_3_cloth_and_antennas_stay_halo_3s() {
    let Some(root) = kit_root(GameId::Halo3) else { return };
    let path = "objects\\weapons\\multiplayer\\flag\\flag";
    let data = kit_preview(&root, GameId::Halo3, path, "weapon", b"weap");
    let cloths = &data.preview.cloth;
    assert_eq!(cloths.len(), 1);
    assert_eq!(cloths[0].game(), super::super::cloth::ClothGame::Halo3);
    assert!(cloths[0].pinned_vertices().len() <= 6);
    let path = "objects\\vehicles\\warthog\\warthog";
    let data = kit_preview(&root, GameId::Halo3, path, "vehicle", b"vehi");
    assert!(!data.preview.antennas.is_empty());
    assert!(data.preview.antennas.iter().all(|antenna| !antenna.is_reach()));
}

/// With `BLAM_TEST_H2EK`: an effect's `lens_flare` and `light_volume`
/// parts draw at their location's markers while their event runs
/// (`sub_515280`), as the sword's and the flak bolt's do.
#[test]
fn halo_2_effects_bring_their_lens_flares_and_light_volumes() {
    let Some(root) = kit_root(GameId::Halo2) else { return };
    let path = "objects\\weapons\\melee\\energy_blade\\energy_blade";
    let sword = kit_preview(&root, GameId::Halo2, path, "weapon", b"weap");
    let flares = sword.preview.h2.flares.iter().filter(|flare| flare.effect_part().is_some());
    assert!(flares.count() > 0, "the blade's effects' lens flares");
    let path = "objects\\weapons\\support_high\\flak_cannon\\projectiles\\flak_bolt\\flak_bolt";
    let bolt = kit_preview(&root, GameId::Halo2, path, "projectile", b"proj");
    let volumes = bolt.preview.h2.light_volumes.iter().filter(|volume| volume.effect_part().is_some());
    assert!(volumes.count() > 0, "the armed bolt's light volume");
}

/// A flag starts settled whenever it starts: one restarted a minute in
/// has nothing to catch up on, so it hangs as one started at the open.
#[test]
fn a_flag_restarted_late_starts_settled() {
    let Some(root) = kit_root(GameId::HaloCe) else { return };
    let data = kit_preview(&root, GameId::HaloCe, "weapons\\flag\\flag", "weapon", b"weap");
    let flags = &data.preview.ce.flags;
    assert!(!flags.is_empty(), "the flag weapon's flag");
    let at_open = super::super::ce::flags::CeFlagField::default().advance(flags, None, 0.0, 0.0);
    let a_minute_in = super::super::ce::flags::CeFlagField::default().advance(flags, None, 0.0, 60.0);
    assert_eq!(at_open, a_minute_in);
}

/// `game_frame` runs `widgets_update`, and so `flags_update`, only on a
/// frame ticks ran, by them: at 120 frames a second the flag moves on
/// every fourth frame, and holds between.
#[test]
fn a_flag_moves_only_on_frames_a_tick_runs() {
    let Some(root) = kit_root(GameId::HaloCe) else { return };
    let data = kit_preview(&root, GameId::HaloCe, "weapons\\flag\\flag", "weapon", b"weap");
    let flags = &data.preview.ce.flags;
    let mut field = super::super::ce::flags::CeFlagField::default();
    let mut last = field.advance(flags, None, 0.0, 10.0);
    let mut moved = Vec::new();
    for frame in 1..=12 {
        let now = field.advance(flags, None, 0.0, 10.0 + frame as f32 / 120.0);
        moved.push(now != last);
        last = now;
    }
    let ticked = [false, false, false, true];
    assert_eq!(moved, ticked.repeat(3));
}

/// `object_set_initial_change_colors` colors every object from its own
/// tag: the scorpion's turret is not white, and not the hull's colors.
#[test]
fn a_halo_3_child_object_has_its_own_change_colors() {
    let Some(root) = kit_root(GameId::Halo3) else { return };
    let data = kit_preview(&root, GameId::Halo3, "objects\\vehicles\\scorpion\\scorpion", "vehicle", b"vehi");
    let colored = data.attached.iter().find(|attached| attached.change_colors != [[1.0; 3]; 4]);
    let colored = colored.expect("a child with change colors");
    let hull = super::super::dx9::passes::change_colors_for(&data.change_colors, "default");
    assert_ne!(colored.change_colors, hull);
    let owned = data.preview.materials.iter().filter(|material| material.object == colored.owner);
    assert!(owned.count() > 0, "its materials read them");
}

/// An attached object's values answer every name its own lights and
/// effects read: the warthog's chaingun lights by one the hull never reads.
#[test]
fn an_attached_object_answers_the_names_its_effects_read() {
    let Some(root) = kit_root(GameId::Halo3) else { return };
    let data = kit_preview(&root, GameId::Halo3, "objects\\vehicles\\warthog\\warthog", "vehicle", b"vehi");
    let name = "warthog_chaingun_illumination";
    assert!(!super::super::frame::h2_shader_inputs(&data, 0).iter().any(|read| read == name));
    let values = super::super::frame::attached_values(&data, 0.0);
    for attached in &data.attached {
        let (_, own) = values.iter().find(|(owner, _)| *owner == attached.owner).unwrap();
        for read in super::super::frame::h2_shader_inputs(&data, attached.owner) {
            assert!(own.contains_key(&read), "owner {} lacks {read}", attached.owner);
        }
    }
    let chaingun = values.iter().filter(|(_, own)| own.contains_key(name));
    assert_eq!(chaingun.count(), 1, "the chaingun answers {name}");
}

/// A `.model` opened on its own shows what its variants attach, as its
/// object does: the warthog's turret and the rest, in Halo 3 and Halo 4.
#[test]
fn a_model_opened_alone_brings_its_variants_children() {
    let kits = [
        (GameId::Halo3, "objects\\vehicles\\warthog\\warthog"),
        (GameId::Halo4, "objects\\vehicles\\human\\storm_warthog\\storm_warthog"),
    ];
    for (game, path) in kits {
        let Some(root) = kit_root(game) else { continue };
        let model = kit_preview(&root, game, path, "model", b"hlmt");
        let vehicle = kit_preview(&root, game, path, "vehicle", b"vehi");
        assert!(!model.attached.is_empty(), "{path}: the model's children");
        let owners = |data: &ModelPreviewData| {
            data.attached.iter().map(|attached| attached.owner).collect::<Vec<_>>()
        };
        assert_eq!(owners(&model), owners(&vehicle), "{path}: the same children, once each");
    }
}

/// The phantom carries two plasma cannons of one tag: each has regions of
/// its own, rather than both answering to one selection.
#[test]
fn children_of_one_name_keep_their_regions_apart() {
    let Some(root) = kit_root(GameId::Halo3) else { return };
    let data = kit_preview(&root, GameId::Halo3, "objects\\vehicles\\phantom\\phantom", "vehicle", b"vehi");
    let mut names: Vec<&str> = data.preview.regions.iter().map(|region| region.name.as_str()).collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "a region name repeats");
    assert!(names.contains(&"plasma_cannon 2: barrel"));
}

/// `light_submit_lens_flares`: the Covenant portable light's light names a
/// lens flare, which draws at the light, in Halo 3 and ODST alike.
#[test]
fn a_lights_own_lens_flare_comes_with_it() {
    let path = "objects\\levels\\solo\\070_waste\\cov_portable_light\\cov_portable_light";
    for game in [GameId::Halo3, GameId::Halo3Odst] {
        let Some(root) = kit_root(game) else { continue };
        let data = kit_preview(&root, game, path, "device_machine", b"mach");
        let from_lights = data.preview.gen3.flares.iter().filter(|at| at.light().is_some()).count();
        assert_eq!(from_lights, 1, "{game:?}: the light's flare");
    }
}

/// A Halo 3 light alone keeps the picture moving with its object, as
/// every other effect does: the outskirts hog light has nothing else.
#[test]
fn a_halo_3_light_alone_counts_as_an_effect() {
    let Some(root) = kit_root(GameId::Halo3) else { return };
    let path = "objects\\levels\\solo\\030_outskirts\\hog_light\\hog_light";
    let data = kit_preview(&root, GameId::Halo3, path, "scenery", b"scen");
    assert_eq!(data.preview.h3.lights.len(), 1);
    assert!(super::super::frame::has_effects(&data.preview));
}

/// A Halo CE part sorts from its centroid on the node tool's postprocess
/// gives it, the one its vertices weigh most: tags on disk say node 0 for
/// every part. The later games' batches carry no sort position.
#[test]
fn a_halo_ce_part_sorts_from_its_centroid_on_its_heaviest_node() {
    let (Some(ce), Some(h3)) = (kit_root(GameId::HaloCe), kit_root(GameId::Halo3)) else { return };
    let data = kit_preview(&ce, GameId::HaloCe, "characters\\cyborg\\cyborg", "biped", b"bipd");
    let preview = &data.preview;
    let mut off_the_root = 0;
    for batch in preview.batches.iter().filter(|batch| batch.layer == ModelPreviewLayer::Render) {
        let (point, node) = batch.sort_position.expect("every CE part knows where it sorts");
        let start = batch.index_start as usize;
        let indices = &preview.indices[start..start + batch.index_count as usize];
        let mut weights = vec![0.0f32; preview.nodes.len()];
        for vertex in indices.iter().map(|index| &preview.vertices[*index as usize]) {
            for (node, weight) in vertex.node_indices.iter().zip(vertex.node_weights) {
                weights[*node as usize] += weight;
            }
        }
        let heaviest = (0..weights.len()).max_by(|a, b| weights[*a].total_cmp(&weights[*b])).unwrap();
        assert_eq!(node, heaviest, "a part's sort node is its heaviest");
        off_the_root += usize::from(node != 0);
        assert!(point.iter().all(|axis| axis.is_finite()));
    }
    assert!(off_the_root > 0, "some part rides a node past the root");
    let chief = "objects\\characters\\masterchief\\masterchief";
    let data = kit_preview(&h3, GameId::Halo3, chief, "biped", b"bipd");
    assert!(data.preview.batches.iter().all(|batch| batch.sort_position.is_none()));
}

/// With `BLAM_TEST_HCEEK`: the cyborg's held rifle names `fire bullet`
/// for its trigger. It makes nothing until played; a play runs once on
/// the rifle and is gone when its particles are, as one
/// `effect_new_from_object` makes is never looped. Between ticks no
/// particle updates, so the muzzle flash, which steps a sprite an update
/// and has one, lives to be drawn.
#[test]
fn a_played_firing_effect_runs_once_on_the_held_weapon() {
    use super::super::ce::effects::{CeEventPlays, CeObjectInputs, CeParticleField};
    let Some(root) = kit_root(GameId::HaloCe) else { return };
    let rifle = "weapons\\assault rifle\\assault rifle";
    let settings = PreviewLoadSettings { weapon: Some(rifle.to_owned()), ..Default::default() };
    let data = kit_preview_with(&root, GameId::HaloCe, "characters\\cyborg\\cyborg", "biped", b"bipd", &settings);
    let effects = &data.preview.ce.effects;
    let fire = effects
        .iter()
        .position(|effect| effect.object == 1 && effect.event.as_ref().is_some_and(|event| event.name == "firing effect: fire bullet"))
        .expect("the held rifle's firing effect is listed on it");
    let inputs = |_| CeObjectInputs { outputs: [0.0; 4], active: [false; 4], change_colors: [[1.0; 3]; 4] };
    let mut plays = CeEventPlays { counts: vec![0; effects.len()], scales: [1.0, 0.0] };
    let mut field = CeParticleField::default();
    let mut time = 10.0;
    let step = |field: &mut CeParticleField, plays: &CeEventPlays, time: &mut f32| {
        *time += 1.0 / 60.0;
        field.advance(effects, plays, inputs, None, 0.0, *time, 0);
    };
    step(&mut field, &plays, &mut time);
    for _ in 0..60 {
        step(&mut field, &plays, &mut time);
    }
    assert!(!field.runs_played(effects.len()), "unplayed, it makes nothing");
    assert_eq!(field.particle_count(), 0);
    plays.counts[fire] = 1;
    let mut made = 0;
    for _ in 0..4 {
        let before = field.particle_count();
        step(&mut field, &plays, &mut time);
        let after = field.particle_count();
        if before == 0 && after > 0 {
            made = after;
            // The frame after the tick runs none: all of them are kept.
            step(&mut field, &plays, &mut time);
            assert_eq!(field.particle_count(), made, "nothing updates between ticks");
            break;
        }
    }
    assert!(made > 0, "the play made particles on its first tick");
    for _ in 0..60 * 40 {
        step(&mut field, &plays, &mut time);
    }
    assert!(!field.runs_played(effects.len()), "a play runs once and is let go");
}

/// A preview knows the game it came from: a Halo CE tag's, a Halo 2
/// tag's, and a gen3 tag's by its kit; and its tag's generation either way.
#[test]
fn a_preview_knows_its_game() {
    use blam_tags::game::Game;
    let (Some(ce), Some(h2)) = (kit_root(GameId::HaloCe), kit_root(GameId::Halo2)) else { return };
    let cyborg = kit_preview(&ce, GameId::HaloCe, "characters\\cyborg\\cyborg", "biped", b"bipd");
    assert_eq!(cyborg.preview.game, Some(GameId::HaloCe));
    assert_eq!(cyborg.preview.generation, Some(Game::Halo1));
    let blade = kit_preview(&h2, GameId::Halo2, "objects\\weapons\\melee\\energy_blade\\energy_blade", "weapon", b"weap");
    assert_eq!(blade.preview.game, Some(GameId::Halo2));
    assert_eq!(blade.preview.generation, Some(Game::Halo2));
    if let Some(reach) = kit_root(GameId::HaloReach) {
        let spartan = kit_preview(&reach, GameId::HaloReach, "objects\\characters\\spartans\\spartans", "biped", b"bipd");
        assert_eq!(spartan.preview.game, Some(GameId::HaloReach));
        assert_eq!(spartan.preview.generation, Some(Game::Halo3));
    }
}

/// With `BLAM_TEST_HCEEK`: firing the held rifle's trigger plays its
/// `fire bullet` and lights the rifle's `muzzle flash` output from its
/// illumination, which goes dark again over the trigger's illumination
/// recovery time. Unfired, it is dark.
#[test]
fn firing_the_held_rifle_lights_its_muzzle_flash() {
    use super::super::ce::firing::{CeWeaponShots, fired_input};
    let Some(root) = kit_root(GameId::HaloCe) else { return };
    let rifle = "weapons\\assault rifle\\assault rifle";
    let settings = PreviewLoadSettings { weapon: Some(rifle.to_owned()), ..Default::default() };
    let data = kit_preview_with(&root, GameId::HaloCe, "characters\\cyborg\\cyborg", "biped", b"bipd", &settings);
    let preview = &data.preview;
    let weapon = preview.ce.firing.iter().find(|weapon| weapon.object == 1).expect("the held rifle fires");
    let functions = data.ce_attached[0].functions.as_ref().expect("the rifle's functions");
    let flash = functions.output_named("muzzle flash").expect("the rifle's muzzle flash function");
    let mut shots = super::super::ce::firing::CeShots::new();
    let at = |shots: &super::super::ce::firing::CeShots, time: f32| {
        let fired = fired_input(&preview.ce.firing, shots, data.geometry_id, 1, time);
        let inputs = functions.with_fired(functions.inputs_for(&Default::default(), (data.geometry_id, 1)), fired);
        functions.outputs(&inputs, time)[flash]
    };
    assert_eq!(at(&shots, 10.0), 0.0, "unfired, the muzzle is dark");
    let fired: &mut CeWeaponShots = shots.entry((data.geometry_id, 1)).or_default();
    let effect = fired.fire(weapon, 0, 1.0, 10.0).expect("the trigger has a firing effect");
    let name = preview.ce.effects[effect].event.as_ref().map(|event| event.name.as_str());
    assert_eq!(name, Some("firing effect: fire bullet"));
    assert!(at(&shots, 10.01) > 0.0, "the shot lights the muzzle");
    assert_eq!(at(&shots, 15.0), 0.0, "and it goes dark");
}

/// With `BLAM_TEST_HCEEK`: the plasma rifle's `overheated` loops its one
/// event, as an attachment would, but a play of it is not looping
/// (`effect_update` loops back only for flag 2, `effect_new_looping`'s),
/// so it runs through once and is let go.
#[test]
fn a_played_event_effect_does_not_loop() {
    use super::super::ce::effects::{CeEventPlays, CeObjectInputs, CeParticleField};
    let Some(root) = kit_root(GameId::HaloCe) else { return };
    let rifle = "weapons\\plasma rifle\\plasma rifle";
    let settings = PreviewLoadSettings { weapon: Some(rifle.to_owned()), ..Default::default() };
    let data = kit_preview_with(&root, GameId::HaloCe, "characters\\cyborg\\cyborg", "biped", b"bipd", &settings);
    let effects = &data.preview.ce.effects;
    let overheated = effects
        .iter()
        .position(|effect| effect.event.as_ref().is_some_and(|event| event.name.ends_with(": overheated")))
        .expect("the plasma rifle's overheated effect is listed");
    assert!(effects[overheated].def.loops(), "its tag loops its event");
    let inputs = |_| CeObjectInputs { outputs: [0.0; 4], active: [false; 4], change_colors: [[1.0; 3]; 4] };
    let mut plays = CeEventPlays { counts: vec![0; effects.len()], scales: [1.0, 0.0] };
    let mut field = CeParticleField::default();
    field.advance(effects, &plays, inputs, None, 0.0, 10.0, 0);
    plays.counts[overheated] = 1;
    let mut ran = false;
    for frame in 1..=30 * 60 {
        field.advance(effects, &plays, inputs, None, 0.0, 10.0 + frame as f32 / 30.0, 0);
        ran |= field.particle_count() > 0;
    }
    assert!(ran, "the play made particles");
    assert!(!field.runs_played(effects.len()), "it ran through once and was let go");
}

/// Particle systems share `particle system particles`: with most of it
/// taken by effects' systems, the steam jets make only what is left.
#[test]
fn particle_systems_make_only_what_the_shared_array_has_room_for() {
    use super::super::ce::particle_systems::{CeParticleSystemField, MOST_SYSTEM_PARTICLES};
    let Some(root) = kit_root(GameId::HaloCe) else { return };
    let data = kit_preview(&root, GameId::HaloCe, "scenery\\emitters\\steam jets\\steam jets", "scenery", b"scen");
    let systems = &data.preview.ce.particle_systems;
    let on = |_: u8| ([1.0; 4], [true; 4], [[1.0; 3]; 4]);
    let made = |others: usize| {
        let mut field = CeParticleSystemField::default();
        field.advance(systems, on, None, 0.0, 10.0, others);
        field.particle_count()
    };
    let left = 10;
    assert!(made(0) > left, "on its own it makes more than {left}");
    assert!(made(MOST_SYSTEM_PARTICLES - left) <= left);
}
