//! How a field's name and type read in the editor: units, help text and
//! enum labels.

use super::*;

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
