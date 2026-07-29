//! Campaign Evolved CU2 runtime poking.
//!
//! This module intentionally implements constrained patching, not tag
//! injection. It can alter bytes in allocations the game already owns, but it
//! never allocates remote memory, changes page protection, resizes a block or
//! data payload, or registers a new tag.

use super::*;

const PROCESS_NAME: &str = "HaloCampaignEvolved.exe";
const TAG_DLL_NAME: &str = "HaloSimulation_tag_release.dll";
const TAG_ENTRY_SIZE: usize = 0x30;
const TAG_ENTRY_GROUP_OFFSET: u64 = 0x04;
const TAG_ENTRY_NAME_OFFSET: u64 = 0x10;
const TAG_ENTRY_ROOT_OFFSET: u64 = 0x18;
const MAX_RUNTIME_TAGS: usize = 0x1_0000;
const MAX_RUNTIME_NAME: usize = 4096;
const RUNTIME_PAGE_SIZE: u64 = 0x1000;
const MAX_RUNTIME_NAME_POOL_SPAN: usize = 64 * 1024 * 1024;
const STRING_ID_BUCKET_COUNT: usize = 1_046_528;
const STRING_ID_MAX_ENTRIES: usize = 523_264;
const STRING_ID_VALUE_SIZE: usize = 4;
const STRING_ID_TABLE_HEADER_SIZE: usize = 0x38;
const STRING_ID_NODE_SIZE: usize = 0x1c;
const STRING_ID_STORAGE_CAPACITY: usize = 26_163_200;
const STRING_ID_MAX_NAME_BYTES: usize = 127;
const STRING_ID_BUILTIN_COUNT: usize = 2_678;
const STRING_ID_SET_ZERO_BUILTIN_COUNT: u32 = 1_068;

#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RuntimeBuildProfile {
    pub(in crate::app) label: &'static str,
    host_sha256: &'static str,
    dll_sha256: &'static str,
    tag_table_pointer_rva: u64,
    segment_table_rva: u64,
    string_id_storage_rva: u64,
    string_id_storage_used_rva: u64,
    string_id_strings_rva: u64,
    string_id_count_rva: u64,
    string_id_mapping_table_rva: u64,
    string_id_builtin_table_rva: u64,
}

