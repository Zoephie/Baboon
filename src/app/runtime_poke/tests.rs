use super::*;
use std::cell::{Cell, RefCell};

struct MockMemory {
    bytes: RefCell<Vec<u8>>,
    fail_write: Cell<Option<usize>>,
    writes: Cell<usize>,
    writable: Cell<bool>,
}

impl MockMemory {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: RefCell::new(bytes),
            fail_write: Cell::new(None),
            writes: Cell::new(0),
            writable: Cell::new(true),
        }
    }

    fn fail_once_at(&self, write: usize) {
        self.fail_write.set(Some(write));
        self.writes.set(0);
    }

    fn snapshot(&self) -> Vec<u8> {
        self.bytes.borrow().clone()
    }
}

impl RuntimeMemory for MockMemory {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let start = usize::try_from(address).map_err(|_| "bad mock address".to_owned())?;
        self.bytes
            .borrow()
            .get(start..start.saturating_add(length))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "bad mock read".to_owned())
    }

    fn is_writable(&self, _address: u64, _length: usize) -> Result<bool, String> {
        Ok(self.writable.get())
    }

    fn resolve_offset(&self, encoded: u32) -> Result<u64, String> {
        let (_, byte_offset) = encoded_offset_parts(encoded)?;
        Ok(byte_offset)
    }
}

impl RuntimeWriteMemory for MockMemory {
    fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String> {
        let attempt = self.writes.get();
        self.writes.set(attempt + 1);
        if self.fail_write.get() == Some(attempt) {
            self.fail_write.set(None);
            return Err("injected write failure".to_owned());
        }
        let start = usize::try_from(address).map_err(|_| "bad mock address".to_owned())?;
        let end = start
            .checked_add(bytes.len())
            .ok_or_else(|| "bad mock write".to_owned())?;
        let mut memory = self.bytes.borrow_mut();
        let destination = memory
            .get_mut(start..end)
            .ok_or_else(|| "bad mock write".to_owned())?;
        destination.copy_from_slice(bytes);
        Ok(())
    }
}

fn transaction_patches() -> Vec<PokePatch> {
    vec![
        PokePatch {
            address: 0,
            expected: vec![1, 1],
            edited: vec![9, 9],
            field_path: "a".to_owned(),
        },
        PokePatch {
            address: 4,
            expected: vec![2, 2],
            edited: vec![8, 8],
            field_path: "b".to_owned(),
        },
        PokePatch {
            address: 8,
            expected: vec![3, 3],
            edited: vec![7, 7],
            field_path: "c".to_owned(),
        },
    ]
}

fn last_poke_for_test(patch: PokePatch, original: Vec<u8>) -> LastPoke {
    LastPoke {
        plan: PokePlan {
            profile: CU2_PROFILE,
            identity: RuntimeIdentity {
                process_id: 1,
                creation_time: 2,
                module_base: 3,
                tag_table: 4,
            },
            tag_path: "objects/test".to_owned(),
            group_tag: u32::from_be_bytes(*b"test"),
            tag_handle: 5,
            tag_entry_address: 6,
            tag_name_pointer: 7,
            root_address: 8,
            patches: vec![patch],
            chain_originals: vec![None],
        },
        originals: vec![original],
    }
}

/// The profile table is searched by tag-module hash and the first match wins,
/// so a duplicate hash would silently shadow a profile, and a hash written in
/// the wrong case or length would never match at all — either way the profile
/// is dead code that looks alive.
#[test]
fn profile_table_hashes_are_unique_and_comparable() {
    let mut seen = Vec::new();
    for profile in PROFILES {
        assert_eq!(
            profile.dll_sha256.len(),
            64,
            "{} has a malformed tag-module hash",
            profile.label
        );
        assert!(
            profile
                .dll_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte)),
            "{} must spell its tag-module hash in uppercase hex to match `format!(\"{{:X}}\")`",
            profile.label
        );
        assert!(
            !seen.contains(&profile.dll_sha256),
            "{} repeats a tag-module hash already claimed by an earlier profile",
            profile.label
        );
        seen.push(profile.dll_sha256);
    }
}

