use super::*;

/// Results belong to the kit the search ran in. They were tagged with the
/// kit focused when they arrived, so switching workspace mid-search gave
/// rows whose keys resolve nowhere.
#[test]
fn field_search_results_belong_to_the_kit_that_ran_the_search() {
    let mut app = Baboon::for_test();
    let searched = app.kits[0].id;
    let stamp = KitStamp {
        kit: searched,
        generation: app.kits[0].generation,
    };
    let other = KitId(searched.0 + 1);
    app.kits.push(Kit::empty(other, TagNameIndex::default()));
    app.active = 1;

    app.handle_field_value_search_finished(stamp, "grass".to_owned(), Ok(Vec::new()));

    let results = app.search.query_results.expect("results");
    assert_eq!(results.kit, searched);
}
