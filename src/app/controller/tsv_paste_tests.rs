use super::*;

#[test]
fn header_maps_case_insensitively_reordered_and_ignores_unknown() {
    let columns = vec![
        ("material name".to_owned(), "material name^".to_owned()),
        ("sweetener mode".to_owned(), "sweetener mode".to_owned()),
    ];
    // Reordered, mixed case, plus an unknown column.
    let mapped = map_tsv_header_to_fields("Sweetener Mode\tbogus\tmaterial name", &columns);
    assert_eq!(
        mapped,
        vec![
            Some("sweetener mode".to_owned()),
            None,
            Some("material name^".to_owned()),
        ]
    );
}
