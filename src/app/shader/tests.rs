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

#[cfg(test)]
mod slashed_field_names {
    use super::*;

    /// A shader bool or int parameter is stored in a field literally named
    /// `int/bool`, and the path grammar has no escapes: a segment is the field's
    /// *clean* name, in which that slash is a backslash. Inventing an escape left
    /// the slash separating, so the segment became `int\` + `bool` and every write
    /// failed with "field path no longer resolves" — which is what made
    /// `no_dynamic_lights`, `use_material_texture` and `order3_area_specular`
    /// impossible to enable.
    #[test]
    fn a_slash_in_a_field_name_becomes_a_backslash_not_an_escape() {
        assert_eq!(escape_field_path_segment("int/bool"), "int\\bool");
        assert_eq!(
            shader_param_field_path("render_method", Some(3), "int/bool").as_deref(),
            Some("render_method/parameters[3]/int\\bool")
        );
        // Ordinary names are untouched, and markup is cleaned exactly as the
        // engine's own addressing does.
        assert_eq!(escape_field_path_segment("parameter type"), "parameter type");
        assert_eq!(escape_field_path_segment("parameter name^"), "parameter name");
        // Whatever the engine says a clean name is, this has to agree with it.
        for raw in ["int/bool", "aiming/looking", "max nodes/vertex", "Densities (g/mL)"] {
            assert_eq!(
                escape_field_path_segment(raw),
                blam_tags::field_name::clean_field_name(raw).into_owned(),
                "{raw} disagreed with the engine's clean name"
            );
        }
    }

    /// End to end against the shipped H3 tags: create a bool parameter the way the
    /// grid's "Override Default" does, and read back what landed.
    #[test]
    fn enabling_a_bool_shader_parameter_writes_it() {
        let path = std::path::Path::new(
            "/Users/camden/Halo/halo3_mcc/tags/objects/characters/brute/shaders/armor_lights.shader",
        );
        if !path.exists() {
            eprintln!("skipping: no H3 editing kit");
            return;
        }
        let bytes = std::fs::read(path).expect("read shader");
        let mut tag = blam_tags::TagFile::read_from_bytes(&bytes).expect("parse shader");
        let prefix = render_method_edit_prefix(&tag);
        let block = append_field_path(&prefix, "parameters");

        for name in ["no_dynamic_lights", "use_material_texture", "order3_area_specular"] {
            let op = ShaderParamOp {
                parameters_block_path: block.clone(),
                parameter_name: name.to_owned(),
                initial_fields: vec![
                    ShaderParamInitialField {
                        field: "parameter type".to_owned(),
                        // `bool` in the parameter-type enum.
                        input: "4".to_owned(),
                    },
                    ShaderParamInitialField {
                        field: "int/bool".to_owned(),
                        input: "1".to_owned(),
                    },
                ],
                animated_parameters: Vec::new(),
            };
            let message = apply_one_shader_param_op(&mut tag, &op)
                .unwrap_or_else(|error| panic!("enabling {name} failed: {error}"));
            eprintln!("{message}");
        }

        // The values have to survive a save, not just the in-memory write.
        let saved = tag.write_to_bytes().expect("serialize");
        let reopened = blam_tags::TagFile::read_from_bytes(&saved).expect("reparse");
        let root = reopened.root();
        let parameters = root
            .field_path(&block)
            .and_then(|field| field.as_block())
            .expect("parameters block");
        let mut enabled = Vec::new();
        for index in 0..parameters.len() {
            let Some(element) = parameters.element(index) else { continue };
            let name = element
                .field("parameter name")
                .and_then(|field| field.value())
                .and_then(|value| match value {
                    blam_tags::TagFieldData::StringId(s)
                    | blam_tags::TagFieldData::OldStringId(s) => Some(s.string.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let value = element
                .field_path("int\\bool")
                .and_then(|field| field.value())
                .and_then(|value| match value {
                    blam_tags::TagFieldData::LongInteger(v) => Some(v),
                    _ => None,
                });
            if value == Some(1) {
                enabled.push(name);
            }
        }
        for expected in ["no_dynamic_lights", "use_material_texture", "order3_area_specular"] {
            assert!(
                enabled.iter().any(|name| name == expected),
                "{expected} is not enabled in the saved tag; enabled: {enabled:?}"
            );
        }
    }
}
