use super::*;

/// A dialog that closes after `frames` draws.
struct Countdown {
    instance: u64,
    frames: usize,
}

impl Dialog for Countdown {
    fn show(&mut self, _cx: &Ctx) -> bool {
        self.frames = self.frames.saturating_sub(1);
        self.frames > 0
    }

    fn instance(&self) -> u64 {
        self.instance
    }
}

struct Other;

impl Dialog for Other {
    fn show(&mut self, _cx: &Ctx) -> bool {
        true
    }
}

/// Opening a dialog of an open type and instance replaces it in place; a
/// different instance or type opens beside it.
#[test]
fn opening_replaces_the_same_type_and_instance() {
    let mut host = DialogHost::default();
    host.open(Countdown {
        instance: 0,
        frames: 1,
    });
    host.open(Other);
    host.open(Countdown {
        instance: 0,
        frames: 5,
    });
    assert_eq!(host.open.len(), 2);
    assert_eq!(host.get::<Countdown>().map(|dialog| dialog.frames), Some(5));
    assert!(
        (&*host.open[0] as &dyn Any).is::<Countdown>(),
        "replaced where it stood"
    );

    host.open(Countdown {
        instance: 1,
        frames: 2,
    });
    assert_eq!(host.open.len(), 3);
}

/// A dialog is taken back out of the host by its type.
#[test]
fn closing_hands_the_dialog_back() {
    let mut host = DialogHost::default();
    host.open(Other);
    host.open(Countdown {
        instance: 0,
        frames: 3,
    });
    host.get_mut::<Countdown>().unwrap().frames = 9;
    assert_eq!(
        host.close::<Countdown>().map(|dialog| dialog.frames),
        Some(9)
    );
    assert!(host.get::<Countdown>().is_none());
    assert!(host.get::<Other>().is_some());
    assert!(host.close::<Countdown>().is_none());
}

/// A dialog is found and changed by its type.
#[test]
fn dialogs_are_found_by_type() {
    let mut host = DialogHost::default();
    assert!(host.get::<Countdown>().is_none());
    host.open(Other);
    host.open(Countdown {
        instance: 0,
        frames: 3,
    });
    host.get_mut::<Countdown>().unwrap().frames = 9;
    assert_eq!(host.get::<Countdown>().map(|dialog| dialog.frames), Some(9));
    assert!(host.get::<Other>().is_some());
}

/// Drawing keeps the dialogs that stay open and drops the ones that close.
#[test]
fn drawing_drops_the_dialogs_that_close() {
    let app = Baboon::for_test();
    let ctx = egui::Context::default();
    let mut host = DialogHost::default();
    host.open(Countdown {
        instance: 0,
        frames: 1,
    });
    host.open(Countdown {
        instance: 1,
        frames: 2,
    });
    host.open(Other);
    host.draw(&cx!(app, &ctx));
    assert_eq!(host.open.len(), 2);
    host.draw(&cx!(app, &ctx));
    assert_eq!(host.open.len(), 1);
    assert!(host.get::<Other>().is_some());
}
