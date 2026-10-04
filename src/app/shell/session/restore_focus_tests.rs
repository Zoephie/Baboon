use super::*;

/// Every completed load makes its own kit active, so a restore's focus can
/// only be honoured once none are left in flight — otherwise whichever
/// source finished last would win, and a loose folder racing a container
/// set has no stable winner. Quitting with Halo 3 focused came back to
/// Campaign Evolved this way.
#[test]
fn the_focus_waits_for_every_restored_kit_to_land() {
    let (halo3, evolved) = (KitId(1), KitId(2));
    let mut restoring = HashSet::from([halo3, evolved]);
    let mut active = Some(halo3);

    assert_eq!(
        focus_after_restore(&mut restoring, &mut active, evolved),
        None,
        "one kit is still loading, so the focus is not settled yet"
    );
    assert_eq!(
        focus_after_restore(&mut restoring, &mut active, halo3),
        Some(halo3),
        "the last landing hands the focus to the kit the session named"
    );
    assert_eq!(active, None, "and it is honoured only once");
}

/// Load order must not change the answer.
#[test]
fn the_focused_kit_wins_whichever_lands_first() {
    let (halo3, evolved) = (KitId(1), KitId(2));
    for order in [[halo3, evolved], [evolved, halo3]] {
        let mut restoring = HashSet::from([halo3, evolved]);
        let mut active = Some(halo3);
        let settled: Vec<_> = order
            .into_iter()
            .filter_map(|kit| focus_after_restore(&mut restoring, &mut active, kit))
            .collect();
        assert_eq!(
            settled,
            [halo3],
            "landing order {order:?} changed the focus"
        );
    }
}

/// A session written before the focused kit was recorded names none, and a
/// load that was never part of a restore must not disturb anything.
#[test]
fn nothing_is_claimed_without_a_named_kit_or_a_restore() {
    let halo3 = KitId(1);
    let mut restoring = HashSet::from([halo3]);
    let mut active = None;
    assert_eq!(
        focus_after_restore(&mut restoring, &mut active, halo3),
        None
    );

    let mut restoring = HashSet::new();
    let mut active = Some(halo3);
    assert_eq!(
        focus_after_restore(&mut restoring, &mut active, KitId(9)),
        None,
        "an ordinary load is not a restore landing"
    );
    assert_eq!(active, Some(halo3), "and leaves the pending focus alone");
}
