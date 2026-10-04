use super::*;

fn queued<const N: usize>(actions: [SoundAction; N]) -> VecDeque<SoundRequest> {
    actions.into_iter().map(SoundRequest::from).collect()
}

fn pcm(samples: usize) -> Arc<Waveform> {
    Arc::new(Waveform::new(Arc::new(DecodedPcm {
        samples: vec![0; samples],
        channels: 1,
        sample_rate: 48_000,
    })))
}

/// Every audition used to stay decoded for the rest of the session.
#[test]
fn the_pcm_cache_drops_the_least_recently_played_first() {
    let mut cache = PcmCache::<u32> {
        budget: 250,
        ..Default::default()
    };
    let each = pcm(50).bytes(); // 100 bytes of samples and a summary
    cache.budget = each * 2 + each / 2;
    cache.insert(1, pcm(50));
    cache.insert(2, pcm(50));
    assert!(cache.get(&1).is_some(), "replaying 1 makes 2 the oldest");
    cache.insert(3, pcm(50));
    assert!(cache.get(&2).is_none(), "over budget: 2 went");
    assert!(cache.get(&1).is_some() && cache.get(&3).is_some());
    assert_eq!(cache.bytes, each * 2);
}

fn decoded(request: u64, cache: Option<PcmKey>) -> AudioDone {
    AudioDone::Decoded {
        request,
        cache,
        label: "rifle_fire".to_owned(),
        owner: None,
        clip: None,
        preview: false,
        result: Ok(pcm(8)),
    }
}

/// A decode that lands after the user moved on is kept for next time but
/// not played over what they are listening to now.
#[test]
fn a_superseded_decode_is_cached_but_not_played() {
    let mut audio = AudioState {
        // No output device, so playing says so instead of opening one.
        engine_tried: true,
        play_request: 2,
        ..Default::default()
    };
    audio.jobs.running = 1;
    let key = || PcmKey::Event {
        generation: 0,
        name: "rifle_fire".to_owned(),
    };
    audio.apply_job(decoded(1, Some(key())));
    assert_eq!(audio.status, None, "request 1 is stale: not played");
    assert!(
        audio.event_cache.get(&"rifle_fire".to_owned()).is_some(),
        "but cached"
    );

    audio.apply_job(decoded(2, None));
    assert_eq!(audio.status.as_deref(), Some("no audio output device"));
    assert_eq!(audio.jobs.running, 0);
}

/// Event names must not be cached against a replacement Wwise bank set.
#[test]
fn a_decode_for_reopened_wwise_banks_is_not_cached() {
    let mut audio = AudioState {
        engine_tried: true,
        wwise_generation: 3,
        ..Default::default()
    };
    audio.apply_job(decoded(
        0,
        Some(PcmKey::Event {
            generation: 2,
            name: "rifle_fire".to_owned(),
        }),
    ));
    assert!(audio.event_cache.get(&"rifle_fire".to_owned()).is_none());
}

fn missing_bank_fixture(name: &str) -> (PathBuf, ExtractRequest, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "baboon-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let tags = root.join("tags");
    std::fs::create_dir_all(&tags).unwrap();
    let output = root.join("data/sound/test.wav");
    let request = ExtractRequest {
        items: vec![super::super::sound_extract::ExtractItem {
            out_path: output.clone(),
            source: ExtractSource::Bank {
                id: Some(123),
                key: "test".to_owned(),
                language: None,
            },
        }],
        tags_root: Some(tags),
        label: "test sound".to_owned(),
    };
    (root, request, output)
}

