//! Textual field-value parsing, reference parsing, and display metadata.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

pub(in crate::app) fn is_editable_tag(entry: &TagEntry, tag: &TagFile) -> bool {
    // Loose tags are edited in place; container (Campaign Evolved) tags are
    // edited in memory and written out as an override on Save/Save As/Rename.
    matches!(
        entry.location,
        TagEntryLocation::LooseFile(_) | TagEntryLocation::Container { .. }
    ) && (tag.classic_engine().is_some() || tag.endian == Endian::Le)
}

pub(in crate::app) fn append_field_path(prefix: &str, field_name: &str) -> String {
    if prefix.is_empty() {
        field_name.to_owned()
    } else {
        format!("{prefix}/{field_name}")
    }
}

/// Like `append_field_path` but appends the field's positional `#ordinal`
/// token (`name#N`), so the path resolves to this EXACT field even when a
/// sibling shares its name/type — Foundation-style positional addressing
/// (see `blam_tags::TagField::ordinal`). Use for RESOLVABLE paths (edits,
/// block ops, function data). Canonical/display paths use the plain-name
/// form and strip the ordinal via `strip_node_indices`.
pub(in crate::app) fn append_field_path_for(prefix: &str, field: &TagField<'_>) -> String {
    // Emit the CLEAN (markup-free) name so field-name markup — `:units`,
    // `[range]`, `#help` — can't collide with the path grammar's own `type:`,
    // `[index]`, and `#ordinal` tokens (the range-hint-as-index bug). The engine
    // resolves the ordinal positionally; the clean name is the readable/fallback
    // key. See `blam_tags::field_name` / `blam_tags::TagFieldPath`.
    let segment = format!("{}#{}", field.clean_name(), field.ordinal());
    if prefix.is_empty() {
        segment
    } else {
        format!("{prefix}/{segment}")
    }
}

pub(in crate::app) fn escape_field_path_segment(field_name: &str) -> String {
    field_name.replace('\\', "\\\\").replace('/', "\\/")
}

pub(in crate::app) fn is_text_editable_value(value: &TagFieldData) -> bool {
    !matches!(
        value,
        TagFieldData::Data(_)
            | TagFieldData::ApiInterop(_)
            | TagFieldData::Custom(_)
            | TagFieldData::Point2d(_)
            | TagFieldData::Rectangle2d(_)
            | TagFieldData::RealPoint2d(_)
            | TagFieldData::RealPoint3d(_)
            | TagFieldData::RealVector2d(_)
            | TagFieldData::RealVector3d(_)
            | TagFieldData::RealQuaternion(_)
            | TagFieldData::RealEulerAngles2d(_)
            | TagFieldData::RealEulerAngles3d(_)
            | TagFieldData::RealPlane2d(_)
            | TagFieldData::RealPlane3d(_)
            | TagFieldData::RgbColor(_)
            | TagFieldData::ArgbColor(_)
            | TagFieldData::RealRgbColor(_)
            | TagFieldData::RealArgbColor(_)
            | TagFieldData::RealHsvColor(_)
            | TagFieldData::RealAhsvColor(_)
            | TagFieldData::ShortIntegerBounds(_)
            | TagFieldData::AngleBounds(_)
            | TagFieldData::RealBounds(_)
            | TagFieldData::FractionBounds(_)
    )
}

