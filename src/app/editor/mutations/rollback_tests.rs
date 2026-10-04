use super::*;

const PARAMETERS: &str = "render_method/parameters";

fn parameter_count(tag: &TagFile) -> usize {
    tag.root()
        .field_path(PARAMETERS)
        .and_then(|field| field.as_block())
        .map(|block| block.len())
        .expect("a Halo 3 shader has a parameters block")
}

/// A value typed into a new scalar row that does not parse must fail the
/// whole op. It used to fail only its last step, leaving the parameter it
/// had just added — named, empty, and unknown to the dirty flag.
#[test]
fn a_failed_shader_parameter_op_leaves_no_parameter_behind() {
    let schema = locate_definitions_root().join("halo3_mcc/shader.json");
    let mut tag = TagFile::new(schema).unwrap();
    let before = parameter_count(&tag);

    let result = apply_one_shader_param_op(
        &mut tag,
        &ShaderParamOp {
            parameters_block_path: PARAMETERS.to_owned(),
            parameter_name: "specular_coefficient".to_owned(),
            initial_fields: vec![ShaderParamInitialField {
                field: "real".to_owned(),
                input: "not a number".to_owned(),
            }],
            animated_parameters: Vec::new(),
        },
    );

    assert!(result.is_err());
    assert_eq!(parameter_count(&tag), before);

    // And the same op with a value that parses still adds one.
    apply_one_shader_param_op(
        &mut tag,
        &ShaderParamOp {
            parameters_block_path: PARAMETERS.to_owned(),
            parameter_name: "specular_coefficient".to_owned(),
            initial_fields: vec![ShaderParamInitialField {
                field: "real".to_owned(),
                input: "0.5".to_owned(),
            }],
            animated_parameters: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(parameter_count(&tag), before + 1);
}
