//! Shader model, editing, and thumbnail unit tests.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[cfg(test)]
mod phase4_tests {
    use super::*;

    fn cell(text: &str) -> ShaderGridCell {
        ShaderGridCell {
            text: text.to_owned(),
            value_kind: "value",
            color: None,
        }
    }

    #[test]
    fn differs_compares_value_vs_default() {
        let mut row = empty_shader_grid_row();
        row.default_cell = Some(cell("value: 1.0"));
        row.value_cell = cell("value: 1.0");
        row.is_overridden = true;
        assert!(!row_differs_from_default(&row), "equal values don't differ");
        row.value_cell = cell("value: 2.0");
        assert!(row_differs_from_default(&row), "changed value differs");
        // numeric tolerance: "1" vs "1.0" are equal
        row.value_cell = cell("value: 1");
        assert!(!row_differs_from_default(&row));
        // inherited rows never count as modified, regardless of displayed text
        row.is_overridden = false;
        row.value_cell = cell("Override Default");
        assert!(!row_differs_from_default(&row));
        // no default => never modified
        row.default_cell = None;
        row.value_cell = cell("value: 9");
        assert!(!row_differs_from_default(&row));
    }

    #[test]
    fn downscale_rgba_caps_dimensions_and_preserves_corners() {
        // 4×2 image, two colors per row; downscale to fit within 2px.
        let red = [255u8, 0, 0, 255];
        let blue = [0u8, 0, 255, 255];
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                rgba.extend_from_slice(if x < 2 { &red } else { &blue });
            }
        }
        let (out, w, h) = downscale_rgba(&rgba, 4, 2, 2);
        assert_eq!((w, h), (2, 1), "scaled to fit within 2px, aspect kept");
        assert_eq!(out.len(), w * h * 4);
        // left sample is red, right sample is blue.
        assert_eq!(&out[0..4], &red);
        assert_eq!(&out[4..8], &blue);
        // malformed input yields an empty image.
        assert_eq!(downscale_rgba(&[], 4, 2, 2).1, 0);
    }

    #[test]
    fn reset_op_deletes_sparse_parameter_for_scalar_override() {
        let mut row = empty_shader_grid_row();
        row.default_cell = Some(cell("value: 0.5"));
        row.is_overridden = true;
        row.edit = Some(ShaderRowEdit {
            path: "parameters[0]/value".to_owned(),
            current: "2.0".to_owned(),
            kind: ShaderRowEditKind::Scalar,
        });
        let reset = reset_op_for_row(&row).expect("scalar override is clearable");
        assert_eq!(reset.path, "parameters");
        assert!(matches!(reset.kind, BlockOpKind::Delete(0)));
        // inherited rows do not produce a clear op.
        row.is_overridden = false;
        assert!(reset_op_for_row(&row).is_none());
        // rows without an edit path can't reset
        row.is_overridden = true;
        row.edit = None;
        assert!(reset_op_for_row(&row).is_none());
    }
}