pub(in crate::app) fn parse_gui_field_value(
    field: &TagField<'_>,
    input: &str,
) -> Result<TagFieldData, String> {
    let trimmed = input.trim();
    match field.field_type() {
        TagFieldType::CharInteger => parse_value(trimmed, "i8").map(TagFieldData::CharInteger),
        TagFieldType::ShortInteger => parse_value(trimmed, "i16").map(TagFieldData::ShortInteger),
        TagFieldType::LongInteger => parse_value(trimmed, "i32").map(TagFieldData::LongInteger),
        TagFieldType::Int64Integer => parse_value(trimmed, "i64").map(TagFieldData::Int64Integer),
        TagFieldType::ByteInteger => parse_value(trimmed, "u8").map(TagFieldData::ByteInteger),
        TagFieldType::WordInteger => parse_value(trimmed, "u16").map(TagFieldData::WordInteger),
        TagFieldType::DwordInteger => parse_value(trimmed, "u32").map(TagFieldData::DwordInteger),
        TagFieldType::QwordInteger => parse_value(trimmed, "u64").map(TagFieldData::QwordInteger),
        TagFieldType::Tag => parse_group_tag(trimmed)
            .map(TagFieldData::Tag)
            .ok_or_else(|| "expected 1..=4 ASCII group tag".to_owned()),
        TagFieldType::Angle => parse_value(trimmed, "f32").map(TagFieldData::Angle),
        TagFieldType::ShortIntegerBounds => {
            let (lower, upper) = parse_short_bounds(trimmed, "short bounds")?;
            Ok(TagFieldData::ShortIntegerBounds(
                blam_tags::math::ShortBounds { lower, upper },
            ))
        }
        TagFieldType::AngleBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "angle bounds")?;
            Ok(TagFieldData::AngleBounds(blam_tags::math::AngleBounds {
                lower,
                upper,
            }))
        }
        TagFieldType::RealBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "real bounds")?;
            Ok(TagFieldData::RealBounds(blam_tags::math::RealBounds {
                lower,
                upper,
            }))
        }
        TagFieldType::FractionBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "fraction bounds")?;
            Ok(TagFieldData::FractionBounds(
                blam_tags::math::FractionBounds { lower, upper },
            ))
        }
        TagFieldType::RealVector2d => {
            let [i, j] = parse_float_channels::<2>(trimmed, "real vector 2d")?;
            Ok(TagFieldData::RealVector2d(blam_tags::math::RealVector2d {
                i,
                j,
            }))
        }
        TagFieldType::RealVector3d => {
            let [i, j, k] = parse_float_channels::<3>(trimmed, "real vector 3d")?;
            Ok(TagFieldData::RealVector3d(blam_tags::math::RealVector3d {
                i,
                j,
                k,
            }))
        }
        TagFieldType::RealPoint2d => {
            let [x, y] = parse_float_channels::<2>(trimmed, "real point 2d")?;
            Ok(TagFieldData::RealPoint2d(blam_tags::math::RealPoint2d {
                x,
                y,
            }))
        }
        TagFieldType::RealPoint3d => {
            let [x, y, z] = parse_float_channels::<3>(trimmed, "real point 3d")?;
            Ok(TagFieldData::RealPoint3d(blam_tags::math::RealPoint3d {
                x,
                y,
                z,
            }))
        }
        TagFieldType::RealQuaternion => {
            let [i, j, k, w] = parse_float_channels::<4>(trimmed, "real quaternion")?;
            Ok(TagFieldData::RealQuaternion(
                blam_tags::math::RealQuaternion { i, j, k, w },
            ))
        }
        TagFieldType::Real => parse_value(trimmed, "f32").map(TagFieldData::Real),
        TagFieldType::RealSlider => parse_value(trimmed, "f32").map(TagFieldData::RealSlider),
        TagFieldType::RealFraction => parse_value(trimmed, "f32").map(TagFieldData::RealFraction),
        TagFieldType::CharEnum => Ok(TagFieldData::CharEnum {
            value: parse_enum_value(field, trimmed)? as i8,
            name: None,
        }),
        TagFieldType::ShortEnum => Ok(TagFieldData::ShortEnum {
            value: parse_enum_value(field, trimmed)? as i16,
            name: None,
        }),
        TagFieldType::LongEnum => Ok(TagFieldData::LongEnum {
            value: parse_enum_value(field, trimmed)?,
            name: None,
        }),
        TagFieldType::ByteFlags => Ok(TagFieldData::ByteFlags {
            value: parse_int_mask(trimmed)? as u8,
            names: Vec::new(),
        }),
        TagFieldType::WordFlags => Ok(TagFieldData::WordFlags {
            value: parse_int_mask(trimmed)? as u16,
            names: Vec::new(),
        }),
        TagFieldType::LongFlags => Ok(TagFieldData::LongFlags {
            value: parse_int_mask(trimmed)? as i32,
            names: Vec::new(),
        }),
        TagFieldType::ByteBlockFlags => {
            Ok(TagFieldData::ByteBlockFlags(parse_int_mask(trimmed)? as u8))
        }
        TagFieldType::WordBlockFlags => {
            Ok(TagFieldData::WordBlockFlags(parse_int_mask(trimmed)? as u16))
        }
        TagFieldType::LongBlockFlags => {
            Ok(TagFieldData::LongBlockFlags(parse_int_mask(trimmed)? as i32))
        }
        TagFieldType::CharBlockIndex => Ok(TagFieldData::CharBlockIndex(
            parse_block_index(trimmed)? as i8,
        )),
        TagFieldType::CustomCharBlockIndex => Ok(TagFieldData::CustomCharBlockIndex(
            parse_block_index(trimmed)? as i8,
        )),
        TagFieldType::ShortBlockIndex => Ok(TagFieldData::ShortBlockIndex(parse_block_index(
            trimmed,
        )? as i16)),
        TagFieldType::CustomShortBlockIndex => Ok(TagFieldData::CustomShortBlockIndex(
            parse_block_index(trimmed)? as i16,
        )),
        TagFieldType::LongBlockIndex => {
            Ok(TagFieldData::LongBlockIndex(parse_block_index(trimmed)?))
        }
        TagFieldType::CustomLongBlockIndex => Ok(TagFieldData::CustomLongBlockIndex(
            parse_block_index(trimmed)?,
        )),
        TagFieldType::String => Ok(TagFieldData::String(trimmed.to_owned())),
        TagFieldType::LongString => Ok(TagFieldData::LongString(trimmed.to_owned())),
        TagFieldType::StringId => Ok(TagFieldData::StringId(StringIdData {
            string: parse_none_string(trimmed),
        })),
        TagFieldType::OldStringId => Ok(TagFieldData::OldStringId(StringIdData {
            string: parse_none_string(trimmed),
        })),
        TagFieldType::TagReference => parse_tag_reference(trimmed).map(TagFieldData::TagReference),
        // Color values: comma-separated floats, written by the color picker
        // swatch. RGB = "r, g, b"; ARGB = "a, r, g, b".
        TagFieldType::RgbColor => {
            let (_, r, g, b) = parse_rgb_or_argb_color_channels(trimmed)?;
            let raw = ((color_float_to_u8(r) as u32) << 16)
                | ((color_float_to_u8(g) as u32) << 8)
                | color_float_to_u8(b) as u32;
            Ok(TagFieldData::RgbColor(blam_tags::math::RgbColor(raw)))
        }
        TagFieldType::ArgbColor => {
            let (a, r, g, b) = parse_rgb_or_argb_color_channels(trimmed)?;
            let raw = ((color_float_to_u8(a) as u32) << 24)
                | ((color_float_to_u8(r) as u32) << 16)
                | ((color_float_to_u8(g) as u32) << 8)
                | color_float_to_u8(b) as u32;
            Ok(TagFieldData::ArgbColor(blam_tags::math::ArgbColor(raw)))
        }
        TagFieldType::RealRgbColor => {
            let [r, g, b] = parse_color_channels::<3>(trimmed)?;
            Ok(TagFieldData::RealRgbColor(blam_tags::math::RealRgbColor {
                red: r,
                green: g,
                blue: b,
            }))
        }
        TagFieldType::RealArgbColor => {
            let [a, r, g, b] = parse_color_channels::<4>(trimmed)?;
            Ok(TagFieldData::RealArgbColor(
                blam_tags::math::RealArgbColor {
                    alpha: a,
                    red: r,
                    green: g,
                    blue: b,
                },
            ))
        }
        // Raw byte blobs (e.g. a `mapping_function` `data` field) are
        // carried through the string edit channel as lowercase hex. The
        // function editor produces these via `push_function_edit`.
        TagFieldType::Data => decode_hex(trimmed).map(TagFieldData::Data),
        _ => Err(format!(
            "editing {} fields is not supported yet",
            field.type_name()
        )),
    }
}