#[test]
fn checked_rva_math_rejects_overflow() {
    assert_eq!(
        checked_add(0x1_8000_0000, 0x0182_e1e8).unwrap(),
        0x1_8182_e1e8
    );
    assert!(checked_add(u64::MAX, 1).is_err());
    assert!(checked_index(u64::MAX - 3, 1, 8).is_err());
}

#[test]
fn runtime_name_pool_is_bounded_and_parsed_from_one_snapshot() {
    assert_eq!(runtime_name_pool_bounds(&[]).unwrap(), None);
    assert_eq!(
        runtime_name_pool_bounds(&[0x1234, 0x5678]).unwrap(),
        Some((0x1000, 0x6000))
    );
    assert_eq!(
        runtime_name_pool_bounds(&[0x1000, 0x1000 + MAX_RUNTIME_NAME_POOL_SPAN as u64]).unwrap(),
        None
    );
    assert!(runtime_name_pool_bounds(&[u64::MAX]).is_err());

    let mut pool = vec![0u8; 32];
    pool[4..15].copy_from_slice(b"objects/foo");
    assert_eq!(
        runtime_name_from_pool(0x1000, &pool, 0x1004).unwrap(),
        "objects/foo"
    );
    assert!(runtime_name_from_pool(0x1000, &pool, 0x0fff).is_err());
}

#[test]
fn only_stale_or_missing_current_tags_refresh_the_runtime_index() {
    assert!(should_refresh_runtime_index(RUNTIME_CACHE_STALE));
    assert!(should_refresh_runtime_index(
        "runtime address cache is stale: unreadable"
    ));
    assert!(should_refresh_runtime_index("tag is not loaded"));
    assert!(!should_refresh_runtime_index(
        "referenced tag is not loaded at barrels/projectile"
    ));
}

#[test]
fn encoded_offset_uses_top_nibble_and_word_units() {
    struct Memory;
    impl RuntimeMemory for Memory {
        fn read(&self, _address: u64, _length: usize) -> Result<Vec<u8>, String> {
            unreachable!()
        }
        fn is_writable(&self, _address: u64, _length: usize) -> Result<bool, String> {
            Ok(true)
        }
        fn resolve_offset(&self, encoded: u32) -> Result<u64, String> {
            let bases = [0x1000u64, 0x8000];
            let (segment, byte_offset) = encoded_offset_parts(encoded)?;
            checked_add(bases[segment], byte_offset)
        }
    }
    assert_eq!(Memory.resolve_offset(0x0000_0003).unwrap(), 0x100c);
    assert_eq!(Memory.resolve_offset(0x1000_0003).unwrap(), 0x4000_800c);
    assert_eq!(
        encoded_offset_parts(0xa123_4567).unwrap(),
        (10, 0x2_848d_159c)
    );
    assert_eq!(
        encoded_offset_parts(0xebd8_5054).unwrap(),
        (14, 0x3_af61_4150)
    );
    assert_eq!(
        checked_add(0x7ffb_4000_0000, 0x3_af61_4150).unwrap(),
        0x7ffe_ef61_4150
    );
    assert!(encoded_offset_parts(u32::MAX).is_err());
}

#[test]
fn path_normalization_matches_runtime_and_browser_forms() {
    assert_eq!(
        normalize_tag_path(r"\Game\Tags\Objects\Weapons\Pistol.weapon"),
        "objects/weapons/pistol"
    );
    assert_eq!(
        normalize_tag_path("/Meteorite/Content/Tags/levels/a/scenario.scenario"),
        "levels/a/scenario"
    );
    assert_eq!(normalize_tag_path("objects/a/biped"), "objects/a/biped");
}

