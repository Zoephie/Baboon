//! Views of vertex and index data as the bytes GL takes, and a slot's draw
//! range.

pub(in crate::app::model_preview) fn model_draw_range(
    index_start: u32,
    index_count: u32,
    total_indices: usize,
) -> Option<(i32, i32)> {
    let start = index_start as usize;
    let available = total_indices.checked_sub(start)?;
    let count = (index_count as usize).min(available);
    let count = count - count % 3;
    if count == 0 || count > i32::MAX as usize {
        return None;
    }
    let byte_offset = start.checked_mul(std::mem::size_of::<u32>())?;
    Some((i32::try_from(byte_offset).ok()?, count as i32))
}

pub(in crate::app::model_preview) fn slice_bytes<T>(slice: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice)) }
}
