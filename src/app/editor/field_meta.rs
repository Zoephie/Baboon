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

#[cfg(test)]
mod tests {
    use super::*;

    // Editor unit and fixture tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    #[test]
    fn field_meta_uses_foundation_marker_semantics() {
        // Foundation semantics (adopted from `TagFieldNameInfo`): `*` = read-only,
        // `!` = hidden/expert-only (Baboon's `advanced` gate). Presence-tested, so
        // order and combination don't matter — the old `ends_with` parser dropped
        // `*` on `angle*!`.
        let ro = field_display_meta("a position*");
        assert!(ro.read_only && !ro.advanced, "'*' => read-only");

        let hidden = field_display_meta("activity!");
        assert!(
            hidden.advanced && !hidden.read_only,
            "'!' => hidden/advanced"
        );

        let both = field_display_meta("angle*!");
        assert!(both.read_only, "combined: '*' still read-only");
        assert!(both.advanced, "combined: '!' still hidden");
        assert_eq!(both.label, "angle");

        let both_rev = field_display_meta("aabb center!*");
        assert!(both_rev.read_only && both_rev.advanced, "order-independent");
        assert_eq!(both_rev.label, "aabb center");
    }

    #[test]
    fn field_meta_separates_range_from_unit_and_suffix_shows_both() {
        // Range in the unit slot: unit is empty, range captured; suffix shows
        // the type (no unit) followed by the range.
        let m = field_display_meta("acceleration scale:[0,+inf]#marine 1.0, grunt 1.4");
        assert_eq!(m.label, "acceleration scale");
        assert_eq!(m.unit, None);
        assert_eq!(m.range.as_deref(), Some("[0,+inf]"));
        assert_eq!(m.help.as_deref(), Some("marine 1.0, grunt 1.4"));
        assert_eq!(field_suffix(&m, "real"), "real [0,+inf]");

        // Range bare in the name (no colon): pulled out of the label.
        let m = field_display_meta("max sounds per tag [1,16]#max sounds");

        assert_eq!(m.label, "max sounds per tag");
        assert_eq!(m.range.as_deref(), Some("[1,16]"));
        assert_eq!(field_suffix(&m, "long_integer"), "long integer [1,16]");

        // Real unit, no range: unit wins over the type, no range appended.
        let m = field_display_meta("preemption time:ms#replaces after this many ms");
        assert_eq!(m.unit.as_deref(), Some("ms"));
        assert_eq!(m.range, None);
        assert_eq!(field_suffix(&m, "short_integer"), "ms");

        // Unit AND range together: unit first, then range.
        let m = field_display_meta("auto-exposure delay:[0.1-1]seconds#how long");
        assert_eq!(m.unit.as_deref(), Some("seconds"));
        assert_eq!(m.range.as_deref(), Some("[0.1-1]"));
        assert_eq!(field_suffix(&m, "real"), "seconds [0.1-1]");
    }
}