#[test]
fn extraction_without_fmod_banks_is_cancelled_before_writing() {
    let (root, request, output) = missing_bank_fixture("missing-extract-bank");
    let mut audio = AudioState::default();

    audio.run_extract(request, &egui::Context::default());

    assert!(
        audio
            .status
            .as_deref()
            .is_some_and(|status| status.starts_with("Extraction cancelled — FMOD banks"))
    );
    assert!(!output.exists());
    assert_eq!(
        audio.jobs.running, 0,
        "a cancelled extraction starts no worker"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn player_reports_the_missing_fmod_bank_location() {
    let (root, request, _) = missing_bank_fixture("missing-player-bank");
    let tags = request.tags_root.unwrap();
    let mut audio = AudioState {
        pending: queued([SoundAction::Play {
            id: Some(123),
            key: "test".to_owned(),
            label: "test".to_owned(),
            tags_root: None,
        }]),
        ..Default::default()
    };

    audio.process(Some(&tags), &egui::Context::default());

    let status = audio.status.as_deref().expect("visible player error");
    assert!(status.starts_with("FMOD audio unavailable:"));
    let expected = root.join("fmod").join("pc").display().to_string();
    assert!(
        status.contains(&expected),
        "{status:?} did not contain {expected:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn player_uses_the_tags_own_kit_instead_of_the_active_kit() {
    let (root, request, _) = missing_bank_fixture("player-owning-kit");
    let owning_tags = request.tags_root.unwrap();
    let other_tags = root.join("other-kit/tags");
    std::fs::create_dir_all(&other_tags).unwrap();
    let mut audio = AudioState {
        pending: queued([SoundAction::Play {
            id: Some(123),
            key: "test".to_owned(),
            label: "test".to_owned(),
            tags_root: Some(owning_tags),
        }]),
        ..Default::default()
    };

    audio.process(Some(&other_tags), &egui::Context::default());

    let status = audio.status.as_deref().expect("visible player error");
    let expected = root.join("fmod").join("pc").display().to_string();
    let wrong = root
        .join("other-kit")
        .join("fmod")
        .join("pc")
        .display()
        .to_string();
    assert!(
        status.contains(&expected),
        "{status:?} did not contain {expected:?}"
    );
    assert!(
        !status.contains(&wrong),
        "player used the active kit: {status:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn language_fallback_does_not_overwrite_the_play_request() {
    let (root, request, _) = missing_bank_fixture("language-then-play");
    let tags = request.tags_root.unwrap();
    let play = SoundAction::Play {
        id: Some(123),
        key: "test".to_owned(),
        label: "test".to_owned(),
        tags_root: Some(tags.clone()),
    };
    let mut audio = AudioState {
        language: Some("language-from-another-kit".to_owned()),
        pending: queued([SoundAction::SetLanguage(None), play]),
        ..Default::default()
    };

    audio.process(Some(&tags), &egui::Context::default());
    assert_eq!(audio.language, None);
    assert_eq!(audio.pending.len(), 1, "Play was lost behind SetLanguage");

    audio.process(Some(&tags), &egui::Context::default());
    assert!(
        audio
            .status
            .as_deref()
            .is_some_and(|status| status.starts_with("FMOD audio unavailable:")),
        "the preserved Play action was not processed: {:?}",
        audio.status
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn player_retries_a_failed_fmod_open_without_a_restart() {
    let (root, request, _) = missing_bank_fixture("retry-player-bank");
    let tags = request.tags_root.unwrap();
    let play = || SoundAction::Play {
        id: Some(123),
        key: "test".to_owned(),
        label: "test".to_owned(),
        tags_root: None,
    };
    let mut audio = AudioState {
        pending: queued([play()]),
        ..Default::default()
    };
    audio.process(Some(&tags), &egui::Context::default());
    let first = audio.status.clone().expect("first bank error");

    let bank_dir = root.join("fmod").join("pc");
    std::fs::create_dir_all(&bank_dir).unwrap();
    std::fs::write(bank_dir.join("sfx.fsb"), b"new but invalid").unwrap();
    audio.pending.push_back(play().into());
    audio.process(Some(&tags), &egui::Context::default());
    let second = audio.status.clone().expect("retried bank error");

    assert_ne!(first, second, "the original missing-bank result was cached");
    assert!(second.contains("read header"), "{second}");
    let _ = std::fs::remove_dir_all(root);
}

fn owner(kit: u64, key: &str) -> SoundOwner {
    SoundOwner {
        kit: KitId(kit),
        key: key.to_owned(),
    }
}

/// Two-channel frames whose samples say where they came from:
/// frame `f` holds `(f * 10, f * 10 + 1)`.
fn numbered(frames: i16) -> Arc<DecodedPcm> {
    Arc::new(DecodedPcm {
        samples: (0..frames)
            .flat_map(|f| [f.wrapping_mul(10), f.wrapping_mul(10).wrapping_add(1)])
            .collect(),
        channels: 2,
        sample_rate: 1000,
    })
}

fn wave(frames: i16) -> Arc<Waveform> {
    Arc::new(Waveform::new(numbered(frames)))
}

fn pcm_source(pcm: &Arc<DecodedPcm>, looping: bool) -> (PcmSource, Arc<PlaybackShared>) {
    let shared = Arc::new(PlaybackShared::new(looping, 1.0));
    let source = PcmSource::new(pcm.clone(), shared.clone(), 0);
    (source, shared)
}

/// The source plays the shared buffer in order, reports where it is a
/// whole frame at a time, takes a seek at the next frame boundary, and
/// either ends or wraps at the end.
#[test]
fn the_pcm_source_reports_position_seeks_and_loops() {
    let pcm = numbered(4);
    let (mut source, shared) = pcm_source(&pcm, false);
    assert_eq!(source.next(), Some(0));
    assert_eq!(
        shared.frame.load(Ordering::Relaxed),
        0,
        "half a frame is not a frame"
    );
    assert_eq!(source.next(), Some(1));
    assert_eq!(shared.frame.load(Ordering::Relaxed), 1);
    shared.seek.store(3, Ordering::Relaxed);
    assert_eq!(source.next(), Some(30), "the seek lands on the next frame");
    assert_eq!(source.next(), Some(31));
    assert_eq!(source.next(), None, "no loop: the sound ends");
    assert_eq!(shared.frame.load(Ordering::Relaxed), 4);

    let (mut source, shared) = pcm_source(&pcm, true);
    shared.seek.store(3, Ordering::Relaxed);
    let played: Vec<i16> = source.by_ref().take(4).collect();
    assert_eq!(played, [30, 31, 0, 1], "looping wraps to the start");
}

/// A voice with no sink — nothing is playing — still keeps a playhead the
/// player can move and read.
#[test]
fn a_stopped_voice_keeps_a_playhead_the_player_can_move() {
    let voice = Voice::new(wave(1000), "x".to_owned(), None, None, false, 1.0);
    voice.seek(250);
    let view = voice.view();
    assert_eq!(view.position, 0.25);
    assert_eq!(view.duration, 1.0);
    assert!(!view.playing);
    voice.seek(5000);
    assert_eq!(
        voice.view().position,
        1.0,
        "a seek past the end stops at the end"
    );
}

/// Closing a tab disposes of its sound, and of a decode or Wwise load
/// still on its way for it. Another tab closing leaves it alone.
#[test]
fn closing_its_tab_disposes_of_the_sound_and_what_is_coming_for_it() {
    let a = owner(1, "file:a.sound");
    let mut audio = AudioState {
        voice: Some(Voice::new(
            wave(10),
            "a".to_owned(),
            Some(a.clone()),
            None,
            false,
            1.0,
        )),
        decode_owner: Some(a.clone()),
        wwise_deferred: Some(("event".to_owned(), "a".to_owned(), Some(a.clone()), None)),
        play_request: 7,
        ..Default::default()
    };

    audio.follow_tabs(None, |owner| owner.key != "file:b.sound");
    assert!(audio.voice.is_some() && audio.wwise_deferred.is_some());
    assert_eq!(audio.play_request, 7, "another tab closing cancels nothing");

    audio.follow_tabs(None, |owner| owner.key != "file:a.sound");
    assert!(
        audio.voice.is_none(),
        "the closed tab's sound is disposed of"
    );
    assert!(audio.wwise_deferred.is_none(), "its Wwise play is dropped");
    assert_eq!(
        audio.play_request, 8,
        "its decode will not play when it lands"
    );
    assert!(audio.playback(Some(&a)).is_none());
}

/// The transport in one tab does not reach another tab's sound, and each
/// tab sees only its own.
#[test]
fn transport_from_another_tab_does_not_move_this_tab_s_sound() {
    let a = owner(1, "file:a.sound");
    let b = owner(1, "file:b.sound");
    let mut audio = AudioState {
        voice: Some(Voice::new(
            wave(1000),
            "a".to_owned(),
            Some(a.clone()),
            None,
            false,
            1.0,
        )),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    audio.pending.push_back(SoundRequest {
        owner: Some(b.clone()),
        clip: None,
        preview: false,
        action: SoundAction::Seek(0.5),
    });
    audio.process(None, &ctx);
    assert_eq!(audio.playback(Some(&a)).unwrap().position, 0.0);
    assert!(audio.playback(Some(&b)).is_none());

    audio.pending.push_back(SoundRequest {
        owner: Some(a.clone()),
        clip: None,
        preview: false,
        action: SoundAction::Seek(0.5),
    });
    audio.process(None, &ctx);
    assert_eq!(audio.playback(Some(&a)).unwrap().position, 0.5);
}

/// Focus moving to another tab pauses the sound; coming back does not
/// start it again. Needs an output device, so it skips without one.
#[test]
fn another_tab_taking_focus_pauses_the_sound() {
    let a = owner(1, "file:a.sound");
    let b = owner(1, "file:b.sound");
    let mut audio = AudioState::default();
    if audio.ensure_engine().is_none() {
        eprintln!("skipping: no audio output device");
        return;
    }
    audio.volume = Volume(0.0);
    audio.play_decoded(wave(30_000), "a", Some(a.clone()), None);
    assert!(audio.playback(Some(&a)).unwrap().playing);

    audio.follow_tabs(Some(&a), |_| true);
    assert!(
        audio.playback(Some(&a)).unwrap().playing,
        "its own tab keeps it playing"
    );

    audio.follow_tabs(Some(&b), |_| true);
    assert!(
        !audio.playback(Some(&a)).unwrap().playing,
        "another tab's focus pauses it"
    );

    audio.follow_tabs(Some(&a), |_| true);
    assert!(
        !audio.playback(Some(&a)).unwrap().playing,
        "returning does not resume it"
    );

    audio.pending.push_back(SoundRequest {
        owner: Some(a.clone()),
        clip: None,
        preview: false,
        action: SoundAction::TogglePause,
    });
    audio.process(None, &egui::Context::default());
    assert!(audio.playback(Some(&a)).unwrap().playing, "play resumes it");
}

/// A status line belongs to the tab whose sound caused it: a failure in
/// one tab (or kit) does not show in another's player, a status no tab
/// caused shows in all of them, and closing the tab clears its own.
#[test]
fn a_status_line_shows_only_in_the_tab_that_caused_it() {
    let a = owner(1, "file:a.sound");
    let b = owner(2, "file:b.sound");
    let mut audio = AudioState::default();
    audio.pending.push_back(SoundRequest {
        owner: Some(a.clone()),
        clip: None,
        preview: false,
        action: SoundAction::Play {
            id: None,
            key: "dth1".to_owned(),
            label: "dth1".to_owned(),
            tags_root: None,
        },
    });
    audio.process(None, &egui::Context::default());
    assert_eq!(audio.status.as_deref(), Some("no source loaded"));
    assert!(audio.status_is_for(&a));
    assert!(
        !audio.status_is_for(&b),
        "another kit's tab shows a's failure"
    );

    // Another tab's volume change leaves a's status a's.
    audio.pending.push_back(SoundRequest {
        owner: Some(b.clone()),
        clip: None,
        preview: false,
        action: SoundAction::SetVolume(0.5),
    });
    audio.process(None, &egui::Context::default());
    assert!(audio.status_is_for(&a) && !audio.status_is_for(&b));

    audio.follow_tabs(None, |owner| owner != &a);
    assert!(audio.status.is_none(), "closing a left its status behind");

    audio.apply_job(AudioDone::Extracted("extracted 3 file(s)".to_owned()));
    assert!(audio.status_is_for(&a) && audio.status_is_for(&b));
}

fn inline_pcm(frames: usize) -> SoundAction {
    SoundAction::PlayInline {
        bytes: (0..frames * 2)
            .flat_map(|i| ((i % 100) as i16 * 100).to_le_bytes())
            .collect(),
        codec: InlineCodec::Pcm { big_endian: false },
        channels: 2,
        sample_rate: 1000,
        chunk_offsets: Vec::new(),
        label: "pcm".to_owned(),
    }
}

fn preview_of(owner: &SoundOwner, clip: &str, action: SoundAction) -> SoundRequest {
    SoundRequest {
        owner: Some(owner.clone()),
        clip: Some(clip.to_owned()),
        preview: true,
        action,
    }
}

/// A preview decodes its clip for the waveform without touching playback:
/// another tab's sound, the status line, a play still decoding and a
/// deferred Wwise play are all left alone. Playing the clip afterwards
/// starts no second decode.
#[test]
fn a_preview_decodes_without_disturbing_playback() {
    let a = owner(1, "file:a.sound");
    let b = owner(1, "file:b.sound");
    let mut audio = AudioState {
        voice: Some(Voice::new(
            wave(100),
            "a".to_owned(),
            Some(a.clone()),
            None,
            false,
            1.0,
        )),
        status: Some("\u{25B6} a".to_owned()),
        status_owner: Some(a.clone()),
        wwise_deferred: Some(("event".to_owned(), "e".to_owned(), Some(a.clone()), None)),
        play_request: 5,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    audio
        .pending
        .push_back(preview_of(&b, "clip", inline_pcm(1500)));
    audio.process(None, &ctx);
    assert!(matches!(
        audio.preview_for(&b).unwrap().state,
        PreviewState::Pending
    ));
    audio.wait_for_audio_jobs();
    let PreviewState::Ready(waveform) = &audio.preview_for(&b).unwrap().state else {
        panic!("the preview did not land");
    };
    assert_eq!((waveform.channels(), waveform.frames()), (2, 1500));
    assert_eq!(audio.status.as_deref(), Some("\u{25B6} a"));
    assert!(audio.status_is_for(&a) && !audio.status_is_for(&b));
    assert_eq!(audio.play_request, 5, "the preview superseded a play");
    assert!(
        audio.wwise_deferred.is_some(),
        "the preview dropped a deferred play"
    );
    assert_eq!(
        audio.voice.as_ref().and_then(|voice| voice.owner.clone()),
        Some(a.clone())
    );

    // Playing the previewed clip uses the preview: no decode starts.
    audio.pending.push_back(SoundRequest {
        owner: Some(b.clone()),
        clip: Some("clip".to_owned()),
        preview: false,
        action: inline_pcm(1500),
    });
    audio.process(None, &ctx);
    assert_eq!(
        audio.jobs.running, 0,
        "the previewed clip was decoded again"
    );

    audio.follow_tabs(None, |owner| owner != &b);
    assert!(
        audio.preview_for(&b).is_none(),
        "closing b kept its preview"
    );
}

/// A preview with nothing to show says why, in the preview rather than
/// on the status line.
#[test]
fn a_preview_that_resolves_nothing_says_why() {
    let b = owner(1, "file:b.sound");
    let mut audio = AudioState::default();
    audio.pending.push_back(preview_of(
        &b,
        "clip",
        SoundAction::Play {
            id: None,
            key: "k".to_owned(),
            label: "k".to_owned(),
            tags_root: None,
        },
    ));
    audio.process(None, &egui::Context::default());
    let PreviewState::Failed(reason) = &audio.preview_for(&b).unwrap().state else {
        panic!("no failure recorded");
    };
    assert_eq!(reason, "no source loaded");
    assert!(audio.status.is_none(), "a preview wrote the status line");
}

/// A region plays its frames and stops at its end, or wraps to its start
/// when looping.
#[test]
fn the_source_plays_a_region_and_loops_it() {
    let pcm = numbered(10);
    let (mut source, shared) = pcm_source(&pcm, false);
    shared.region_start.store(3, Ordering::Relaxed);
    shared.region_end.store(5, Ordering::Relaxed);
    shared.seek.store(3, Ordering::Relaxed);
    let played: Vec<i16> = source.by_ref().collect();
    assert_eq!(played, [30, 31, 40, 41], "the region, then stop");
    assert_eq!(shared.frame.load(Ordering::Relaxed), 5);

    let (mut source, shared) = pcm_source(&pcm, true);
    shared.region_start.store(3, Ordering::Relaxed);
    shared.region_end.store(5, Ordering::Relaxed);
    shared.seek.store(4, Ordering::Relaxed);
    let played: Vec<i16> = source.by_ref().take(6).collect();
    assert_eq!(played, [40, 41, 30, 31, 40, 41], "the region, looped");
}

#[test]
fn play_starts_at_the_playhead_inside_the_region_else_its_start() {
    assert_eq!(play_from(4, (3, 8)), 4);
    assert_eq!(play_from(1, (3, 8)), 3, "before the region");
    assert_eq!(play_from(8, (3, 8)), 3, "at its end: again from the start");
    assert_eq!(
        play_from(10, (0, 10)),
        0,
        "no region, finished: from the top"
    );
}

/// A region set on a clip before it is loaded holds when it plays; one
/// set on another clip does not. Needs an output device.
#[test]
fn a_region_set_before_loading_holds_for_its_clip() {
    let a = owner(1, "file:a.sound");
    let mut audio = AudioState::default();
    if audio.ensure_engine().is_none() {
        eprintln!("skipping: no audio output device");
        return;
    }
    audio.volume = Volume(0.0);
    audio.pending.push_back(SoundRequest {
        owner: Some(a.clone()),
        clip: Some("c".to_owned()),
        preview: false,
        action: SoundAction::SetRegion(Some((0.25, 0.5))),
    });
    audio.process(None, &egui::Context::default());
    audio.play_decoded(wave(1000), "c", Some(a.clone()), Some("c".to_owned()));
    assert_eq!(audio.voice.as_ref().unwrap().region(), Some((0.25, 0.5)));
    assert!(
        audio.playback(Some(&a)).unwrap().position >= 0.25,
        "play did not start at the region"
    );

    audio.play_decoded(wave(1000), "d", Some(a.clone()), Some("d".to_owned()));
    assert_eq!(
        audio.voice.as_ref().unwrap().region(),
        None,
        "another clip took the region"
    );
}

/// Speed steps through the sound: 2× every other frame, ½× halfway
/// between frames (not past a region's end), 0× silence with the
/// playhead held.
#[test]
fn the_source_plays_at_the_speed_set() {
    let pcm = numbered(10);
    let (source, shared) = pcm_source(&pcm, false);
    shared.speed.store(2.0f32.to_bits(), Ordering::Relaxed);
    let left: Vec<i16> = source.step_by(2).collect();
    assert_eq!(left, [0, 20, 40, 60, 80]);

    let (source, shared) = pcm_source(&pcm, false);
    shared.speed.store(0.5f32.to_bits(), Ordering::Relaxed);
    shared.region_start.store(0, Ordering::Relaxed);
    shared.region_end.store(2, Ordering::Relaxed);
    let left: Vec<i16> = source.step_by(2).collect();
    assert_eq!(
        left,
        [0, 5, 10, 10],
        "halfway between frames, held at the region's last"
    );

    let (mut source, shared) = pcm_source(&pcm, false);
    shared.speed.store(0.0f32.to_bits(), Ordering::Relaxed);
    shared.seek.store(4, Ordering::Relaxed);
    let held: Vec<i16> = source.by_ref().take(20).collect();
    assert!(held.iter().all(|sample| *sample == 0), "{held:?}");
    assert_eq!(
        shared.frame.load(Ordering::Relaxed),
        4,
        "the playhead moved at 0x"
    );
}

/// Speed is clamped to its range and reaches the sound playing now.
#[test]
fn setting_the_speed_reaches_the_sound_playing() {
    let mut audio = AudioState {
        voice: Some(Voice::new(
            wave(100),
            "a".to_owned(),
            None,
            None,
            false,
            1.0,
        )),
        ..Default::default()
    };
    assert_eq!(audio.speed(), 1.0, "speed starts at 1x");
    audio.pending.push_back(SoundAction::SetSpeed(7.0).into());
    audio.process(None, &egui::Context::default());
    assert_eq!(audio.speed(), 7.0, "a typed speed past the slider stands");
    assert_eq!(audio.voice.as_ref().unwrap().shared.speed(), 7.0);
    audio.pending.push_back(SoundAction::SetSpeed(50.0).into());
    audio.process(None, &egui::Context::default());
    assert_eq!(audio.speed(), SPEED_LIMIT);
    audio.pending.push_back(SoundAction::SetSpeed(-1.0).into());
    audio.process(None, &egui::Context::default());
    assert_eq!(audio.speed(), 0.0);
}