#[test]
fn lookup_requires_path_and_group() {
    let index = RuntimeTagIndex {
        identity: RuntimeIdentity {
            process_id: 1,
            creation_time: 2,
            module_base: 3,
            tag_table: 4,
        },
        tags: vec![
            RuntimeTag {
                path: "objects/a".to_owned(),
                group_tag: 10,
                handle: 1,
                entry_address: 100,
                name_pointer: 1_000,
                root_descriptor: 124,
            },
            RuntimeTag {
                path: "objects/a".to_owned(),
                group_tag: 11,
                handle: 2,
                entry_address: 200,
                name_pointer: 2_000,
                root_descriptor: 224,
            },
        ],
    };
    assert_eq!(index.find("objects/a.weapon", 10).unwrap().handle, 1);
    assert_eq!(index.find("objects/a.weapon", 11).unwrap().handle, 2);
    assert!(index.find("objects/a.weapon", 12).is_err());
}

#[test]
fn loaded_reference_handle_uniquely_recovers_its_field_position() {
    let index = RuntimeTagIndex {
        identity: RuntimeIdentity {
            process_id: 1,
            creation_time: 2,
            module_base: 3,
            tag_table: 4,
        },
        tags: vec![
            RuntimeTag {
                path: "objects/projectiles/flak_bolt".to_owned(),
                group_tag: u32::from_be_bytes(*b"proj"),
                handle: 0x1234_0056,
                entry_address: 100,
                name_pointer: 1_000,
                root_descriptor: 124,
            },
            RuntimeTag {
                path: "objects/effects/flak".to_owned(),
                group_tag: u32::from_be_bytes(*b"effe"),
                handle: 0x4321_0065,
                entry_address: 200,
                name_pointer: 2_000,
                root_descriptor: 224,
            },
        ],
    };
    let mut live = vec![0u8; 16];
    live[8..12].copy_from_slice(&0x1234_0056u32.to_le_bytes());
    live[12..16].copy_from_slice(&0x4321_0065u32.to_le_bytes());
    assert_eq!(
        loaded_reference_positions(&live, &index, u32::from_be_bytes(*b"proj")),
        vec![8]
    );
    assert_eq!(
        loaded_reference_positions(&live, &index, u32::from_be_bytes(*b"effe")),
        vec![12]
    );
}

#[test]
fn cached_tag_address_is_rejected_when_its_entry_identity_changes() {
    let mut bytes = vec![0u8; 128];
    let entry = 32usize;
    bytes[entry..entry + 2].copy_from_slice(&0x1234u16.to_le_bytes());
    bytes[entry + TAG_ENTRY_GROUP_OFFSET as usize..entry + TAG_ENTRY_GROUP_OFFSET as usize + 4]
        .copy_from_slice(&0x7061_6577u32.to_le_bytes());
    bytes[entry + TAG_ENTRY_NAME_OFFSET as usize..entry + TAG_ENTRY_NAME_OFFSET as usize + 8]
        .copy_from_slice(&0x7788u64.to_le_bytes());
    let memory = MockMemory::new(bytes);
    let tag = RuntimeTag {
        path: "objects/a".to_owned(),
        group_tag: 0x7061_6577,
        handle: 0x1234_0007,
        entry_address: entry as u64,
        name_pointer: 0x7788,
        root_descriptor: (entry as u64) + TAG_ENTRY_ROOT_OFFSET,
    };
    validate_runtime_tag_entry(&memory, &tag).unwrap();

    memory.bytes.borrow_mut()[entry + TAG_ENTRY_GROUP_OFFSET as usize] ^= 1;
    assert_eq!(
        validate_runtime_tag_entry(&memory, &tag).unwrap_err(),
        RUNTIME_CACHE_STALE
    );
}

#[test]
fn reference_only_edit_inside_a_block_is_not_structural() {
    let mut baseline =
        TagFile::new(test_definition_path("haloreach_mcc/material_effects.json")).unwrap();
    add_block_element(&mut baseline, "effects").unwrap();
    add_block_element(&mut baseline, "effects[0]/effects").unwrap();
    apply_field_edit(
        &mut baseline,
        "effects[0]/effects[0]/tag (effect or sound)",
        "effects\\weapons\\first.effect",
    )
    .unwrap();

    let bytes = baseline.write_to_bytes().unwrap();
    let mut edited = TagFile::read_from_bytes(&bytes).unwrap();
    apply_field_edit(
        &mut edited,
        "effects[0]/effects[0]/tag (effect or sound)",
        "effects\\weapons\\second.effect",
    )
    .unwrap();

    validate_poke_structure(baseline.root(), edited.root(), "").unwrap();
}