/// Decode a contiguous lowercase/uppercase hex string (no separators)
/// into bytes. Used to ferry function blobs through `PendingFieldEdit`.
pub(in crate::app) fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    let s = input.trim();
    if s.len() % 2 != 0 {
        return Err("hex blob must have an even number of digits".to_owned());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| "invalid hex digit".to_owned())?;
        let lo = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| "invalid hex digit".to_owned())?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

/// Encode bytes as a contiguous lowercase hex string.
pub(in crate::app) fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0xF) as u32, 16).unwrap());
    }
    out
}

pub(in crate::app) fn parse_value<T: std::str::FromStr>(
    input: &str,
    expected: &str,
) -> Result<T, String> {
    input
        .parse()
        .map_err(|_| format!("expected {expected} value"))
}

/// Parse exactly `N` comma-separated float channels (used for color values).
pub(in crate::app) fn parse_color_channels<const N: usize>(
    input: &str,
) -> Result<[f32; N], String> {
    let parts: Vec<f32> = input
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("expected {N} comma-separated color channels"))?;
    parts
        .try_into()
        .map_err(|_: Vec<f32>| format!("expected {N} comma-separated color channels"))
}

pub(in crate::app) fn parse_rgb_or_argb_color_channels(
    input: &str,
) -> Result<(f32, f32, f32, f32), String> {
    let parts = input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| parse_value::<f32>(part, "color channel"))
        .collect::<Result<Vec<_>, _>>()?;
    match parts.as_slice() {
        [r, g, b] => Ok((1.0, *r, *g, *b)),
        [a, r, g, b] => Ok((*a, *r, *g, *b)),
        _ => Err("expected 3 or 4 comma-separated color channels".to_owned()),
    }
}

