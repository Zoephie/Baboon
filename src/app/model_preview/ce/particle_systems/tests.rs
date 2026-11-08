use super::*;

#[test]
fn states_end_go_round_or_come_back_as_the_flags_say() {
    let mut forward = true;
    assert_eq!(next_state(0, &mut forward, 3, false, false), Some(1));
    assert_eq!(next_state(2, &mut forward, 3, false, false), None);
    assert_eq!(next_state(2, &mut forward, 3, true, false), Some(0));
    assert!(forward);
    assert_eq!(next_state(2, &mut forward, 3, true, true), Some(1));
    assert!(!forward);
    assert_eq!(next_state(1, &mut forward, 3, true, true), Some(0));
    assert_eq!(next_state(0, &mut forward, 3, true, true), Some(1));
    assert!(forward);
}