#[test]
fn changed_block_count_is_structural() {
    let baseline =
        TagFile::new(test_definition_path("haloreach_mcc/material_effects.json")).unwrap();
    let bytes = baseline.write_to_bytes().unwrap();
    let mut edited = TagFile::read_from_bytes(&bytes).unwrap();
    add_block_element(&mut edited, "effects").unwrap();

    let error = validate_poke_structure(baseline.root(), edited.root(), "").unwrap_err();
    assert!(error.contains("structural edit cannot be poked"), "{error}");
}

#[test]
fn reorder_classifier_does_not_treat_content_edits_as_reorders() {
    assert!(!unique_permutation_reordered(&[1, 2], &[1, 3]));
    assert!(unique_permutation_reordered(&[1, 2], &[2, 1]));
    assert!(!unique_permutation_reordered(&[1, 1], &[1, 1]));
    assert!(!unique_permutation_reordered(&[1, 1], &[1, 2]));
}

#[test]
fn preflight_conflict_blocks_all_writes() {
    let memory = MockMemory::new(vec![1, 1, 0, 0, 5, 5, 0, 0, 3, 3]);
    let result = apply_transaction(&memory, &transaction_patches());
    assert!(result.unwrap_err().contains("live value conflict"));
    assert_eq!(memory.writes.get(), 0);
    assert_eq!(memory.snapshot(), vec![1, 1, 0, 0, 5, 5, 0, 0, 3, 3]);
}

#[test]
fn trusted_previous_poke_can_be_replaced_or_reverted() {
    let prior_patch = PokePatch {
        address: 0,
        expected: vec![1],
        edited: vec![2],
        field_path: "projectile".to_owned(),
    };
    let prior = last_poke_for_test(prior_patch, vec![1]);

    let memory = MockMemory::new(vec![2]);
    let mut replacement = Vec::new();
    append_patch(
        &memory,
        &mut replacement,
        Some(&prior),
        0,
        &[1],
        &[3],
        "projectile",
    )
    .unwrap();
    assert_eq!(replacement[0].expected, vec![2]);
    assert_eq!(replacement[0].edited, vec![3]);

    let mut revert = Vec::new();
    append_patch(
        &memory,
        &mut revert,
        Some(&prior),
        0,
        &[1],
        &[1],
        "projectile",
    )
    .unwrap();
    assert_eq!(revert[0].expected, vec![2]);
    assert_eq!(revert[0].edited, vec![1]);
}

#[test]
fn string_id_patch_uses_verified_runtime_values() {
    let memory = MockMemory::new(0x0000_17d2u32.to_le_bytes().to_vec());
    let string_ids = RuntimeStringIdIndex {
        by_name: HashMap::from([
            (b"warthog_d".to_vec(), 0x0000_17d2),
            (b"fork_d".to_vec(), 0x0000_1677),
        ]),
    };
    let mut patches = Vec::new();
    append_string_id_patch(
        &memory,
        &mut patches,
        None,
        &string_ids,
        0,
        "warthog_d",
        "fork_d",
        "unit/seats[0]/label",
    )
    .unwrap();

    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].expected, 0x0000_17d2u32.to_le_bytes());
    assert_eq!(patches[0].edited, 0x0000_1677u32.to_le_bytes());
}