pub(in crate::app) fn color_float_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(in crate::app) fn parse_float_channels<const N: usize>(
    input: &str,
    expected: &str,
) -> Result<[f32; N], String> {
    let parts = if input.contains(',') {
        input
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
    } else {
        input.split_whitespace().collect::<Vec<_>>()
    };
    let values = parts
        .into_iter()
        .map(|part| part.parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("expected {N} values for {expected}"))?;
    values
        .try_into()
        .map_err(|_: Vec<f32>| format!("expected {N} values for {expected}"))
}

pub(in crate::app) fn parse_float_bounds(
    input: &str,
    expected: &str,
) -> Result<(f32, f32), String> {
    let (lower, upper) = parse_bounds_parts(input, expected)?;
    Ok((
        lower
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
        upper
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
    ))
}

pub(in crate::app) fn parse_short_bounds(
    input: &str,
    expected: &str,
) -> Result<(i16, i16), String> {
    let (lower, upper) = parse_bounds_parts(input, expected)?;
    Ok((
        lower
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
        upper
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
    ))
}

pub(in crate::app) fn parse_bounds_parts<'a>(
    input: &'a str,
    expected: &str,
) -> Result<(&'a str, &'a str), String> {
    if let Some((lower, upper)) = input.split_once("..") {
        return Ok((lower.trim(), upper.trim()));
    }

    if let Some((lower, upper)) = input.split_once(" to ") {
        return Ok((lower.trim(), upper.trim()));
    }

    let parts = if input.contains(',') {
        input
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
    } else {
        input.split_whitespace().collect::<Vec<_>>()
    };
    let [lower, upper]: [&str; 2] = parts
        .try_into()
        .map_err(|_| format!("expected {expected} as lower..upper"))?;
    Ok((lower, upper))
}

pub(in crate::app) fn parse_none_string(input: &str) -> String {
    if input.eq_ignore_ascii_case("none") {
        String::new()
    } else {
        input.to_owned()
    }
}

pub(in crate::app) fn parse_block_index(input: &str) -> Result<i32, String> {
    if input.eq_ignore_ascii_case("none") {
        Ok(-1)
    } else {
        parse_value(input, "block index")
    }
}

pub(in crate::app) fn parse_int_mask(input: &str) -> Result<u64, String> {
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).map_err(|_| "expected integer mask".to_owned())
    } else {
        input
            .parse()
            .map_err(|_| "expected integer mask".to_owned())
    }
}

pub(in crate::app) fn parse_enum_value(field: &TagField<'_>, input: &str) -> Result<i32, String> {
    if let Ok(value) = input.parse() {
        return Ok(value);
    }
    let Some(blam_tags::TagOptions::Enum { names, .. }) = field.options() else {
        return Err("expected enum name or integer".to_owned());
    };
    let Some((index, _)) = names
        .iter()
        .enumerate()
        .find(|(_, name)| name.eq_ignore_ascii_case(input))
    else {
        return Err("expected enum name or integer".to_owned());
    };
    Ok(index as i32)
}

pub(in crate::app) fn parse_tag_reference(input: &str) -> Result<TagReferenceData, String> {
    if input.eq_ignore_ascii_case("none") || input.is_empty() {
        return Ok(TagReferenceData {
            group_tag_and_name: None,
        });
    }
    if let Some((group, path)) = input.split_once(':') {
        let group_tag = parse_group_tag(group)
            .ok_or_else(|| "tag reference group must be 1..=4 ASCII chars".to_owned())?;
        return Ok(TagReferenceData {
            group_tag_and_name: Some((group_tag, path.replace('/', "\\"))),
        });
    }
    let Some((path, extension)) = input.rsplit_once('.') else {
        return Err("expected <path>.<group> or GROUP:<path>".to_owned());
    };
    let group_tag = extension_to_group_tag(extension)
        .or_else(|| parse_group_tag(extension))
        .ok_or_else(|| format!("unknown tag group {extension:?}"))?;
    Ok(TagReferenceData {
        group_tag_and_name: Some((group_tag, path.replace('/', "\\"))),
    })
}

