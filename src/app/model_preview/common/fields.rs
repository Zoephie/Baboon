//! Reading a tag's fields as the preview does for every game: a block's
//! elements, an object's inherited block, and flag and function-slot options
//! by name.

use blam_tags::{TagFile, TagStruct};

/// Every element of `holder`'s block `name`, none where it has no such block.
pub(in crate::app::model_preview) fn elements<'a>(holder: TagStruct<'a>, name: &str) -> Vec<TagStruct<'a>> {
    let block = holder.field(name).and_then(|field| field.as_block());
    (0..block.as_ref().map_or(0, |block| block.len()))
        .filter_map(|index| block.as_ref()?.element(index))
        .collect()
}

/// Whether the flags field `name` has its option `option` set, the option
/// named as the editors show it (its `#help` left out).
pub(in crate::app::model_preview) fn has_flag(holder: TagStruct<'_>, name: &str, option: &str) -> bool {
    holder.field(name).and_then(|field| field.flag(option)).is_some_and(|flag| flag.is_set())
}

/// [`has_flag`] for an option older layouts label otherwise: the first of
/// `labels` the field's layout names.
pub(in crate::app::model_preview) fn has_flag_labelled(holder: TagStruct<'_>, name: &str, labels: &[&str]) -> bool {
    let field = holder.field(name);
    let flag = field.and_then(|field| labels.iter().find_map(|label| field.flag(label)));
    flag.is_some_and(|flag| flag.is_set())
}

/// The function slot an `A out` to `D out` option names (a Halo CE change
/// color's `A` to `D`, a Halo 2 one's `primary` to `quaternary`), 0 to 3:
/// none for `none`, for a number its list does not name, and for no field.
pub(in crate::app::model_preview) fn function_slot(field: Option<blam_tags::TagField<'_>>) -> Option<usize> {
    let Some(blam_tags::TagOptions::Enum { names, current }) = field?.options() else {
        return None;
    };
    let held = names.get(usize::try_from(current?).ok()?)?;
    match blam_tags::option_display_name(held).as_ref() {
        "A out" | "A" | "primary" => Some(0),
        "B out" | "B" | "secondary" => Some(1),
        "C out" | "C" | "tertiary" => Some(2),
        "D out" | "D" | "quaternary" => Some(3),
        _ => None,
    }
}

/// [`function_slot`] of `holder`'s field `name`.
pub(in crate::app::model_preview) fn function_slot_of(holder: TagStruct<'_>, name: &str) -> Option<usize> {
    function_slot(holder.field(name))
}

/// Whether `option` is set in any of `holder`'s fields named `name`: a
/// struct can hold more than one (a shader's own `flags` beside its
/// kind's), whose options don't repeat.
pub(in crate::app::model_preview) fn has_flag_in_any(holder: TagStruct<'_>, name: &str, option: &str) -> bool {
    holder
        .fields()
        .filter(|field| field.clean_name() == name)
        .any(|field| field.flag(option).is_some_and(|flag| flag.is_set()))
}

/// Whether the option after `option` in `holder`'s `name` flags is set: for
/// an option whose own name another shares, found by the one before it.
pub(in crate::app::model_preview) fn has_flag_after(holder: TagStruct<'_>, name: &str, option: &str) -> bool {
    let Some(field) = holder.field(name) else {
        return false;
    };
    let Some(bit) = field.flag(option).map(|flag| flag.bit() as usize + 1) else {
        return false;
    };
    match field.options() {
        Some(blam_tags::TagOptions::Flags(options)) => options.get(bit).is_some_and(|flag| flag.is_set),
        _ => false,
    }
}

/// The struct of an object tag holding a block, down its inherited structs.
pub(in crate::app::model_preview) fn object_block<'a>(object: &'a TagFile, block: &str) -> Option<TagStruct<'a>> {
    let mut holder = object.root();
    loop {
        if holder.field(block).is_some() {
            return Some(holder);
        }
        holder = holder.fields_all().find_map(|field| field.as_struct())?;
    }
}