#[test]
fn string_id_patch_rejects_unloaded_or_conflicting_values() {
    let string_ids = RuntimeStringIdIndex {
        by_name: HashMap::from([(b"warthog_d".to_vec(), 0x0000_17d2)]),
    };
    let memory = MockMemory::new(0x0000_17d2u32.to_le_bytes().to_vec());
    let missing = append_string_id_patch(
        &memory,
        &mut Vec::new(),
        None,
        &string_ids,
        0,
        "warthog_d",
        "not_loaded",
        "label",
    )
    .unwrap_err();
    assert!(missing.contains("is not registered"), "{missing}");

    let string_ids = RuntimeStringIdIndex {
        by_name: HashMap::from([
            (b"warthog_d".to_vec(), 0x0000_17d2),
            (b"fork_d".to_vec(), 0x0000_1677),
        ]),
    };
    let memory = MockMemory::new(0x0000_2222u32.to_le_bytes().to_vec());
    let conflict = append_string_id_patch(
        &memory,
        &mut Vec::new(),
        None,
        &string_ids,
        0,
        "warthog_d",
        "fork_d",
        "label",
    )
    .unwrap_err();
    assert!(conflict.contains("live value conflicts"), "{conflict}");
}

#[test]
fn string_id_names_use_engine_normalization_and_none_encoding() {
    let string_ids = RuntimeStringIdIndex {
        by_name: HashMap::from([(b"fork_d".to_vec(), 0x0000_1677)]),
    };
    assert_eq!(string_ids.resolve("FORK-D").unwrap(), 0x0000_1677);
    assert_eq!(string_ids.resolve("fork d").unwrap(), 0x0000_1677);
    assert_eq!(string_ids.resolve("").unwrap(), u32::MAX);
    assert!(string_ids.resolve(&"x".repeat(128)).is_err());
}

#[test]
fn engine_string_id_hashtable_parser_walks_nodes_and_rejects_corrupt_chains() {
    let table_address = 0x0000_0001_0000_0000u64;
    let buckets_size = STRING_ID_BUCKET_COUNT * 8;
    let nodes_start = STRING_ID_TABLE_HEADER_SIZE + buckets_size;
    let allocation_size = nodes_start + STRING_ID_MAX_ENTRIES * STRING_ID_NODE_SIZE;
    let mut table = vec![0u8; allocation_size];
    table[0..4].copy_from_slice(&(STRING_ID_BUCKET_COUNT as u32).to_le_bytes());
    table[4..8].copy_from_slice(&(STRING_ID_MAX_ENTRIES as u32).to_le_bytes());
    table[8..16].copy_from_slice(&(STRING_ID_VALUE_SIZE as u64).to_le_bytes());

    let node_address = table_address + nodes_start as u64;
    table[STRING_ID_TABLE_HEADER_SIZE..STRING_ID_TABLE_HEADER_SIZE + 8]
        .copy_from_slice(&node_address.to_le_bytes());
    table[nodes_start..nodes_start + 8].copy_from_slice(&0x0000_1677u64.to_le_bytes());
    table[nodes_start + 0x18..nodes_start + 0x1c].copy_from_slice(&0u32.to_le_bytes());

    let parsed = parse_runtime_string_id_table(table_address, &table, b"fork_d\0", 1).unwrap();
    assert_eq!(parsed.by_name.len(), 1);
    assert_eq!(parsed.resolve("fork_d").unwrap(), 0x0000_1677);

    table[nodes_start + 0x10..nodes_start + 0x18].copy_from_slice(&node_address.to_le_bytes());
    let error = parse_runtime_string_id_table(table_address, &table, b"fork_d\0", 1).unwrap_err();
    assert!(error.contains("corrupt chain"), "{error}");
}

#[test]
fn previous_poke_does_not_allow_an_unrelated_live_value() {
    let prior = last_poke_for_test(
        PokePatch {
            address: 0,
            expected: vec![1],
            edited: vec![2],
            field_path: "projectile".to_owned(),
        },
        vec![1],
    );
    let memory = MockMemory::new(vec![4]);
    let error = append_patch(
        &memory,
        &mut Vec::new(),
        Some(&prior),
        0,
        &[1],
        &[3],
        "projectile",
    )
    .unwrap_err();
    assert!(error.contains("conflicts with the shipped tag"), "{error}");
}