const CU2_PROFILE: RuntimeBuildProfile = RuntimeBuildProfile {
    label: "Steam CU2 2026.06.26.1097863.1",
    host_sha256: "0670FAA751E2553940B90DF6BE43D3B0FF59EA87F22155CF3C3FE9D439367F1D",
    dll_sha256: "8EE1A37F6F0BC89241F47946546EDCA798962F81E2D06B386196BC75DE991705",
    tag_table_pointer_rva: 0x0182_E1E8,
    segment_table_rva: 0x02C2_DCC0,
    string_id_storage_rva: 0x0135_8470,
    string_id_storage_used_rva: 0x0135_8478,
    string_id_strings_rva: 0x0135_8480,
    string_id_count_rva: 0x0135_8488,
    string_id_mapping_table_rva: 0x0135_84A0,
    string_id_builtin_table_rva: 0x0083_0060,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeIdentity {
    process_id: u32,
    creation_time: u64,
    module_base: u64,
    tag_table: u64,
}

#[derive(Clone, Debug)]
struct RuntimeTag {
    path: String,
    group_tag: u32,
    handle: u32,
    entry_address: u64,
    name_pointer: u64,
    root_descriptor: u64,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct RuntimeTagIndex {
    identity: RuntimeIdentity,
    tags: Vec<RuntimeTag>,
}

#[derive(Clone, Debug)]
struct RuntimeStringIdIndex {
    by_name: HashMap<Vec<u8>, u32>,
}

impl RuntimeStringIdIndex {
    fn resolve(&self, name: &str) -> Result<u32, String> {
        let Some(normalized) = normalize_string_id_name(name)? else {
            return Ok(u32::MAX);
        };
        self.by_name.get(&normalized).copied().ok_or_else(|| {
            let normalized = String::from_utf8_lossy(&normalized);
            format!(
                "'{normalized}' is not registered in the running game; it cannot be poked (restart into a build that loads it)"
            )
        })
    }
}

impl RuntimeTagIndex {
    fn find(&self, path: &str, group_tag: u32) -> Result<&RuntimeTag, String> {
        let path = normalize_tag_path(path);
        let mut matches = self
            .tags
            .iter()
            .filter(|tag| tag.group_tag == group_tag && normalize_tag_path(&tag.path) == path);
        let first = matches
            .next()
            .ok_or_else(|| "tag is not loaded".to_owned())?;
        if matches.next().is_some() {
            return Err(format!(
                "runtime tag lookup is ambiguous for {} ({})",
                path,
                format_group_tag(group_tag)
            ));
        }
        Ok(first)
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct PokePatch {
    pub(in crate::app) address: u64,
    pub(in crate::app) expected: Vec<u8>,
    pub(in crate::app) edited: Vec<u8>,
    pub(in crate::app) field_path: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct PokePlan {
    profile: RuntimeBuildProfile,
    identity: RuntimeIdentity,
    tag_path: String,
    group_tag: u32,
    tag_handle: u32,
    tag_entry_address: u64,
    tag_name_pointer: u64,
    root_address: u64,
    pub(in crate::app) patches: Vec<PokePatch>,
    chain_originals: Vec<Option<Vec<u8>>>,
}

impl PokePlan {
    pub(in crate::app) fn byte_count(&self) -> usize {
        self.patches.iter().map(|patch| patch.edited.len()).sum()
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastPoke {
    plan: PokePlan,
    originals: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct PokeReport {
    pub(in crate::app) written_fields: usize,
    pub(in crate::app) written_bytes: usize,
    pub(in crate::app) skipped_fields: usize,
    pub(in crate::app) message: String,
}

impl PokeReport {
    fn status(&self) -> String {
        format!(
            "{}: {} field(s), {} byte(s), {} no-op(s)",
            self.message, self.written_fields, self.written_bytes, self.skipped_fields
        )
    }
}

pub(in crate::app) struct PokeDialog {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    pub(in crate::app) state: PokeDialogState,
}

pub(in crate::app) enum PokeDialogState {
    Scanning,
    Ready(PokePlan),
    Writing,
    Error(String),
}

struct PokeRequest {
    kit: KitId,
    key: String,
    source: TagSource,
    entries: Vec<TagEntry>,
    entry: TagEntry,
    edited_bytes: Vec<u8>,
    prior: Option<LastPoke>,
}

trait RuntimeMemory {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String>;
    fn is_writable(&self, address: u64, length: usize) -> Result<bool, String>;
    fn resolve_offset(&self, encoded: u32) -> Result<u64, String>;
}

trait RuntimeWriteMemory: RuntimeMemory {
    fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String>;
}

fn checked_add(address: u64, offset: u64) -> Result<u64, String> {
    address
        .checked_add(offset)
        .ok_or_else(|| "runtime pointer arithmetic overflow".to_owned())
}

fn checked_index(base: u64, index: usize, stride: usize) -> Result<u64, String> {
    let offset = index
        .checked_mul(stride)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "runtime pointer arithmetic overflow".to_owned())?;
    checked_add(base, offset)
}

fn runtime_name_pool_bounds(pointers: &[u64]) -> Result<Option<(u64, usize)>, String> {
    let Some(minimum) = pointers.iter().copied().min() else {
        return Ok(None);
    };
    let maximum = pointers.iter().copied().max().unwrap();
    if minimum == 0 {
        return Ok(None);
    }
    let start = minimum & !(RUNTIME_PAGE_SIZE - 1);
    let end = checked_add(maximum, MAX_RUNTIME_NAME as u64)?
        .checked_add(RUNTIME_PAGE_SIZE - 1)
        .ok_or_else(|| "runtime pointer arithmetic overflow".to_owned())?
        & !(RUNTIME_PAGE_SIZE - 1);
    let length = usize::try_from(
        end.checked_sub(start)
            .ok_or_else(|| "runtime pointer arithmetic overflow".to_owned())?,
    )
    .map_err(|_| "runtime name pool is too large".to_owned())?;
    Ok((length <= MAX_RUNTIME_NAME_POOL_SPAN).then_some((start, length)))
}

fn runtime_name_from_pool(pool_start: u64, pool: &[u8], pointer: u64) -> Result<String, String> {
    let offset = usize::try_from(
        pointer
            .checked_sub(pool_start)
            .ok_or_else(|| "runtime tag name pointer is outside the name pool".to_owned())?,
    )
    .map_err(|_| "runtime tag name pointer is outside the name pool".to_owned())?;
    let tail = pool
        .get(offset..)
        .ok_or_else(|| "runtime tag name pointer is outside the name pool".to_owned())?;
    let end = tail
        .iter()
        .take(MAX_RUNTIME_NAME)
        .position(|byte| *byte == 0)
        .ok_or_else(|| "runtime tag name is not terminated".to_owned())?;
    std::str::from_utf8(&tail[..end])
        .map(str::to_owned)
        .map_err(|_| "runtime tag name is not UTF-8".to_owned())
}

fn encoded_offset_parts(encoded: u32) -> Result<(usize, u64), String> {
    if encoded == u32::MAX {
        return Err("null encoded runtime offset".to_owned());
    }
    Ok((
        (encoded >> 28) as usize,
        u64::from(encoded)
            .checked_mul(4)
            .ok_or_else(|| "runtime pointer arithmetic overflow".to_owned())?,
    ))
}

fn read_u32(memory: &dyn RuntimeMemory, address: u64) -> Result<u32, String> {
    let bytes = memory.read(address, 4)?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("four bytes requested"),
    ))
}

fn read_u64(memory: &dyn RuntimeMemory, address: u64) -> Result<u64, String> {
    let bytes = memory.read(address, 8)?;
    Ok(u64::from_le_bytes(
        bytes.try_into().expect("eight bytes requested"),
    ))
}

const RUNTIME_CACHE_STALE: &str = "runtime address cache is stale";

fn should_refresh_runtime_index(error: &str) -> bool {
    error.starts_with(RUNTIME_CACHE_STALE) || error == "tag is not loaded"
}

fn loaded_reference_positions(live: &[u8], index: &RuntimeTagIndex, group_tag: u32) -> Vec<usize> {
    live.chunks_exact(4)
        .enumerate()
        .filter_map(|(index_in_field, chunk)| {
            let handle = u32::from_le_bytes(chunk.try_into().unwrap());
            index
                .tags
                .iter()
                .any(|tag| tag.handle == handle && tag.group_tag == group_tag)
                .then_some(index_in_field * 4)
        })
        .collect()
}

fn validate_runtime_tag_entry(memory: &dyn RuntimeMemory, tag: &RuntimeTag) -> Result<(), String> {
    let entry = memory
        .read(tag.entry_address, TAG_ENTRY_ROOT_OFFSET as usize)
        .map_err(|error| format!("{RUNTIME_CACHE_STALE}: {error}"))?;
    let salt = u16::from_le_bytes(entry[0..2].try_into().unwrap());
    let group_offset = TAG_ENTRY_GROUP_OFFSET as usize;
    let name_offset = TAG_ENTRY_NAME_OFFSET as usize;
    let group_tag = u32::from_le_bytes(entry[group_offset..group_offset + 4].try_into().unwrap());
    let name_pointer = u64::from_le_bytes(entry[name_offset..name_offset + 8].try_into().unwrap());
    if (u32::from(salt) << 16) != tag.handle & 0xffff_0000
        || group_tag != tag.group_tag
        || name_pointer != tag.name_pointer
    {
        return Err(RUNTIME_CACHE_STALE.to_owned());
    }
    Ok(())
}

fn normalize_tag_path(path: &str) -> String {
    let mut normalized = path
        .trim_matches('\0')
        .trim()
        .replace('\\', "/")
        .to_ascii_lowercase();
    while normalized.starts_with('/') {
        normalized.remove(0);
    }
    loop {
        let mut stripped = false;
        for prefix in ["meteorite/content/", "content/", "game/tags/", "tags/"] {
            if let Some(rest) = normalized.strip_prefix(prefix) {
                normalized = rest.to_owned();
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    if let Some((stem, extension)) = normalized.rsplit_once('/')
        && let Some((file, _)) = extension.rsplit_once('.')
    {
        normalized = format!("{stem}/{file}");
    } else if let Some((file, _)) = normalized.rsplit_once('.') {
        normalized = file.to_owned();
    }
    normalized.trim_matches('/').to_owned()
}

fn field_path(parent: &str, field: TagField<'_>) -> String {
    let name = field.display_name();
    let component = if name.is_empty() {
        format!("{}#{}", field.type_name(), field.ordinal())
    } else {
        format!("{}#{}", name, field.ordinal())
    };
    if parent.is_empty() {
        component
    } else {
        format!("{parent}/{component}")
    }
}

fn field_span(st: TagStruct<'_>, field: TagField<'_>) -> Result<(usize, usize), String> {
    let start = field.definition().offset() as usize;
    if start > st.size() {
        return Err(format!("schema offset is outside {}", st.name()));
    }
    let end = st
        .fields_all()
        .map(|other| other.definition().offset() as usize)
        .filter(|&offset| offset > start)
        .min()
        .unwrap_or(st.size());
    if end < start || end > st.size() {
        return Err(format!("schema field size is invalid in {}", st.name()));
    }
    Ok((start, end - start))
}

fn raw_span<'a>(st: TagStruct<'a>, start: usize, length: usize) -> Result<&'a [u8], String> {
    st.raw()
        .get(start..start.saturating_add(length))
        .ok_or_else(|| format!("tag data for {} is truncated", st.name()))
}

fn probe_inline_scalars(
    memory: &dyn RuntimeMemory,
    baseline: TagStruct<'_>,
    live_address: u64,
    parent_path: &str,
    compared: &mut usize,
) -> Result<(), String> {
    for field in baseline.fields_all() {
        if *compared >= 2 {
            break;
        }
        let path = field_path(parent_path, field);
        let (offset, span) = field_span(baseline, field)?;
        let field_address = checked_add(live_address, offset as u64)?;
        match field.field_type() {
            TagFieldType::Struct => {
                if let Some(nested) = field.as_struct() {
                    probe_inline_scalars(memory, nested, field_address, &path, compared)?;
                }
            }
            TagFieldType::Array => {
                if let Some(array) = field.as_array() {
                    let element_size = array.definition().struct_definition().size();
                    for index in 0..array.len() {
                        if *compared >= 2 {
                            break;
                        }
                        let element_address = checked_index(field_address, index, element_size)?;
                        probe_inline_scalars(
                            memory,
                            array.element(index).unwrap(),
                            element_address,
                            &format!("{path}[{index}]"),
                            compared,
                        )?;
                    }
                }
            }
            TagFieldType::Block
            | TagFieldType::Data
            | TagFieldType::TagReference
            | TagFieldType::StringId
            | TagFieldType::OldStringId
            | TagFieldType::PageableResource
            | TagFieldType::ApiInterop
            | TagFieldType::VertexBuffer
            | TagFieldType::Pointer
            | TagFieldType::NonCacheRuntimeValue
            | TagFieldType::Custom
            | TagFieldType::Unknown
            | TagFieldType::Pad
            | TagFieldType::UselessPad
            | TagFieldType::Skip
            | TagFieldType::Explanation
            | TagFieldType::Terminator => {}
            _ => {
                let shipped = raw_span(baseline, offset, span)?;
                if shipped.is_empty() {
                    continue;
                }
                let live = memory.read(field_address, span)?;
                if live != shipped {
                    return Err(format!(
                        "live scalar probe does not match the shipped tag at {path}"
                    ));
                }
                *compared += 1;
            }
        }
    }
    Ok(())
}

fn field_debug_value(field: TagField<'_>) -> String {
    format!("{:?}", field.value())
}

fn string_id_text(field: TagField<'_>) -> Result<String, String> {
    match field.value() {
        Some(TagFieldData::StringId(value)) | Some(TagFieldData::OldStringId(value)) => {
            Ok(value.string)
        }
        _ => Err(format!(
            "missing string-id value at {}",
            field.display_name()
        )),
    }
}

fn normalize_string_id_bytes(name: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if name.is_empty() {
        return Ok(None);
    }
    if name.len() > STRING_ID_MAX_NAME_BYTES {
        return Err(format!(
            "string id name is longer than {STRING_ID_MAX_NAME_BYTES} bytes"
        ));
    }
    let mut bytes = name.to_vec();
    for byte in &mut bytes {
        *byte = match *byte {
            b'A'..=b'Z' => *byte + (b'a' - b'A'),
            b' ' | b'-' => b'_',
            byte => byte,
        };
    }
    Ok(Some(bytes))
}

fn normalize_string_id_name(name: &str) -> Result<Option<Vec<u8>>, String> {
    normalize_string_id_bytes(name.as_bytes())
}

fn string_id_storage_name(storage: &[u8], offset: u32) -> Result<&[u8], String> {
    let offset =
        usize::try_from(offset).map_err(|_| "string id storage offset overflow".to_owned())?;
    let tail = storage
        .get(offset..)
        .ok_or_else(|| "string id storage offset is outside the name blob".to_owned())?;
    let length = tail
        .iter()
        .take(STRING_ID_MAX_NAME_BYTES + 1)
        .position(|byte| *byte == 0)
        .ok_or_else(|| "runtime string id name is not terminated within 128 bytes".to_owned())?;
    Ok(&tail[..length])
}

fn parse_runtime_string_id_table(
    table_address: u64,
    table: &[u8],
    storage: &[u8],
    expected_count: usize,
) -> Result<RuntimeStringIdIndex, String> {
    if table.len() < STRING_ID_TABLE_HEADER_SIZE {
        return Err("runtime string id mapping table header is truncated".to_owned());
    }
    let bucket_count = u32::from_le_bytes(table[0..4].try_into().unwrap()) as usize;
    let max_entries = u32::from_le_bytes(table[4..8].try_into().unwrap()) as usize;
    let value_size = u64::from_le_bytes(table[8..16].try_into().unwrap()) as usize;
    if bucket_count != STRING_ID_BUCKET_COUNT
        || max_entries != STRING_ID_MAX_ENTRIES
        || value_size != STRING_ID_VALUE_SIZE
    {
        return Err("unsupported runtime string id mapping-table layout".to_owned());
    }
    if expected_count == 0 {
        return Err("runtime string id registry is not initialized".to_owned());
    }
    if expected_count > max_entries {
        return Err("runtime string id count exceeds the mapping-table capacity".to_owned());
    }
    let buckets_size = bucket_count
        .checked_mul(8)
        .ok_or_else(|| "runtime string id table size overflow".to_owned())?;
    let nodes_start = STRING_ID_TABLE_HEADER_SIZE
        .checked_add(buckets_size)
        .ok_or_else(|| "runtime string id table size overflow".to_owned())?;
    let allocation_size = nodes_start
        .checked_add(
            max_entries
                .checked_mul(STRING_ID_NODE_SIZE)
                .ok_or_else(|| "runtime string id table size overflow".to_owned())?,
        )
        .ok_or_else(|| "runtime string id table size overflow".to_owned())?;
    if table.len() != allocation_size {
        return Err("runtime string id mapping-table allocation is truncated".to_owned());
    }

    let mut visited = HashSet::with_capacity(expected_count);
    let mut by_name = HashMap::with_capacity(expected_count);
    for bucket in 0..bucket_count {
        let bucket_offset = STRING_ID_TABLE_HEADER_SIZE + bucket * 8;
        let mut node_address =
            u64::from_le_bytes(table[bucket_offset..bucket_offset + 8].try_into().unwrap());
        while node_address != 0 {
            if visited.len() >= max_entries || !visited.insert(node_address) {
                return Err("runtime string id mapping table contains a corrupt chain".to_owned());
            }
            let node_offset = usize::try_from(
                node_address
                    .checked_sub(table_address)
                    .ok_or_else(|| "runtime string id node precedes its allocation".to_owned())?,
            )
            .map_err(|_| "runtime string id node offset overflow".to_owned())?;
            if node_offset < nodes_start
                || (node_offset - nodes_start) % STRING_ID_NODE_SIZE != 0
                || node_offset
                    .checked_add(STRING_ID_NODE_SIZE)
                    .is_none_or(|end| end > table.len())
            {
                return Err("runtime string id node is outside its allocation".to_owned());
            }
            let key = u64::from_le_bytes(table[node_offset..node_offset + 8].try_into().unwrap());
            let id = u32::try_from(key)
                .map_err(|_| "runtime string id mapping key exceeds 32 bits".to_owned())?;
            node_address = u64::from_le_bytes(
                table[node_offset + 0x10..node_offset + 0x18]
                    .try_into()
                    .unwrap(),
            );
            let storage_offset = u32::from_le_bytes(
                table[node_offset + 0x18..node_offset + 0x1c]
                    .try_into()
                    .unwrap(),
            );
            let name = string_id_storage_name(storage, storage_offset).map_err(|error| {
                let offset = storage_offset as usize;
                let preview = storage
                    .get(offset..offset.saturating_add(32).min(storage.len()))
                    .unwrap_or_default();
                format!(
                    "{error} for string id 0x{id:08X} at storage offset 0x{storage_offset:08X}: {preview:02X?}"
                )
            })?;
            let normalized = normalize_string_id_bytes(name)?.unwrap_or_default();
            if normalized != name {
                let name = String::from_utf8_lossy(name);
                return Err(format!(
                    "runtime string id registry contains an unnormalized name: {name}"
                ));
            }
            if let Some(previous) = by_name.insert(normalized.clone(), id)
                && previous != id
            {
                let normalized = String::from_utf8_lossy(&normalized);
                return Err(format!(
                    "runtime string id registry maps '{normalized}' to multiple ids"
                ));
            }
        }
    }
    if visited.len() != expected_count || by_name.len() != expected_count {
        return Err(format!(
            "runtime string id registry count mismatch: expected {expected_count}, found {}",
            by_name.len()
        ));
    }
    Ok(RuntimeStringIdIndex { by_name })
}

fn resources_equal(before: TagResource<'_>, after: TagResource<'_>) -> bool {
    std::mem::discriminant(&before.kind()) == std::mem::discriminant(&after.kind())
        && before.inline_bytes() == after.inline_bytes()
        && before.exploded_payload() == after.exploded_payload()
        && before.xsync_payload() == after.xsync_payload()
        && match (before.as_struct(), after.as_struct()) {
            (Some(before), Some(after)) => structs_equal(before, after),
            (None, None) => true,
            _ => false,
        }
}

fn structs_equal(before: TagStruct<'_>, after: TagStruct<'_>) -> bool {
    if before.size() != after.size() || before.raw() != after.raw() {
        return false;
    }
    let before_fields: Vec<_> = before.fields_all().collect();
    let after_fields: Vec<_> = after.fields_all().collect();
    if before_fields.len() != after_fields.len() {
        return false;
    }
    before_fields.into_iter().zip(after_fields).all(
        |(before_field, after_field)| match before_field.field_type() {
            TagFieldType::Struct => match (before_field.as_struct(), after_field.as_struct()) {
                (Some(before), Some(after)) => structs_equal(before, after),
                (None, None) => true,
                _ => false,
            },
            TagFieldType::Array => match (before_field.as_array(), after_field.as_array()) {
                (Some(before), Some(after)) if before.len() == after.len() => (0..before.len())
                    .all(|index| {
                        structs_equal(
                            before.element(index).unwrap(),
                            after.element(index).unwrap(),
                        )
                    }),
                (None, None) => true,
                _ => false,
            },
            TagFieldType::Block => match (before_field.as_block(), after_field.as_block()) {
                (Some(before), Some(after)) if before.len() == after.len() => (0..before.len())
                    .all(|index| {
                        structs_equal(
                            before.element(index).unwrap(),
                            after.element(index).unwrap(),
                        )
                    }),
                (None, None) => true,
                _ => false,
            },
            TagFieldType::Data => before_field.as_data() == after_field.as_data(),
            TagFieldType::PageableResource => {
                match (before_field.as_resource(), after_field.as_resource()) {
                    (Some(before), Some(after)) => resources_equal(before, after),
                    (None, None) => true,
                    _ => false,
                }
            }
            _ => field_debug_value(before_field) == field_debug_value(after_field),
        },
    )
}

fn unique_permutation_reordered(before: &[u64], after: &[u64]) -> bool {
    if before.len() != after.len() || before == after {
        return false;
    }
    let before_unique: HashSet<u64> = before.iter().copied().collect();
    let after_unique: HashSet<u64> = after.iter().copied().collect();
    before_unique.len() == before.len()
        && after_unique.len() == after.len()
        && before_unique == after_unique
}

fn validate_poke_structure(
    baseline: TagStruct<'_>,
    edited: TagStruct<'_>,
    parent_path: &str,
) -> Result<(), String> {
    if baseline.size() != edited.size() {
        return Err(format!("structural edit cannot be poked at {parent_path}"));
    }
    let baseline_fields: Vec<_> = baseline.fields_all().collect();
    let edited_fields: Vec<_> = edited.fields_all().collect();
    if baseline_fields.len() != edited_fields.len() {
        return Err(format!("schema mismatch at {parent_path}"));
    }
    for (before_field, after_field) in baseline_fields.into_iter().zip(edited_fields) {
        if before_field.field_type() != after_field.field_type()
            || before_field.definition().offset() != after_field.definition().offset()
        {
            return Err(format!("schema mismatch at {parent_path}"));
        }
        let path = field_path(parent_path, before_field);
        match before_field.field_type() {
            TagFieldType::Struct => {
                let before = before_field
                    .as_struct()
                    .ok_or_else(|| format!("missing shipped struct at {path}"))?;
                let after = after_field
                    .as_struct()
                    .ok_or_else(|| format!("missing edited struct at {path}"))?;
                validate_poke_structure(before, after, &path)?;
            }
            TagFieldType::Array => {
                let before = before_field
                    .as_array()
                    .ok_or_else(|| format!("missing shipped fixed array at {path}"))?;
                let after = after_field
                    .as_array()
                    .ok_or_else(|| format!("missing edited fixed array at {path}"))?;
                if before.len() != after.len() {
                    return Err(format!("structural edit cannot be poked at {path}"));
                }
                for index in 0..before.len() {
                    validate_poke_structure(
                        before.element(index).unwrap(),
                        after.element(index).unwrap(),
                        &format!("{path}[{index}]"),
                    )?;
                }
            }
            TagFieldType::Block => {
                let before = before_field.as_block();
                let after = after_field.as_block();
                let before_len = before.map_or(0, |block| block.len());
                let after_len = after.map_or(0, |block| block.len());
                if before_len != after_len {
                    return Err(format!("structural edit cannot be poked at {path}"));
                }
                let (Some(before), Some(after)) = (before, after) else {
                    continue;
                };
                let before_full: Vec<u64> = (0..before_len)
                    .map(|index| element_fingerprint(before.element(index)))
                    .collect();
                let after_full: Vec<u64> = (0..after_len)
                    .map(|index| element_fingerprint(after.element(index)))
                    .collect();
                let before_shallow: Vec<u64> = (0..before_len)
                    .map(|index| shallow_fingerprint(before.element(index)))
                    .collect();
                let after_shallow: Vec<u64> = (0..after_len)
                    .map(|index| shallow_fingerprint(after.element(index)))
                    .collect();
                if unique_permutation_reordered(&before_full, &after_full)
                    || unique_permutation_reordered(&before_shallow, &after_shallow)
                {
                    return Err(format!("block reorder cannot be poked at {path}"));
                }
                for index in 0..before_len {
                    validate_poke_structure(
                        before.element(index).unwrap(),
                        after.element(index).unwrap(),
                        &format!("{path}[{index}]"),
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn append_patch(
    memory: &dyn RuntimeMemory,
    patches: &mut Vec<PokePatch>,
    prior: Option<&LastPoke>,
    address: u64,
    expected: &[u8],
    edited: &[u8],
    path: &str,
) -> Result<(), String> {
    let prior_patch = prior.and_then(|last| {
        last.plan.patches.iter().find(|patch| {
            patch.address == address
                && patch.edited.len() == edited.len()
                && patch.field_path == path
        })
    });
    if expected == edited && prior_patch.is_none() {
        return Ok(());
    }
    let live = memory.read(address, expected.len())?;
    let effective_expected = if live == expected || live == edited {
        expected
    } else if let Some(prior_patch) = prior_patch.filter(|patch| live == patch.edited) {
        prior_patch.edited.as_slice()
    } else {
        return Err(format!(
            "live value conflicts with the shipped tag at {path}"
        ));
    };
    if live == edited && expected == edited {
        return Ok(());
    }
    if !memory.is_writable(address, expected.len())? {
        return Err(format!("destination is not writable at {path}"));
    }
    patches.push(PokePatch {
        address,
        expected: effective_expected.to_vec(),
        edited: edited.to_vec(),
        field_path: path.to_owned(),
    });
    Ok(())
}

fn append_string_id_patch(
    memory: &dyn RuntimeMemory,
    patches: &mut Vec<PokePatch>,
    prior: Option<&LastPoke>,
    string_ids: &RuntimeStringIdIndex,
    address: u64,
    before: &str,
    after: &str,
    path: &str,
) -> Result<(), String> {
    let expected = string_ids
        .resolve(before)
        .map_err(|error| format!("shipped {error} at {path}"))?;
    let edited = string_ids
        .resolve(after)
        .map_err(|error| format!("{error} at {path}"))?;
    append_patch(
        memory,
        patches,
        prior,
        address,
        &expected.to_le_bytes(),
        &edited.to_le_bytes(),
        path,
    )
}

fn rollback<M: RuntimeWriteMemory + ?Sized>(
    memory: &M,
    completed: &[(u64, Vec<u8>)],
) -> Result<(), String> {
    let mut failures = Vec::new();
    for (address, bytes) in completed.iter().rev() {
        if let Err(error) = memory.write(*address, bytes) {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn apply_transaction<M: RuntimeWriteMemory + ?Sized>(
    memory: &M,
    patches: &[PokePatch],
) -> Result<(Vec<Vec<u8>>, PokeReport), String> {
    let mut originals = Vec::with_capacity(patches.len());
    for patch in patches {
        let live = memory.read(patch.address, patch.expected.len())?;
        if live != patch.expected && live != patch.edited {
            return Err(format!("live value conflict at {}", patch.field_path));
        }
        if !memory.is_writable(patch.address, patch.edited.len())? {
            return Err(format!(
                "destination is not writable at {}",
                patch.field_path
            ));
        }
        originals.push(live);
    }

    let mut completed: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut written_fields = 0usize;
    let mut written_bytes = 0usize;
    let mut skipped_fields = 0usize;
    for (index, patch) in patches.iter().enumerate() {
        let current = match memory.read(patch.address, patch.expected.len()) {
            Ok(bytes) => bytes,
            Err(error) => {
                return match rollback(memory, &completed) {
                    Ok(()) => Err(format!(
                        "poke pre-write check failed at {}: {error}; completed writes were rolled back",
                        patch.field_path
                    )),
                    Err(rollback_error) => Err(format!(
                        "poke pre-write check failed at {}: {error}; rollback also failed: {rollback_error}",
                        patch.field_path
                    )),
                };
            }
        };
        if current != patch.expected && current != patch.edited {
            return match rollback(memory, &completed) {
                Ok(()) => Err(format!(
                    "live value changed during poke at {}; completed writes were rolled back",
                    patch.field_path
                )),
                Err(rollback_error) => Err(format!(
                    "live value changed during poke at {}; rollback also failed: {rollback_error}",
                    patch.field_path
                )),
            };
        }
        originals[index] = current;
        if originals[index] == patch.edited {
            skipped_fields += 1;
            continue;
        }
        let attempt = memory.write(patch.address, &patch.edited).and_then(|()| {
            let verified = memory.read(patch.address, patch.edited.len())?;
            (verified == patch.edited)
                .then_some(())
                .ok_or_else(|| "write verification failed".to_owned())
        });
        if let Err(error) = attempt {
            let current_restore = memory.write(patch.address, &originals[index]);
            return match rollback(memory, &completed) {
                Ok(()) if current_restore.is_ok() => Err(format!(
                    "poke failed at {}: {error}; all attempted writes were rolled back",
                    patch.field_path
                )),
                Err(rollback_error) => Err(format!(
                    "poke failed at {}: {error}; rollback also failed: {rollback_error}",
                    patch.field_path
                )),
                Ok(()) => Err(format!(
                    "poke failed at {}: {error}; the failing write could not be restored: {}",
                    patch.field_path,
                    current_restore.unwrap_err()
                )),
            };
        }
        completed.push((patch.address, originals[index].clone()));
        written_fields += 1;
        written_bytes += patch.edited.len();
    }
    Ok((
        originals,
        PokeReport {
            written_fields,
            written_bytes,
            skipped_fields,
            message: "Poke complete".to_owned(),
        },
    ))
}

fn undo_transaction<M: RuntimeWriteMemory + ?Sized>(
    memory: &M,
    patches: &[PokePatch],
    originals: &[Vec<u8>],
) -> Result<PokeReport, String> {
    if patches.len() != originals.len() {
        return Err("undo record is inconsistent".to_owned());
    }
    for patch in patches {
        let live = memory.read(patch.address, patch.edited.len())?;
        if live != patch.edited {
            return Err(format!(
                "cannot undo because live memory changed at {}",
                patch.field_path
            ));
        }
    }
    let mut completed: Vec<(u64, Vec<u8>)> = Vec::new();
    for (index, patch) in patches.iter().enumerate() {
        let original = &originals[index];
        if original == &patch.edited {
            continue;
        }
        let current = match memory.read(patch.address, patch.edited.len()) {
            Ok(bytes) => bytes,
            Err(error) => {
                return match rollback(memory, &completed) {
                    Ok(()) => Err(format!(
                        "undo pre-write check failed at {}: {error}; restored bytes were re-applied",
                        patch.field_path
                    )),
                    Err(rollback_error) => Err(format!(
                        "undo pre-write check failed at {}: {error}; undo rollback also failed: {rollback_error}",
                        patch.field_path
                    )),
                };
            }
        };
        if current != patch.edited {
            return match rollback(memory, &completed) {
                Ok(()) => Err(format!(
                    "live memory changed during undo at {}; restored bytes were re-applied",
                    patch.field_path
                )),
                Err(rollback_error) => Err(format!(
                    "live memory changed during undo at {}; undo rollback also failed: {rollback_error}",
                    patch.field_path
                )),
            };
        }
        let attempt = memory.write(patch.address, original).and_then(|()| {
            let verified = memory.read(patch.address, original.len())?;
            (verified == *original)
                .then_some(())
                .ok_or_else(|| "undo verification failed".to_owned())
        });
        if let Err(error) = attempt {
            let current_restore = memory.write(patch.address, &patch.edited);
            return match rollback(memory, &completed) {
                Ok(()) if current_restore.is_ok() => Err(format!(
                    "undo failed at {}: {error}; already-restored bytes were re-applied",
                    patch.field_path
                )),
                Err(rollback_error) => Err(format!(
                    "undo failed at {}: {error}; undo rollback also failed: {rollback_error}",
                    patch.field_path
                )),
                Ok(()) => Err(format!(
                    "undo failed at {}: {error}; the failing restore could not be re-applied: {}",
                    patch.field_path,
                    current_restore.unwrap_err()
                )),
            };
        }
        completed.push((patch.address, patch.edited.clone()));
    }
    Ok(PokeReport {
        written_fields: completed.len(),
        written_bytes: completed.iter().map(|(_, bytes)| bytes.len()).sum(),
        skipped_fields: patches.len() - completed.len(),
        message: "Undo complete".to_owned(),
    })
}

struct Planner<'a> {
    memory: &'a dyn RuntimeMemory,
    index: &'a RuntimeTagIndex,
    string_ids: &'a RuntimeStringIdIndex,
    prior: Option<&'a LastPoke>,
    patches: Vec<PokePatch>,
}

impl Planner<'_> {
    fn plan_struct(
        &mut self,
        baseline: TagStruct<'_>,
        edited: TagStruct<'_>,
        live_address: u64,
        parent_path: &str,
    ) -> Result<(), String> {
        if baseline.size() != edited.size() {
            return Err(format!("structural edit cannot be poked at {parent_path}"));
        }
        let baseline_fields: Vec<_> = baseline.fields_all().collect();
        let edited_fields: Vec<_> = edited.fields_all().collect();
        if baseline_fields.len() != edited_fields.len() {
            return Err(format!("schema mismatch at {parent_path}"));
        }
        for (before_field, after_field) in baseline_fields.into_iter().zip(edited_fields) {
            if before_field.field_type() != after_field.field_type()
                || before_field.definition().offset() != after_field.definition().offset()
            {
                return Err(format!("schema mismatch at {parent_path}"));
            }
            let path = field_path(parent_path, before_field);
            let (offset, span) = field_span(baseline, before_field)?;
            let field_address = checked_add(live_address, offset as u64)?;
            match before_field.field_type() {
                TagFieldType::Struct => {
                    let before = before_field
                        .as_struct()
                        .ok_or_else(|| format!("missing shipped struct at {path}"))?;
                    let after = after_field
                        .as_struct()
                        .ok_or_else(|| format!("missing edited struct at {path}"))?;
                    self.plan_struct(before, after, field_address, &path)?;
                }
                TagFieldType::Array => {
                    self.plan_array(before_field, after_field, field_address, &path)?;
                }
                TagFieldType::Block => {
                    self.plan_block(before_field, after_field, field_address, &path)?;
                }
                TagFieldType::Data => {
                    self.plan_data(before_field, after_field, field_address, span, &path)?;
                }
                TagFieldType::TagReference => {
                    self.plan_reference(before_field, after_field, field_address, span, &path)?;
                }
                TagFieldType::PageableResource => {
                    let before = before_field.as_resource();
                    let after = after_field.as_resource();
                    let unchanged = match (before, after) {
                        (Some(before), Some(after)) => resources_equal(before, after),
                        (None, None) => true,
                        _ => false,
                    };
                    if !unchanged {
                        return Err(format!("pageable-resource edit cannot be poked at {path}"));
                    }
                }
                TagFieldType::StringId | TagFieldType::OldStringId => {
                    let before = string_id_text(before_field)?;
                    let after = string_id_text(after_field)?;
                    if before != after {
                        append_string_id_patch(
                            self.memory,
                            &mut self.patches,
                            self.prior,
                            self.string_ids,
                            field_address,
                            &before,
                            &after,
                            &path,
                        )?;
                    } else if let Some((_, original)) = self.prior.and_then(|last| {
                        last.plan
                            .patches
                            .iter()
                            .enumerate()
                            .find(|(_, patch)| {
                                patch.address == field_address
                                    && patch.field_path == path
                                    && patch.edited.len() == 4
                            })
                            .and_then(|(index, patch)| {
                                last.originals.get(index).map(|original| (patch, original))
                            })
                    }) {
                        append_patch(
                            self.memory,
                            &mut self.patches,
                            self.prior,
                            field_address,
                            original,
                            original,
                            &path,
                        )?;
                    }
                }
                TagFieldType::ApiInterop
                | TagFieldType::VertexBuffer
                | TagFieldType::Pointer
                | TagFieldType::NonCacheRuntimeValue
                | TagFieldType::Custom
                | TagFieldType::Unknown => {
                    let before = raw_span(baseline, offset, span)?;
                    let after = raw_span(edited, offset, span)?;
                    if before != after
                        || field_debug_value(before_field) != field_debug_value(after_field)
                    {
                        return Err(format!("unverified field representation at {path}"));
                    }
                }
                TagFieldType::Pad
                | TagFieldType::UselessPad
                | TagFieldType::Skip
                | TagFieldType::Explanation
                | TagFieldType::Terminator => {}
                _ => {
                    let before = raw_span(baseline, offset, span)?;
                    let after = raw_span(edited, offset, span)?;
                    if before == after
                        && field_debug_value(before_field) != field_debug_value(after_field)
                    {
                        return Err(format!("unverified field representation at {path}"));
                    }
                    append_patch(
                        self.memory,
                        &mut self.patches,
                        self.prior,
                        field_address,
                        before,
                        after,
                        &path,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn plan_array(
        &mut self,
        before_field: TagField<'_>,
        after_field: TagField<'_>,
        live_address: u64,
        path: &str,
    ) -> Result<(), String> {
        let before = before_field
            .as_array()
            .ok_or_else(|| format!("missing shipped fixed array at {path}"))?;
        let after = after_field
            .as_array()
            .ok_or_else(|| format!("missing edited fixed array at {path}"))?;
        if before.len() != after.len() {
            return Err(format!("structural edit cannot be poked at {path}"));
        }
        let element_size = before.definition().struct_definition().size();
        if element_size != after.definition().struct_definition().size() {
            return Err(format!("schema mismatch at {path}"));
        }
        for index in 0..before.len() {
            let before_element = before
                .element(index)
                .ok_or_else(|| format!("missing shipped array element at {path}[{index}]"))?;
            let after_element = after
                .element(index)
                .ok_or_else(|| format!("missing edited array element at {path}[{index}]"))?;
            let element_address = checked_index(live_address, index, element_size)?;
            self.plan_struct(
                before_element,
                after_element,
                element_address,
                &format!("{path}[{index}]"),
            )?;
        }
        Ok(())
    }

    fn plan_block(
        &mut self,
        before_field: TagField<'_>,
        after_field: TagField<'_>,
        descriptor_address: u64,
        path: &str,
    ) -> Result<(), String> {
        let before = before_field.as_block();
        let after = after_field.as_block();
        let before_len = before.map_or(0, |block| block.len());
        let after_len = after.map_or(0, |block| block.len());
        if before_len != after_len {
            return Err(format!("structural edit cannot be poked at {path}"));
        }
        let descriptor = self.memory.read(descriptor_address, 12)?;
        let live_count = u32::from_le_bytes(descriptor[0..4].try_into().unwrap()) as usize;
        if live_count != before_len {
            return Err(format!("stale live block count at {path}"));
        }
        if before_len == 0 {
            return Ok(());
        }
        let encoded_data = u32::from_le_bytes(descriptor[4..8].try_into().unwrap());
        let encoded_definition = u32::from_le_bytes(descriptor[8..12].try_into().unwrap());
        let data_address = self.memory.resolve_offset(encoded_data)?;
        let definition_address = self.memory.resolve_offset(encoded_definition)?;
        self.memory
            .read(definition_address, 1)
            .map_err(|_| format!("live block definition pointer is unreadable at {path}"))?;
        let before = before.unwrap();
        let after = after.unwrap();
        if before.element_size() != after.element_size() {
            return Err(format!("structural edit cannot be poked at {path}"));
        }
        for index in 0..before_len {
            let element_address = checked_index(data_address, index, before.element_size())?;
            self.plan_struct(
                before.element(index).unwrap(),
                after.element(index).unwrap(),
                element_address,
                &format!("{path}[{index}]"),
            )?;
        }
        Ok(())
    }

    fn plan_data(
        &mut self,
        before_field: TagField<'_>,
        after_field: TagField<'_>,
        descriptor_address: u64,
        span: usize,
        path: &str,
    ) -> Result<(), String> {
        let before = before_field
            .as_data()
            .ok_or_else(|| format!("missing shipped data at {path}"))?;
        let after = after_field
            .as_data()
            .ok_or_else(|| format!("missing edited data at {path}"))?;
        if before.len() != after.len() {
            return Err(format!("resized data cannot be poked at {path}"));
        }
        if before.is_empty() {
            return if before == after {
                Ok(())
            } else {
                Err(format!("empty data cannot be replaced at {path}"))
            };
        }
        let descriptor = self.memory.read(descriptor_address, span)?;
        let length =
            u32::try_from(before.len()).map_err(|_| format!("data is too large at {path}"))?;
        if !descriptor
            .chunks_exact(4)
            .any(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()) == length)
        {
            return Err(format!("live data length was not found at {path}"));
        }
        let mut descriptor_addresses = Vec::new();
        let mut candidates = Vec::new();
        for chunk in descriptor.chunks_exact(4) {
            let encoded = u32::from_le_bytes(chunk.try_into().unwrap());
            let Ok(address) = self.memory.resolve_offset(encoded) else {
                continue;
            };
            descriptor_addresses.push(address);
            let Ok(live) = self.memory.read(address, before.len()) else {
                continue;
            };
            if live == before || live == after {
                candidates.push(address);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        let trusted_prior = self.prior.and_then(|last| {
            let mut matches = last.plan.patches.iter().filter(|patch| {
                patch.field_path == path
                    && patch.edited.len() == before.len()
                    && descriptor_addresses.contains(&patch.address)
                    && self
                        .memory
                        .read(patch.address, patch.edited.len())
                        .is_ok_and(|live| live == patch.edited)
            });
            let first = matches.next();
            first.filter(|_| matches.next().is_none())
        });
        if let Some(prior_patch) = trusted_prior {
            candidates.clear();
            candidates.push(prior_patch.address);
        } else if before == after {
            return Ok(());
        }
        if candidates.len() != 1 {
            return Err(format!(
                "live data descriptor layout is not uniquely verified at {path}"
            ));
        }
        append_patch(
            self.memory,
            &mut self.patches,
            self.prior,
            candidates[0],
            before,
            after,
            path,
        )
    }

    fn reference_handle(
        &self,
        reference: Option<(u32, String)>,
        path: &str,
    ) -> Result<u32, String> {
        match reference {
            None => Ok(u32::MAX),
            Some((group, name)) => {
                let tag = self
                    .index
                    .find(&name, group)
                    .map_err(|_| format!("referenced tag is not loaded at {path}: {name}"))?;
                validate_runtime_tag_entry(self.memory, tag)?;
                Ok(tag.handle)
            }
        }
    }

    fn plan_reference(
        &mut self,
        before_field: TagField<'_>,
        after_field: TagField<'_>,
        field_address: u64,
        span: usize,
        path: &str,
    ) -> Result<(), String> {
        let before = match before_field.value() {
            Some(TagFieldData::TagReference(reference)) => reference.group_tag_and_name,
            _ => return Err(format!("missing shipped tag reference at {path}")),
        };
        let after = match after_field.value() {
            Some(TagFieldData::TagReference(reference)) => reference.group_tag_and_name,
            _ => return Err(format!("missing edited tag reference at {path}")),
        };
        let unchanged = before == after;
        let reference_group = after.as_ref().or(before.as_ref()).map(|(group, _)| *group);
        let field_end = checked_add(field_address, span as u64)?;
        let has_prior_patch = self.prior.is_some_and(|last| {
            last.plan.patches.iter().any(|patch| {
                patch.field_path == path
                    && patch.edited.len() == 4
                    && patch.address >= field_address
                    && checked_add(patch.address, 4).is_ok_and(|end| end <= field_end)
            })
        });
        if unchanged && !has_prior_patch {
            return Ok(());
        }
        let expected = self.reference_handle(before, path)?;
        let edited = self.reference_handle(after, path)?;
        let live = self.memory.read(field_address, span)?;
        let positions: Vec<usize> = live
            .chunks_exact(4)
            .enumerate()
            .filter_map(|(index, chunk)| {
                (u32::from_le_bytes(chunk.try_into().unwrap()) == expected).then_some(index * 4)
            })
            .collect();
        let edited_positions: Vec<usize> = live
            .chunks_exact(4)
            .enumerate()
            .filter_map(|(index, chunk)| {
                (u32::from_le_bytes(chunk.try_into().unwrap()) == edited).then_some(index * 4)
            })
            .collect();
        let loaded_positions = reference_group
            .map(|group| loaded_reference_positions(&live, self.index, group))
            .unwrap_or_default();
        let trusted_prior = self.prior.and_then(|last| {
            let mut matches = last.plan.patches.iter().filter(|patch| {
                patch.field_path == path
                    && patch.edited.len() == 4
                    && patch.address >= field_address
                    && checked_add(patch.address, 4).is_ok_and(|end| end <= field_end)
                    && self
                        .memory
                        .read(patch.address, 4)
                        .is_ok_and(|current| current == patch.edited)
            });
            let first = matches.next();
            first.filter(|_| matches.next().is_none())
        });
        let (address, recovered_live_reference) = if let Some(prior_patch) = trusted_prior {
            (prior_patch.address, false)
        } else if unchanged {
            return Ok(());
        } else if positions.len() == 1 {
            (checked_add(field_address, positions[0] as u64)?, false)
        } else if edited_positions.len() == 1 {
            (
                checked_add(field_address, edited_positions[0] as u64)?,
                false,
            )
        } else if !has_prior_patch && loaded_positions.len() == 1 {
            (
                checked_add(field_address, loaded_positions[0] as u64)?,
                true,
            )
        } else {
            return Err(format!(
                "live tag-reference representation is not uniquely verified at {path}"
            ));
        };
        if recovered_live_reference {
            let current = self.memory.read(address, 4)?;
            if !self.memory.is_writable(address, 4)? {
                return Err(format!("destination is not writable at {path}"));
            }
            self.patches.push(PokePatch {
                address,
                expected: current,
                edited: edited.to_le_bytes().to_vec(),
                field_path: path.to_owned(),
            });
            return Ok(());
        }
        append_patch(
            self.memory,
            &mut self.patches,
            self.prior,
            address,
            &expected.to_le_bytes(),
            &edited.to_le_bytes(),
            path,
        )
    }
}

fn root_data_address(
    memory: &dyn RuntimeMemory,
    tag: &RuntimeTag,
    expected_size: usize,
) -> Result<u64, String> {
    let descriptor = memory.read(tag.root_descriptor, 12)?;
    let count = u32::from_le_bytes(descriptor[0..4].try_into().unwrap());
    if count != 1 {
        return Err("load a mission first (root tag block is unavailable)".to_owned());
    }
    let data = memory.resolve_offset(u32::from_le_bytes(descriptor[4..8].try_into().unwrap()))?;
    let definition =
        memory.resolve_offset(u32::from_le_bytes(descriptor[8..12].try_into().unwrap()))?;
    memory
        .read(definition, 1)
        .map_err(|_| "root block definition is unreadable".to_owned())?;
    memory
        .read(data, expected_size)
        .map_err(|_| "root tag data is unreadable".to_owned())?;
    Ok(data)
}

fn compile_plan(
    memory: &dyn RuntimeMemory,
    index: RuntimeTagIndex,
    string_ids: &RuntimeStringIdIndex,
    baseline: &TagFile,
    edited: &TagFile,
    tag_path: &str,
    group_tag: u32,
    prior: Option<&LastPoke>,
) -> Result<PokePlan, String> {
    let live_tag = index.find(tag_path, group_tag)?.clone();
    validate_runtime_tag_entry(memory, &live_tag)?;
    let root_address = root_data_address(memory, &live_tag, baseline.root().size())?;
    let normalized_path = normalize_tag_path(tag_path);
    let prior = prior.filter(|last| {
        last.originals.len() == last.plan.patches.len()
            && last.plan.identity == index.identity
            && last.plan.tag_path == normalized_path
            && last.plan.group_tag == group_tag
            && last.plan.tag_handle == live_tag.handle
            && last.plan.tag_entry_address == live_tag.entry_address
            && last.plan.tag_name_pointer == live_tag.name_pointer
            && last.plan.root_address == root_address
    });
    let mut planner = Planner {
        memory,
        index: &index,
        string_ids,
        prior,
        patches: Vec::new(),
    };
    planner.plan_struct(baseline.root(), edited.root(), root_address, "")?;
    planner
        .patches
        .sort_by_key(|patch| (patch.address, patch.edited.len()));
    for pair in planner.patches.windows(2) {
        let first_end = checked_add(pair[0].address, pair[0].edited.len() as u64)?;
        if first_end > pair[1].address {
            return Err(format!(
                "overlapping runtime patches at {} and {}",
                pair[0].field_path, pair[1].field_path
            ));
        }
    }
    let chain_originals = planner
        .patches
        .iter()
        .map(|patch| {
            prior.and_then(|last| {
                last.plan
                    .patches
                    .iter()
                    .position(|prior_patch| {
                        prior_patch.address == patch.address
                            && prior_patch.edited.len() == patch.edited.len()
                            && prior_patch.field_path == patch.field_path
                    })
                    .and_then(|index| last.originals.get(index).cloned())
            })
        })
        .collect();
    Ok(PokePlan {
        profile: CU2_PROFILE,
        identity: index.identity.clone(),
        tag_path: normalized_path,
        group_tag,
        tag_handle: live_tag.handle,
        tag_entry_address: live_tag.entry_address,
        tag_name_pointer: live_tag.name_pointer,
        root_address,
        patches: planner.patches,
        chain_originals,
    })
}

#[cfg(windows)]
mod platform {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::ffi::c_void;
    use std::fs::File;
    use std::io::Read;
    use std::mem::size_of;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W,
        Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY,
        PAGE_GUARD, PAGE_NOACCESS, PAGE_READWRITE, PAGE_WRITECOPY, VirtualQueryEx,
    };
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_OPERATION,
        PROCESS_VM_READ, PROCESS_VM_WRITE,
    };

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    struct ProcessMemory {
        handle: OwnedHandle,
        identity: RuntimeIdentity,
        segment_table: u64,
    }

    static RUNTIME_INDEX_CACHE: OnceLock<Mutex<Option<RuntimeTagIndex>>> = OnceLock::new();
    static VERIFIED_PROCESS_CACHE: OnceLock<Mutex<Option<(u32, u64, u64)>>> = OnceLock::new();

    fn index_cache() -> &'static Mutex<Option<RuntimeTagIndex>> {
        RUNTIME_INDEX_CACHE.get_or_init(|| Mutex::new(None))
    }

    fn cache_guard() -> std::sync::MutexGuard<'static, Option<RuntimeTagIndex>> {
        index_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn verified_process_cache() -> &'static Mutex<Option<(u32, u64, u64)>> {
        VERIFIED_PROCESS_CACHE.get_or_init(|| Mutex::new(None))
    }

    fn verified_process_matches(process_id: u32, creation_time: u64, module_base: u64) -> bool {
        *verified_process_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            == Some((process_id, creation_time, module_base))
    }

    fn store_verified_process(process_id: u32, creation_time: u64, module_base: u64) {
        *verified_process_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((process_id, creation_time, module_base));
    }

    fn cached_index(identity: &RuntimeIdentity) -> Option<RuntimeTagIndex> {
        cache_guard()
            .as_ref()
            .filter(|index| index.identity == *identity)
            .cloned()
    }

    fn cached_identity_matches(identity: &RuntimeIdentity) -> bool {
        cache_guard()
            .as_ref()
            .is_some_and(|index| index.identity == *identity)
    }

    fn store_cached_index(index: RuntimeTagIndex) {
        *cache_guard() = Some(index);
    }

    fn clear_cached_index() {
        *cache_guard() = None;
    }

    pub(super) fn clear_runtime_cache() {
        clear_cached_index();
    }

    impl RuntimeMemory for ProcessMemory {
        fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
            if length == 0 {
                return Ok(Vec::new());
            }
            self.query_region(address, length, false)?;
            let mut bytes = vec![0u8; length];
            let mut read = 0usize;
            unsafe {
                ReadProcessMemory(
                    self.handle.0,
                    address as *const c_void,
                    bytes.as_mut_ptr().cast(),
                    length,
                    Some(&mut read),
                )
                .map_err(|error| format!("could not read game memory: {error}"))?;
            }
            if read != length {
                return Err("short read from game memory".to_owned());
            }
            Ok(bytes)
        }

        fn is_writable(&self, address: u64, length: usize) -> Result<bool, String> {
            self.query_region(address, length, true).map(|_| true)
        }

        fn resolve_offset(&self, encoded: u32) -> Result<u64, String> {
            let (segment, byte_offset) = encoded_offset_parts(encoded)?;
            let segment_entry = checked_index(self.segment_table, segment, 8)?;
            let base = read_u64(self, segment_entry)?;
            if base == 0 {
                return Err("runtime segment is not loaded".to_owned());
            }
            checked_add(base, byte_offset)
        }
    }

    impl ProcessMemory {
        fn query_region(&self, address: u64, length: usize, write: bool) -> Result<(), String> {
            let end = checked_add(address, length as u64)?;
            let mut cursor = address;
            while cursor < end {
                let mut info = MEMORY_BASIC_INFORMATION::default();
                let returned = unsafe {
                    VirtualQueryEx(
                        self.handle.0,
                        Some(cursor as *const c_void),
                        &mut info,
                        size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                };
                if returned == 0 || info.State != MEM_COMMIT {
                    return Err("runtime pointer references uncommitted memory".to_owned());
                }
                let protection = info.Protect;
                if protection.contains(PAGE_GUARD) || protection.contains(PAGE_NOACCESS) {
                    return Err("runtime pointer references inaccessible memory".to_owned());
                }
                if write
                    && ![
                        PAGE_READWRITE,
                        PAGE_WRITECOPY,
                        PAGE_EXECUTE_READWRITE,
                        PAGE_EXECUTE_WRITECOPY,
                    ]
                    .iter()
                    .any(|allowed| protection.contains(*allowed))
                {
                    return Err("runtime destination is not writable".to_owned());
                }
                let base = info.BaseAddress as usize as u64;
                let region_end = checked_add(base, info.RegionSize as u64)?;
                if region_end <= cursor {
                    return Err("invalid runtime memory region".to_owned());
                }
                cursor = region_end.min(end);
            }
            Ok(())
        }
    }

    impl RuntimeWriteMemory for ProcessMemory {
        fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String> {
            self.query_region(address, bytes.len(), true)?;
            let mut written = 0usize;
            unsafe {
                WriteProcessMemory(
                    self.handle.0,
                    address as *const c_void,
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    Some(&mut written),
                )
                .map_err(|error| format!("could not write game memory: {error}"))?;
            }
            if written != bytes.len() {
                return Err("short write to game memory".to_owned());
            }
            Ok(())
        }
    }

    #[derive(Clone)]
    struct ModuleInfo {
        name: String,
        path: PathBuf,
        base: u64,
        size: usize,
    }

    fn wide_string(value: &[u16]) -> String {
        let end = value
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..end])
    }

    fn process_ids() -> Result<Vec<u32>, String> {
        let snapshot = OwnedHandle(unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
                .map_err(|error| format!("could not enumerate processes: {error}"))?
        });
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ids = Vec::new();
        if unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_ok() {
            loop {
                if wide_string(&entry.szExeFile).eq_ignore_ascii_case(PROCESS_NAME) {
                    ids.push(entry.th32ProcessID);
                }
                if unsafe { Process32NextW(snapshot.0, &mut entry) }.is_err() {
                    break;
                }
            }
        }
        Ok(ids)
    }

    fn modules(process_id: u32) -> Result<Vec<ModuleInfo>, String> {
        let snapshot = OwnedHandle(unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, process_id)
                .map_err(|error| format!("could not enumerate game modules: {error}"))?
        });
        let mut entry = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        let mut modules = Vec::new();
        if unsafe { Module32FirstW(snapshot.0, &mut entry) }.is_ok() {
            loop {
                modules.push(ModuleInfo {
                    name: wide_string(&entry.szModule),
                    path: PathBuf::from(wide_string(&entry.szExePath)),
                    base: entry.modBaseAddr as usize as u64,
                    size: entry.modBaseSize as usize,
                });
                if unsafe { Module32NextW(snapshot.0, &mut entry) }.is_err() {
                    break;
                }
            }
        }
        Ok(modules)
    }

    fn sha256(path: &Path) -> Result<String, String> {
        let mut file = File::open(path)
            .map_err(|error| format!("could not open {}: {error}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 1024 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| format!("could not hash {}: {error}", path.display()))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok(format!("{:X}", hasher.finalize()))
    }

    fn creation_time(handle: HANDLE) -> Result<u64, String> {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user)
                .map_err(|error| format!("could not identify game process: {error}"))?;
        }
        Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
    }

    fn attach(write: bool) -> Result<(ProcessMemory, ModuleInfo), String> {
        let ids = process_ids()?;
        let process_id = match ids.as_slice() {
            [] => return Err("game not running".to_owned()),
            [only] => *only,
            _ => return Err("multiple matching game processes are running".to_owned()),
        };
        let modules = modules(process_id)?;
        let host = modules
            .iter()
            .find(|module| module.name.eq_ignore_ascii_case(PROCESS_NAME))
            .ok_or_else(|| "game executable module is missing".to_owned())?;
        let dll = modules
            .iter()
            .find(|module| module.name.eq_ignore_ascii_case(TAG_DLL_NAME))
            .cloned()
            .ok_or_else(|| "load a mission first (tag module is not loaded)".to_owned())?;
        let mut rights = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ;
        if write {
            rights |= PROCESS_VM_WRITE | PROCESS_VM_OPERATION;
        }
        let handle = OwnedHandle(unsafe {
            OpenProcess(rights, false, process_id)
                .map_err(|error| format!("could not open game process: {error}"))?
        });
        let creation_time = creation_time(handle.0)?;
        if !verified_process_matches(process_id, creation_time, dll.base) {
            clear_cached_index();
            if sha256(&host.path)? != CU2_PROFILE.host_sha256
                || sha256(&dll.path)? != CU2_PROFILE.dll_sha256
            {
                return Err("unsupported game build".to_owned());
            }
            store_verified_process(process_id, creation_time, dll.base);
        }
        let table_pointer = checked_add(dll.base, CU2_PROFILE.tag_table_pointer_rva)?;
        if [
            CU2_PROFILE.tag_table_pointer_rva,
            CU2_PROFILE.segment_table_rva,
            CU2_PROFILE.string_id_storage_rva,
            CU2_PROFILE.string_id_storage_used_rva,
            CU2_PROFILE.string_id_strings_rva,
            CU2_PROFILE.string_id_count_rva,
            CU2_PROFILE.string_id_mapping_table_rva,
            CU2_PROFILE.string_id_builtin_table_rva,
        ]
        .iter()
        .any(|rva| *rva >= dll.size as u64)
        {
            return Err("unsupported tag module layout".to_owned());
        }
        let provisional = ProcessMemory {
            handle,
            identity: RuntimeIdentity {
                process_id,
                creation_time,
                module_base: dll.base,
                tag_table: 0,
            },
            segment_table: checked_add(dll.base, CU2_PROFILE.segment_table_rva)?,
        };
        let tag_table = read_u64(&provisional, table_pointer)?;
        if tag_table == 0 {
            return Err("load a mission first (tag table is empty)".to_owned());
        }
        let mut memory = provisional;
        memory.identity.tag_table = tag_table;
        if !cached_identity_matches(&memory.identity) {
            clear_cached_index();
        }
        Ok((memory, dll))
    }

    fn read_c_string(memory: &ProcessMemory, address: u64) -> Result<String, String> {
        if address == 0 {
            return Err("runtime tag has a null name".to_owned());
        }
        let mut bytes = Vec::new();
        let mut cursor = address;
        while bytes.len() < MAX_RUNTIME_NAME {
            let chunk = memory.read(cursor, 64)?;
            if let Some(end) = chunk.iter().position(|&byte| byte == 0) {
                bytes.extend_from_slice(&chunk[..end]);
                return String::from_utf8(bytes)
                    .map_err(|_| "runtime tag name is not UTF-8".to_owned());
            }
            bytes.extend_from_slice(&chunk);
            cursor = checked_add(cursor, chunk.len() as u64)?;
        }
        Err("runtime tag name is not terminated".to_owned())
    }

    fn discover_index(memory: &ProcessMemory) -> Result<RuntimeTagIndex, String> {
        let object = memory.identity.tag_table;
        let element_size = read_u32(memory, checked_add(object, 0x20)?)? as usize;
        let maximum = read_u32(memory, checked_add(object, 0x2c)?)? as usize;
        let high_water = read_u32(memory, checked_add(object, 0x44)?)? as usize;
        let used = read_u32(memory, checked_add(object, 0x48)?)? as usize;
        let entries = read_u64(memory, checked_add(object, 0x50)?)?;
        let bitset = read_u64(memory, checked_add(object, 0x58)?)?;
        if element_size != TAG_ENTRY_SIZE
            || maximum == 0
            || maximum > MAX_RUNTIME_TAGS
            || high_water > maximum
            || used > high_water
            || entries == 0
            || bitset == 0
        {
            return Err("unsupported live tag-table layout".to_owned());
        }
        let bits = memory.read(bitset, high_water.div_ceil(8))?;
        let entries_length = high_water
            .checked_mul(TAG_ENTRY_SIZE)
            .ok_or_else(|| "runtime tag-table size overflow".to_owned())?;
        let entry_bytes = memory.read(entries, entries_length)?;
        struct PendingTag {
            index: usize,
            salt: u16,
            group_tag: u32,
            name_pointer: u64,
            entry_address: u64,
        }

        let mut pending = Vec::with_capacity(used);
        for index in 0..high_water {
            if bits[index / 8] & (1 << (index % 8)) == 0 {
                continue;
            }
            let entry = checked_index(entries, index, TAG_ENTRY_SIZE)?;
            let entry_offset = index * TAG_ENTRY_SIZE;
            let bytes = &entry_bytes[entry_offset..entry_offset + TAG_ENTRY_SIZE];
            let group_offset = TAG_ENTRY_GROUP_OFFSET as usize;
            let name_offset = TAG_ENTRY_NAME_OFFSET as usize;
            let salt = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
            let group_tag =
                u32::from_le_bytes(bytes[group_offset..group_offset + 4].try_into().unwrap());
            let name_pointer =
                u64::from_le_bytes(bytes[name_offset..name_offset + 8].try_into().unwrap());
            pending.push(PendingTag {
                index,
                salt,
                group_tag,
                name_pointer,
                entry_address: entry,
            });
        }
        let pointers: Vec<u64> = pending.iter().map(|tag| tag.name_pointer).collect();
        let name_pool = runtime_name_pool_bounds(&pointers)?.and_then(|(start, length)| {
            memory.read(start, length).ok().map(|bytes| (start, bytes))
        });
        let mut tags = Vec::with_capacity(used);
        for pending in pending {
            let path = match &name_pool {
                Some((start, bytes)) => {
                    runtime_name_from_pool(*start, bytes, pending.name_pointer)?
                }
                None => read_c_string(memory, pending.name_pointer)?,
            };
            let handle = (u32::from(pending.salt) << 16) | pending.index as u32;
            tags.push(RuntimeTag {
                path,
                group_tag: pending.group_tag,
                handle,
                entry_address: pending.entry_address,
                name_pointer: pending.name_pointer,
                root_descriptor: checked_add(pending.entry_address, TAG_ENTRY_ROOT_OFFSET)?,
            });
        }
        if tags.len() != used {
            return Err("live tag-table allocation map is inconsistent".to_owned());
        }
        Ok(RuntimeTagIndex {
            identity: memory.identity.clone(),
            tags,
        })
    }

    fn validate_baseline_sample(
        memory: &ProcessMemory,
        index: &RuntimeTagIndex,
        tag: &RuntimeTag,
        baseline: &TagFile,
    ) -> Result<bool, String> {
        let root = baseline.root();
        let live_root = root_data_address(memory, tag, root.size())?;
        let mut compared = 0usize;
        probe_inline_scalars(memory, root, live_root, "", &mut compared)?;
        if index.find(&tag.path, tag.group_tag)?.entry_address != tag.entry_address {
            return Err("runtime tag index changed during discovery".to_owned());
        }
        Ok(compared > 0)
    }

    fn validate_discovery(
        memory: &ProcessMemory,
        index: &RuntimeTagIndex,
        source: &TagSource,
        entries: &[TagEntry],
        current_path: &str,
        current_group: u32,
        current_baseline: &TagFile,
    ) -> Result<(), String> {
        let current = index.find(current_path, current_group)?;
        let mut groups = HashSet::new();
        let mut validated = 0usize;
        if validate_baseline_sample(memory, index, current, current_baseline)? {
            groups.insert(current_group);
            validated += 1;
        }
        for entry in entries {
            if validated >= 3 || groups.contains(&entry.group_tag) {
                continue;
            }
            if entry.group_tag == current_group
                && normalize_tag_path(&entry.display_path) == normalize_tag_path(current_path)
            {
                continue;
            }
            let Ok(live) = index.find(&entry.display_path, entry.group_tag) else {
                continue;
            };
            let Ok(baseline) = read_entry(source, entry) else {
                continue;
            };
            if validate_baseline_sample(memory, index, live, &baseline)? {
                groups.insert(entry.group_tag);
                validated += 1;
            }
        }
        if validated < 3 {
            return Err(
                "load a mission first (not enough known tag groups passed the discovery probe)"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn build_runtime_string_id_index(
        memory: &ProcessMemory,
    ) -> Result<RuntimeStringIdIndex, String> {
        let address = |rva| checked_add(memory.identity.module_base, rva);
        let storage_address = read_u64(memory, address(CU2_PROFILE.string_id_storage_rva)?)?;
        let storage_used =
            read_u32(memory, address(CU2_PROFILE.string_id_storage_used_rva)?)? as usize;
        let strings_address = read_u64(memory, address(CU2_PROFILE.string_id_strings_rva)?)?;
        let count = read_u32(memory, address(CU2_PROFILE.string_id_count_rva)?)? as usize;
        let table_address = read_u64(memory, address(CU2_PROFILE.string_id_mapping_table_rva)?)?;
        if storage_address == 0 || strings_address == 0 || table_address == 0 || count == 0 {
            return Err("runtime string id registry is not initialized".to_owned());
        }
        if storage_used == 0 || storage_used > STRING_ID_STORAGE_CAPACITY {
            return Err("runtime string id name storage has an invalid size".to_owned());
        }
        if count < STRING_ID_BUILTIN_COUNT || count > STRING_ID_MAX_ENTRIES {
            return Err(
                "runtime string id registry count is outside the supported range".to_owned(),
            );
        }

        let header = memory.read(table_address, STRING_ID_TABLE_HEADER_SIZE)?;
        let bucket_count = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
        let max_entries = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let value_size = u64::from_le_bytes(header[8..16].try_into().unwrap()) as usize;
        if bucket_count != STRING_ID_BUCKET_COUNT
            || max_entries != STRING_ID_MAX_ENTRIES
            || value_size != STRING_ID_VALUE_SIZE
        {
            return Err("unsupported runtime string id mapping-table layout".to_owned());
        }
        let allocation_size = STRING_ID_TABLE_HEADER_SIZE
            .checked_add(
                bucket_count
                    .checked_mul(8)
                    .ok_or_else(|| "runtime string id table size overflow".to_owned())?,
            )
            .and_then(|size| {
                max_entries
                    .checked_mul(STRING_ID_NODE_SIZE)
                    .and_then(|nodes| size.checked_add(nodes))
            })
            .ok_or_else(|| "runtime string id table size overflow".to_owned())?;
        let table = memory.read(table_address, allocation_size)?;
        let storage = memory.read(storage_address, storage_used)?;
        let index = parse_runtime_string_id_table(table_address, &table, &storage, count)?;

        let strings_size = count
            .checked_mul(8)
            .ok_or_else(|| "runtime string id pointer table size overflow".to_owned())?;
        let strings = memory.read(strings_address, strings_size)?;
        let builtins_address = address(CU2_PROFILE.string_id_builtin_table_rva)?;
        let builtins = memory.read(
            builtins_address,
            STRING_ID_BUILTIN_COUNT
                .checked_mul(16)
                .ok_or_else(|| "runtime built-in string id table size overflow".to_owned())?,
        )?;
        for registration_index in 0..count {
            let pointer_offset = registration_index * 8;
            let name_pointer = u64::from_le_bytes(
                strings[pointer_offset..pointer_offset + 8]
                    .try_into()
                    .unwrap(),
            );
            let storage_offset =
                u32::try_from(name_pointer.checked_sub(storage_address).ok_or_else(|| {
                    "runtime string id pointer precedes the name storage".to_owned()
                })?)
                .map_err(|_| "runtime string id pointer is outside the name storage".to_owned())?;
            let name = string_id_storage_name(&storage, storage_offset)?;
            let expected = if registration_index < STRING_ID_BUILTIN_COUNT {
                let offset = registration_index * 16;
                u32::from_le_bytes(builtins[offset..offset + 4].try_into().unwrap())
            } else {
                STRING_ID_SET_ZERO_BUILTIN_COUNT
                    .checked_add(
                        u32::try_from(registration_index - STRING_ID_BUILTIN_COUNT)
                            .map_err(|_| "dynamic string id index overflow".to_owned())?,
                    )
                    .ok_or_else(|| "dynamic string id index overflow".to_owned())?
            };
            let normalized = normalize_string_id_bytes(name)?.unwrap_or_default();
            if index.by_name.get(&normalized) != Some(&expected) {
                let normalized = String::from_utf8_lossy(&normalized);
                return Err(format!(
                    "runtime string id registration cross-check failed at index {registration_index} ({normalized})"
                ));
            }
        }
        Ok(index)
    }

    pub(super) fn prepare(
        source: TagSource,
        entries: Vec<TagEntry>,
        entry: TagEntry,
        edited_bytes: Vec<u8>,
        prior: Option<LastPoke>,
    ) -> Result<PokePlan, String> {
        let baseline = read_entry(&source, &entry)
            .map_err(|error| format!("could not read shipped tag: {error:#}"))?;
        let edited = TagFile::read_from_bytes(&edited_bytes)
            .map_err(|error| format!("could not snapshot edited tag: {error}"))?;
        if baseline.header.group_tag != edited.header.group_tag
            || edited.header.group_tag != entry.group_tag
        {
            return Err("edited tag group does not match the shipped tag".to_owned());
        }
        validate_poke_structure(baseline.root(), edited.root(), "")?;
        let (memory, _) = attach(false)?;
        let string_ids = build_runtime_string_id_index(&memory)?;
        if let Some(index) = cached_index(&memory.identity) {
            match compile_plan(
                &memory,
                index,
                &string_ids,
                &baseline,
                &edited,
                &entry.display_path,
                entry.group_tag,
                prior.as_ref(),
            ) {
                Ok(plan) => return Ok(plan),
                Err(error) if should_refresh_runtime_index(&error) => {
                    clear_cached_index();
                }
                Err(error) => return Err(error),
            }
        }
        let index = discover_index(&memory)?;
        validate_discovery(
            &memory,
            &index,
            &source,
            &entries,
            &entry.display_path,
            entry.group_tag,
            &baseline,
        )?;
        store_cached_index(index.clone());
        compile_plan(
            &memory,
            index,
            &string_ids,
            &baseline,
            &edited,
            &entry.display_path,
            entry.group_tag,
            prior.as_ref(),
        )
    }

    fn validate_identity(memory: &ProcessMemory, plan: &PokePlan) -> Result<(), String> {
        if memory.identity != plan.identity {
            return Err("game process or tag table changed; run preflight again".to_owned());
        }
        let tag = RuntimeTag {
            path: plan.tag_path.clone(),
            group_tag: plan.group_tag,
            handle: plan.tag_handle,
            entry_address: plan.tag_entry_address,
            name_pointer: plan.tag_name_pointer,
            root_descriptor: checked_add(plan.tag_entry_address, TAG_ENTRY_ROOT_OFFSET)?,
        };
        if validate_runtime_tag_entry(memory, &tag).is_err()
            || root_data_address(memory, &tag, 1)? != plan.root_address
        {
            return Err("live tag allocation changed; run preflight again".to_owned());
        }
        Ok(())
    }

    pub(super) fn execute(plan: PokePlan) -> Result<(LastPoke, PokeReport), String> {
        let (memory, _) = attach(true)?;
        validate_identity(&memory, &plan)?;
        let (mut originals, report) = apply_transaction(&memory, &plan.patches)?;
        for (original, chain_original) in originals.iter_mut().zip(&plan.chain_originals) {
            if let Some(chain_original) = chain_original {
                *original = chain_original.clone();
            }
        }
        Ok((LastPoke { plan, originals }, report))
    }

    pub(super) fn undo(last: LastPoke) -> Result<PokeReport, String> {
        let (memory, _) = attach(true)?;
        validate_identity(&memory, &last.plan)?;
        undo_transaction(&memory, &last.plan.patches, &last.originals)
    }

    #[cfg(test)]
    pub(super) fn manual_read_only_discovery() -> Result<usize, String> {
        let (memory, _) = attach(false)?;
        discover_index(&memory).map(|index| index.tags.len())
    }

    #[cfg(test)]
    pub(super) fn manual_read_only_string_id_index_diagnostic() -> Result<String, String> {
        let (memory, _) = attach(false)?;
        let index = build_runtime_string_id_index(&memory)?;
        let warthog = index.resolve("warthog_d")?;
        let fork = index.resolve("fork_d")?;
        Ok(format!(
            "{} registered string ids; warthog_d=0x{warthog:08X}; fork_d=0x{fork:08X}; all {} built-ins cross-checked",
            index.by_name.len(),
            STRING_ID_BUILTIN_COUNT
        ))
    }

    #[cfg(test)]
    pub(super) fn manual_read_only_root_diagnostic(path: &str) -> Result<String, String> {
        let (memory, _) = attach(false)?;
        let index = match cached_index(&memory.identity) {
            Some(index) => index,
            None => {
                let index = discover_index(&memory)?;
                store_cached_index(index.clone());
                index
            }
        };
        let normalized = normalize_tag_path(path);
        let tag = index
            .tags
            .iter()
            .find(|tag| normalize_tag_path(&tag.path) == normalized)
            .ok_or_else(|| "tag is not loaded".to_owned())?;
        let descriptor = memory.read(tag.root_descriptor, 12)?;
        let count = u32::from_le_bytes(descriptor[0..4].try_into().unwrap());
        let data = u32::from_le_bytes(descriptor[4..8].try_into().unwrap());
        let definition = u32::from_le_bytes(descriptor[8..12].try_into().unwrap());
        let describe = |encoded: u32| -> Result<String, String> {
            let (segment, byte_offset) = encoded_offset_parts(encoded)?;
            let segment_entry = checked_index(memory.segment_table, segment, 8)?;
            let base = read_u64(&memory, segment_entry)?;
            let resolved = checked_add(base, byte_offset)?;
            let readable = memory
                .read(resolved, 1)
                .map(|bytes| format!("yes ({:02X})", bytes[0]))
                .unwrap_or_else(|error| format!("no ({error})"));
            Ok(format!(
                "encoded=0x{encoded:08X} segment={segment} base=0x{base:X} byte_offset=0x{byte_offset:X} resolved=0x{resolved:X} readable={readable}"
            ))
        };
        Ok(format!(
            "entry=0x{:X} descriptor=0x{:X} count={count}\ndata: {}\ndefinition: {}",
            tag.entry_address,
            tag.root_descriptor,
            describe(data)?,
            describe(definition)?
        ))
    }

    #[cfg(test)]
    pub(super) fn manual_read_only_cache_diagnostic() -> Result<String, String> {
        use std::time::Instant;

        clear_cached_index();
        let cold_started = Instant::now();
        let (memory, _) = attach(false)?;
        let index = discover_index(&memory)?;
        let count = index.tags.len();
        store_cached_index(index);
        let cold = cold_started.elapsed();

        clear_runtime_cache();
        let reopened_started = Instant::now();
        let (memory, _) = attach(false)?;
        let reopened_index = discover_index(&memory)?;
        if reopened_index.tags.len() != count {
            return Err("runtime index changed while testing source reopen".to_owned());
        }
        store_cached_index(reopened_index);
        let reopened = reopened_started.elapsed();

        let warm_started = Instant::now();
        let (memory, _) = attach(false)?;
        cached_index(&memory.identity)
            .ok_or_else(|| "runtime address cache was not reused".to_owned())?;
        let warm = warm_started.elapsed();
        Ok(format!(
            "{count} tags; cold={}ms reopened={}ms warm={}ms",
            cold.as_millis(),
            reopened.as_millis(),
            warm.as_millis()
        ))
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) fn clear_runtime_cache() {}

    pub(super) fn prepare(
        _source: TagSource,
        _entries: Vec<TagEntry>,
        _entry: TagEntry,
        _edited_bytes: Vec<u8>,
        _prior: Option<LastPoke>,
    ) -> Result<PokePlan, String> {
        Err("Runtime poking is only available on Windows".to_owned())
    }

    pub(super) fn execute(_plan: PokePlan) -> Result<(LastPoke, PokeReport), String> {
        Err("Runtime poking is only available on Windows".to_owned())
    }

    pub(super) fn undo(_last: LastPoke) -> Result<PokeReport, String> {
        Err("Runtime poking is only available on Windows".to_owned())
    }
}

impl Baboon {
    pub(super) fn reset_runtime_poke_source_state(&mut self) {
        platform::clear_runtime_cache();
        self.poke_dialog = None;
    }

    pub(super) fn can_poke_current_tag(&self) -> bool {
        cfg!(windows)
            && self
                .selected_entry()
                .is_some_and(|entry| matches!(entry.location, TagEntryLocation::Container { .. }))
            && self
                .kits
                .get(self.active)
                .and_then(|kit| kit.selected_key.as_ref())
                .is_some_and(|key| self.kits[self.active].parsed_tags.contains_key(key))
    }

    fn current_poke_request(&self) -> Result<PokeRequest, String> {
        if !cfg!(windows) {
            return Err("Runtime poking is only available on Windows".to_owned());
        }
        let key = self.kits[self.active]
            .selected_key
            .clone()
            .ok_or_else(|| "No tag selected".to_owned())?;
        let entry = self
            .entry_for_key(&key)
            .cloned()
            .ok_or_else(|| "Selected tag is no longer in the source".to_owned())?;
        if !matches!(entry.location, TagEntryLocation::Container { .. }) {
            return Err(match entry.location {
                TagEntryLocation::NewContainer { .. } => {
                    "New tags cannot be poked; export them as a mod".to_owned()
                }
                _ => "Poke Current Tag is only for Campaign Evolved container tags".to_owned(),
            });
        }
        let source_data = self.kits[self.active]
            .source
            .as_ref()
            .ok_or_else(|| "No source loaded".to_owned())?;
        let document = self.kits[self.active]
            .parsed_tags
            .get(&key)
            .ok_or_else(|| "Load the selected tag before poking".to_owned())?;
        let edited_bytes = document
            .tag
            .write_to_bytes()
            .map_err(|error| format!("Could not snapshot edited tag: {error}"))?;
        Ok(PokeRequest {
            kit: self.active_kit_id(),
            key,
            source: source_data.source.clone(),
            entries: source_data.full_entry_set().to_vec(),
            entry,
            edited_bytes,
            prior: self.last_poke.clone(),
        })
    }

    pub(super) fn begin_poke_current_tag(&mut self, ctx: egui::Context) {
        if self.poke_direct_running || self.poke_undo_running {
            self.status = "A runtime poke or undo is already in progress".to_owned();
            return;
        }
        let request = match self.current_poke_request() {
            Ok(request) => request,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let PokeRequest {
            kit,
            key,
            source,
            entries,
            entry,
            edited_bytes,
            prior,
        } = request;
        let tx = self.tx.clone();
        self.poke_dialog = Some(PokeDialog {
            kit,
            key: key.clone(),
            state: PokeDialogState::Scanning,
        });
        thread::spawn(move || {
            let result = platform::prepare(source, entries, entry, edited_bytes, prior);
            let _ = tx.send(WorkerMessage::PokePreflightFinished { kit, key, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_poke_current_tag_direct(&mut self, ctx: egui::Context) {
        if self.poke_direct_running || self.poke_undo_running || self.poke_dialog.is_some() {
            self.status = "A runtime poke or undo is already in progress".to_owned();
            return;
        }
        let request = match self.current_poke_request() {
            Ok(request) => request,
            Err(error) => {
                self.status = format!("Poke failed: {error}");
                return;
            }
        };
        let PokeRequest {
            kit,
            key,
            source,
            entries,
            entry,
            edited_bytes,
            prior,
        } = request;
        self.poke_direct_running = true;
        self.status = "Poking current tag…".to_owned();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                platform::prepare(source, entries, entry, edited_bytes, prior).and_then(|plan| {
                    if plan.patches.is_empty() {
                        Ok(None)
                    } else {
                        platform::execute(plan).map(Some)
                    }
                });
            let _ = tx.send(WorkerMessage::PokeDirectFinished { kit, key, result });
            ctx.request_repaint();
        });
    }

    fn confirm_poke(&mut self, ctx: egui::Context) {
        let Some(dialog) = self.poke_dialog.as_mut() else {
            return;
        };
        let PokeDialogState::Ready(plan) = &dialog.state else {
            return;
        };
        let plan = plan.clone();
        let kit = dialog.kit;
        let key = dialog.key.clone();
        dialog.state = PokeDialogState::Writing;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = platform::execute(plan);
            let _ = tx.send(WorkerMessage::PokeWriteFinished { kit, key, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_undo_last_poke(&mut self, ctx: egui::Context) {
        if self.poke_undo_running || self.poke_direct_running || self.poke_dialog.is_some() {
            self.status = "A runtime poke or undo is already in progress".to_owned();
            return;
        }
        let Some(last) = self.last_poke.take() else {
            self.status = "There is no runtime poke to undo".to_owned();
            return;
        };
        self.poke_undo_running = true;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = platform::undo(last);
            let _ = tx.send(WorkerMessage::PokeUndoFinished { result });
            ctx.request_repaint();
        });
    }

    pub(super) fn handle_poke_preflight(
        &mut self,
        kit: KitId,
        key: String,
        result: Result<PokePlan, String>,
    ) {
        let Some(dialog) = self.poke_dialog.as_mut() else {
            return;
        };
        if dialog.kit != kit || dialog.key != key {
            return;
        }
        dialog.state = match result {
            Ok(plan) => PokeDialogState::Ready(plan),
            Err(error) => PokeDialogState::Error(error),
        };
    }

    pub(super) fn handle_poke_write(
        &mut self,
        kit: KitId,
        key: String,
        result: Result<(LastPoke, PokeReport), String>,
    ) {
        let current = self
            .poke_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.kit == kit && dialog.key == key);
        if !current {
            return;
        }
        match result {
            Ok((last, report)) => {
                self.last_poke = Some(last);
                self.status = report.status();
                self.poke_dialog = None;
            }
            Err(error) => {
                if let Some(dialog) = self.poke_dialog.as_mut() {
                    dialog.state = PokeDialogState::Error(error);
                }
            }
        }
    }

    pub(super) fn handle_poke_direct(
        &mut self,
        kit: KitId,
        key: String,
        result: Result<Option<(LastPoke, PokeReport)>, String>,
    ) {
        self.poke_direct_running = false;
        let current = self
            .kits
            .iter()
            .find(|candidate| candidate.id == kit)
            .is_some_and(|candidate| candidate.parsed_tags.contains_key(&key));
        if !current {
            return;
        }
        match result {
            Ok(Some((last, report))) => {
                self.last_poke = Some(last);
                self.status = report.status();
            }
            Ok(None) => {
                self.status = "Poke complete: no changed fields".to_owned();
            }
            Err(error) => {
                self.status = format!("Poke failed: {error}");
            }
        }
    }

    pub(super) fn handle_poke_undo(&mut self, result: Result<PokeReport, String>) {
        self.poke_undo_running = false;
        match result {
            Ok(report) => self.status = report.status(),
            Err(error) => {
                self.status = format!("Undo Last Poke failed: {error}");
                // A failed undo is intentionally not offered again. Its
                // process-bound state may now be only partially applicable.
            }
        }
    }

    pub(super) fn draw_poke_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.poke_dialog.as_ref() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut close = false;
        egui::Window::new("Poke Current Tag")
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .open(&mut open)
            .show(ctx, |ui| match &dialog.state {
                PokeDialogState::Scanning => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            "Resolving the live tag, validating CU2, and building a read-only plan…",
                        );
                    });
                }
                PokeDialogState::Writing => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Writing and verifying runtime memory…");
                    });
                }
                PokeDialogState::Error(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                }
                PokeDialogState::Ready(plan) => {
                    ui.label(format!(
                        "Process: {} (PID {})",
                        PROCESS_NAME, plan.identity.process_id
                    ));
                    ui.label(format!("Build: {}", plan.profile.label));
                    ui.label(format!(
                        "Live tag: {}.{}",
                        plan.tag_path,
                        format_group_tag(plan.group_tag)
                    ));
                    ui.label(format!(
                        "{} supported field change(s), {} byte(s)",
                        plan.patches.len(),
                        plan.byte_count()
                    ));
                    ui.separator();
                    if plan.patches.is_empty() {
                        ui.label("The current tag already matches the shipped runtime values.");
                    } else {
                        ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            for patch in &plan.patches {
                                ui.label(format!(
                                    "{} — {} byte(s) at 0x{:X}",
                                    patch.field_path,
                                    patch.edited.len(),
                                    patch.address
                                ));
                            }
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!plan.patches.is_empty(), egui::Button::new("Poke"))
                            .clicked()
                        {
                            confirm = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            });
        if !open || close {
            self.poke_dialog = None;
        } else if confirm {
            self.confirm_poke(ctx.clone());
        }
    }
}

#[cfg(test)]
mod tests;