pub(in crate::app) fn field_display_meta(name: &str) -> FieldDisplayMeta {
    // The engine owns the canonical field-name markup grammar (Foundation's
    // `TagFieldNameInfo`). We map its decomposition onto Baboon's display meta.
    // Note the Foundation marker semantics adopted here: `*` = read-only,
    // `!` = hidden/expert-only (Baboon's `advanced` gate). See
    // `blam_tags::field_name`.
    let info = blam_tags::parse_field_name(name);
    FieldDisplayMeta {
        label: info.clean_name.into_owned(),
        unit: info.units.map(str::to_owned),
        range: info.range.map(str::to_owned),
        help: info.description.map(str::to_owned),
        tag_reference_allowed: Vec::new(),
        read_only: info.read_only,
        advanced: info.hidden,
    }
}

/// Metadata shown after a field's value: the unit (preferred over the type
/// name), then the `[range]` hint if present.
pub(in crate::app) fn field_suffix(meta: &FieldDisplayMeta, type_name: &str) -> String {
    let base = meta
        .unit
        .clone()
        .unwrap_or_else(|| clean_type_name(type_name));
    match &meta.range {
        Some(range) => {
            if base.is_empty() {
                range.clone()
            } else {
                format!("{base} {range}")
            }
        }
        None => base,
    }
}

pub(in crate::app) fn draw_field_help(ui: &mut Ui, meta: &FieldDisplayMeta) {
    // Field documentation is shown on hover over the name label (see
    // `foundation_label_cell`); this only surfaces the read-only marker.
    if meta.read_only {
        ui.label(RichText::new("read-only").color(subtle_dark()).small());
    }
}

pub(in crate::app) fn enum_option_label(options: &[&str], selected: i64) -> String {
    if selected < 0 {
        return "NONE".to_owned();
    }
    options
        .get(selected as usize)
        .map(|name| format!("{selected}. {name}"))
        .unwrap_or_else(|| selected.to_string())
}

pub(in crate::app) fn extension_to_group_tag(extension: &str) -> Option<u32> {
    let fourcc = match extension {
        "material" => "mat",
        "material_shader" => "mats",
        "material_effects" => "foot",
        "object" => "obje",
        "model" => "hlmt",
        "character" => "char",

        "style" => "styl",
        "unit" => "unit",
        "render_model" => "mode",
        "collision_model" => "coll",
        "physics_model" => "phmo",
        "model_animation_graph" => "jmad",
        "biped" => "bipd",
        "vehicle" => "vehi",
        "weapon" => "weap",
        "equipment" => "eqip",
        "item" => "item",
        "giant" => "gint",
        "creature" => "crea",
        "scenery" => "scen",
        "crate" => "crat",
        "bitmap" => "bitm",
        "scenario_structure_bsp" => "sbsp",
        "structure_design" => "sddt",
        "scenario" => "scnr",
        "projectile" => "proj",
        "effect" => "effe",
        "effect_scenery" => "efsc",
        "damage_effect" => "jpt!",
        "sound" => "snd!",
        "sound_looping" => "lsnd",
        "sound_scenery" => "ssce",
        "dialogue" => "udlg",
        "light" => "ligh",
        "lens_flare" => "lens",
        "camera_track" => "trak",
        "device" => "devi",
        "device_control" => "ctrl",
        "device_machine" => "mach",
        "device_terminal" => "term",
        "globals" => "matg",
        "shader" => "rmsh",
        "shader_terrain" => "rmtr",
        "shader_water" => "rmw ",
        "shader_foliage" => "rmfl",
        "shader_decal" => "rmd ",
        "shader_halogram" => "rmhg",
        "shader_skin" => "rmsk",
        "shader_cortana" => "rmct",
        "shader_custom" => "rmcs",
        "shader_particle" => "rmp ",
        "shader_beam" => "rmb ",
        "shader_contrail" => "rmco",
        "shader_light_volume" => "rmlv",
        _ => return None,
    };
    parse_group_tag(fourcc)
}