#[test]
fn already_edited_values_are_idempotent_and_undo_is_safe() {
    let memory = MockMemory::new(vec![1, 1, 0, 0, 8, 8, 0, 0, 3, 3]);
    let patches = transaction_patches();
    let (originals, report) = apply_transaction(&memory, &patches).unwrap();
    assert_eq!(report.written_fields, 2);
    assert_eq!(report.skipped_fields, 1);
    assert_eq!(memory.snapshot(), vec![9, 9, 0, 0, 8, 8, 0, 0, 7, 7]);
    let undo = undo_transaction(&memory, &patches, &originals).unwrap();
    assert_eq!(undo.written_fields, 2);
    assert_eq!(memory.snapshot(), vec![1, 1, 0, 0, 8, 8, 0, 0, 3, 3]);
}

#[test]
fn every_forward_write_failure_rolls_back_in_reverse() {
    let original = vec![1, 1, 0, 0, 2, 2, 0, 0, 3, 3];
    for fail_at in 0..3 {
        let memory = MockMemory::new(original.clone());
        memory.fail_once_at(fail_at);
        let error = apply_transaction(&memory, &transaction_patches()).unwrap_err();
        assert!(error.contains("rolled back"), "{error}");
        assert_eq!(memory.snapshot(), original, "failure at write {fail_at}");
    }
}

#[test]
fn undo_failure_reapplies_already_restored_bytes() {
    let original = vec![1, 1, 0, 0, 2, 2, 0, 0, 3, 3];
    let memory = MockMemory::new(original);
    let patches = transaction_patches();
    let (originals, _) = apply_transaction(&memory, &patches).unwrap();
    let edited = memory.snapshot();
    memory.fail_once_at(1);
    let error = undo_transaction(&memory, &patches, &originals).unwrap_err();
    assert!(error.contains("re-applied"), "{error}");
    assert_eq!(memory.snapshot(), edited);
}

#[cfg(windows)]
#[test]
fn windows_process_memory_smoke_uses_a_controlled_local_buffer() {
    use std::ffi::c_void;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_OPERATION,
        PROCESS_VM_READ, PROCESS_VM_WRITE,
    };

    let mut controlled = [1u8, 2, 3, 4];
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION
                | PROCESS_VM_READ
                | PROCESS_VM_WRITE
                | PROCESS_VM_OPERATION,
            false,
            GetCurrentProcessId(),
        )
        .unwrap()
    };
    let mut read_back = [0u8; 4];
    unsafe {
        ReadProcessMemory(
            handle,
            controlled.as_ptr().cast::<c_void>(),
            read_back.as_mut_ptr().cast::<c_void>(),
            read_back.len(),
            None,
        )
        .unwrap();
    }
    assert_eq!(read_back, controlled);
    let replacement = [9u8, 8, 7, 6];
    unsafe {
        WriteProcessMemory(
            handle,
            controlled.as_mut_ptr().cast::<c_void>(),
            replacement.as_ptr().cast::<c_void>(),
            replacement.len(),
            None,
        )
        .unwrap();
    }
    assert_eq!(controlled, replacement);
    unsafe {
        CloseHandle(handle).unwrap();
    }
}

#[cfg(windows)]
#[test]
#[ignore = "requires the exact supported Campaign Evolved CU2 build with a mission loaded"]
fn manual_cu2_enumerates_live_tags_without_writing() {
    let count = platform::manual_read_only_discovery().unwrap();
    assert!(count > 0);
}

#[cfg(windows)]
#[test]
#[ignore = "requires the exact supported Campaign Evolved CU2 build with a mission loaded"]
fn manual_cu2_reports_a_live_root_descriptor_without_writing() {
    for path in [
        "objects/weapons/rifle/assault_rifle/assault_rifle",
        "objects/weapons/support_high/rocket_launcher/projectiles/rocket_launcher_rocket",
    ] {
        eprintln!(
            "{path}\n{}",
            platform::manual_read_only_root_diagnostic(path).unwrap()
        );
    }
}

#[cfg(windows)]
#[test]
#[ignore = "requires the exact supported Campaign Evolved CU2 build with a mission loaded"]
fn manual_cu2_reuses_validated_runtime_address_cache_without_writing() {
    eprintln!("{}", platform::manual_read_only_cache_diagnostic().unwrap());
}

#[cfg(windows)]
#[test]
#[ignore = "requires the exact supported Campaign Evolved CU2 build with a mission loaded"]
fn manual_cu2_reads_and_cross_checks_engine_string_id_registry_without_writing() {
    eprintln!(
        "{}",
        platform::manual_read_only_string_id_index_diagnostic().unwrap()
    );
}

#[cfg(windows)]
#[test]
#[ignore = "temporarily writes and immediately restores the live CU2 assault-rifle projectile reference"]
fn manual_cu2_pokes_and_undoes_assault_rifle_projectile_reference() {
    fn find_reference_path(st: TagStruct<'_>, parent: &str, wanted_name: &str) -> Option<String> {
        for field in st.fields_all() {
            let path = field_path(parent, field);
            if field.field_type() == TagFieldType::TagReference
                && field.clean_name().eq_ignore_ascii_case(wanted_name)
            {
                return Some(path);
            }
            if let Some(nested) = field.as_struct()
                && let Some(found) = find_reference_path(nested, &path, wanted_name)
            {
                return Some(found);
            }
            if let Some(array) = field.as_array() {
                for index in 0..array.len() {
                    if let Some(found) = find_reference_path(
                        array.element(index).unwrap(),
                        &format!("{path}[{index}]"),
                        wanted_name,
                    ) {
                        return Some(found);
                    }
                }
            }
            if let Some(block) = field.as_block() {
                for index in 0..block.len() {
                    if let Some(found) = find_reference_path(
                        block.element(index).unwrap(),
                        &format!("{path}[{index}]"),
                        wanted_name,
                    ) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    let paks = std::env::var_os("CE_PAKS")
        .map(PathBuf::from)
        .expect("set CE_PAKS to Meteorite/Content/Paks");
    let definitions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = TagNameIndex::load_from_definitions(&definitions);
    let loaded = crate::source::load_iostore_container_set(paks, &names, &definitions)
        .expect("mount Campaign Evolved containers");
    let weapon_group = u32::from_be_bytes(*b"weap");
    let entry = loaded
        .entries
        .iter()
        .find(|entry| {
            entry.group_tag == weapon_group
                && normalize_tag_path(&entry.display_path)
                    == "objects/weapons/rifle/assault_rifle/assault_rifle"
        })
        .cloned()
        .expect("find assault_rifle.weapon");
    let mut edited = read_entry(&loaded.source, &entry).expect("read shipped assault rifle");
    let projectile_path = find_reference_path(edited.root(), "", "projectile")
        .expect("find projectile tag-reference field");
    apply_field_edit(
        &mut edited,
        &projectile_path,
        "objects/weapons/support_high/rocket_launcher/projectiles/rocket_launcher_rocket.projectile",
    )
    .expect("edit projectile reference");
    let edited_bytes = edited.write_to_bytes().expect("snapshot edited tag");
    let first_plan = platform::prepare(
        loaded.source.clone(),
        loaded.entries.clone(),
        entry.clone(),
        edited_bytes,
        None,
    )
    .expect("build live poke plan");
    assert_eq!(
        first_plan.patches.len(),
        1,
        "expected one reference-handle patch"
    );
    assert!(
        first_plan.patches[0].field_path.contains("projectile"),
        "unexpected patch: {}",
        first_plan.patches[0].field_path
    );

    let (first_last, first_report) =
        platform::execute(first_plan).expect("write or verify first projectile reference");

    apply_field_edit(
        &mut edited,
        &projectile_path,
        "objects/characters/hunter/hunter_fuel_rod/projectiles/hunter_fuel_rod_bolt.projectile",
    )
    .expect("edit projectile reference a second time");
    let second_bytes = edited.write_to_bytes().expect("snapshot second edit");
    let second_plan = match platform::prepare(
        loaded.source.clone(),
        loaded.entries.clone(),
        entry,
        second_bytes,
        Some(first_last.clone()),
    ) {
        Ok(plan) => plan,
        Err(error) => {
            platform::undo(first_last).expect("restore first poke after second preflight failure");
            panic!("build chained live poke plan: {error}");
        }
    };
    assert_eq!(
        second_plan.patches.len(),
        1,
        "expected one chained reference-handle patch"
    );
    let (second_last, second_report) = match platform::execute(second_plan) {
        Ok(result) => result,
        Err(error) => {
            platform::undo(first_last).expect("restore first poke after second write failure");
            panic!("write and verify second projectile reference: {error}");
        }
    };
    let undo_report = platform::undo(second_last)
        .expect("restore and verify the projectile reference from before the chain");
    assert!(first_report.written_fields <= 1);
    assert_eq!(second_report.written_fields, 1);
    assert_eq!(undo_report.written_fields, 1);
}

#[cfg(windows)]
#[test]
#[ignore = "requires Campaign Evolved containers and the exact supported CU2 build"]
fn manual_cu2_pokes_and_undoes_loaded_string_id() {
    use std::time::Instant;

    let paks = std::env::var_os("CE_PAKS")
        .map(PathBuf::from)
        .expect("set CE_PAKS to Meteorite/Content/Paks");
    let definitions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = TagNameIndex::load_from_definitions(&definitions);
    let loaded = crate::source::load_iostore_container_set(paks, &names, &definitions)
        .expect("mount Campaign Evolved containers");
    let vehicle_group = u32::from_be_bytes(*b"vehi");
    let entry = loaded
        .entries
        .iter()
        .find(|entry| {
            entry.group_tag == vehicle_group
                && normalize_tag_path(&entry.display_path)
                    == "objects/vehicles/human/warthog/warthog"
        })
        .cloned()
        .expect("find warthog vehicle tag");
    let baseline = read_entry(&loaded.source, &entry).expect("read warthog vehicle tag");
    let original_bytes = baseline
        .write_to_bytes()
        .expect("snapshot shipped warthog tag");
    let mut edited = TagFile::read_from_bytes(&original_bytes).expect("copy warthog vehicle tag");
    apply_field_edit(&mut edited, "unit#0/seats#79[0]/label#1", "fork_d")
        .expect("edit warthog driver seat label");

    platform::clear_runtime_cache();
    let started = Instant::now();
    let plan = platform::prepare(
        loaded.source.clone(),
        loaded.entries.clone(),
        entry.clone(),
        edited.write_to_bytes().expect("snapshot warthog tag"),
        None,
    )
    .expect("prepare warthog string-id poke");
    let preflight = started.elapsed();
    assert_eq!(plan.patches.len(), 1);
    assert_eq!(plan.byte_count(), 4);
    assert_eq!(plan.patches[0].expected, 0x0000_17d2u32.to_le_bytes());
    assert_eq!(plan.patches[0].edited, 0x0000_1677u32.to_le_bytes());

    let (first_last, first_report) =
        platform::execute(plan).expect("poke warthog driver seat label");
    let revert = platform::prepare(
        loaded.source.clone(),
        loaded.entries.clone(),
        entry,
        original_bytes,
        Some(first_last),
    )
    .expect("prepare chained string-id revert");
    assert_eq!(revert.patches.len(), 1);
    assert_eq!(revert.patches[0].expected, 0x0000_1677u32.to_le_bytes());
    assert_eq!(revert.patches[0].edited, 0x0000_17d2u32.to_le_bytes());
    let (last, revert_report) =
        platform::execute(revert).expect("restore warthog driver seat label");
    let undo = platform::undo(last).expect("verify chained string-id undo");
    assert_eq!(first_report.written_fields, 1);
    assert_eq!(revert_report.written_fields, 1);
    assert_eq!(undo.written_fields, 0);
    eprintln!("cold string-id preflight: {}ms", preflight.as_millis());
}
