//! Editor unit and fixture tests.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// The extract-layout default-range detector matches the tool's `|default|`
    /// rule plus our synthesized placeholder for unnamed ranges.
    #[test]
    fn default_pitch_range_detection() {
        assert!(is_default_pitch_range(""));
        assert!(is_default_pitch_range("|default|"));
        assert!(is_default_pitch_range("default"));
        assert!(is_default_pitch_range("pitch range 0"));
        assert!(is_default_pitch_range("pitch range 12"));
        assert!(!is_default_pitch_range("close"));
        assert!(!is_default_pitch_range("pitch range x"));
    }

    /// End-to-end validation of the sound-player glue against real H3 files
    /// (skip-if-absent): extract permutation names exactly as `draw_sound_player`
    /// does, then resolve each against the FMOD banks and decode — the same path
    /// `AudioState::process` takes on a Play click.
    #[test]
    #[ignore]
    fn sound_player_permutations_resolve_and_decode() {
        use blam_tags::audio::{SoundBanks, decode_subsound};
        // Overridable so the same check runs against any game's tags + banks.
        let root = std::env::var("SND_TAGS_ROOT")
            .unwrap_or_else(|_| "/Users/camden/Halo/halo3_mcc/tags".to_owned());
        let rel = std::env::var("SND_TAG")
            .unwrap_or_else(|_| "sound/visual_fx/ambient_vehicle_destroyed_large.sound".to_owned());
        let tags_root = std::path::Path::new(&root);
        let tag_path = tags_root.join(&rel);
        if !tag_path.exists() {
            eprintln!("skip: no H3 tags at {}", tag_path.display());
            return;
        }
        let tag = blam_tags::TagFile::read(&tag_path).expect("read sound tag");
        let root = tag.root();
        let pitch_ranges = find_block_field(&root, "pitch range").expect("pitch ranges block");
        let mut names = Vec::new();
        for pr_index in 0..pitch_ranges.len() {
            let pitch_range = pitch_ranges.element(pr_index).unwrap();
            let permutations =
                find_block_field(&pitch_range, "permutation").expect("permutations block");
            for perm_index in 0..permutations.len() {
                let perm = permutations.element(perm_index).unwrap();
                if let Some(name) = find_full_field_name(&perm, "name")
                    .and_then(|full| perm.read_string_id(full))
                    .filter(|n| !n.is_empty())
                {
                    names.push(name);
                }
            }
        }
        assert!(!names.is_empty(), "extracted no permutation names");

        let banks = SoundBanks::open_pc(tags_root).expect("open FMOD banks");
        let mut resolved = 0usize;
        for name in &names {
            if let Some((bank_index, sub_index)) = banks.resolve(name) {
                let bank = banks.bank(bank_index);
                let sub = &bank.subsounds[sub_index];
                let data = bank.read_subsound_data(sub_index).unwrap();
                let pcm =
                    decode_subsound(&data, sub.channels, sub.frequency, sub.setup_hash).unwrap();
                assert!(pcm.frame_count() > 0, "'{name}' decoded to nothing");
                resolved += 1;
            }
        }
        eprintln!(
            "permutations: {} extracted, {} resolved+decoded",
            names.len(),
            resolved
        );
        assert!(resolved > 0, "no permutation names resolved in the bank");
    }

    /// End-to-end validation of the Halo 4 Wwise glue (skip-if-absent): read a
    /// real `.sound` tag, extract its event name exactly as `draw_sound_player`
    /// does, then resolve+decode it against the game's `.pck` banks — the same
    /// path `AudioState::process` takes on a PlayEvent click.
    #[test]
    #[ignore]
    fn h4_event_resolves_and_decodes() {
        use blam_tags::audio::WwiseBanks;
        let root = std::env::var("H4_TAGS_ROOT")
            .unwrap_or_else(|_| "/Users/camden/Halo/halo4_mcc/tags".to_owned());
        let rel = std::env::var("H4_SND_TAG")
            .unwrap_or_else(|_| "sound/ui/m30_a_60_sfx.sound".to_owned());
        let tags_root = std::path::Path::new(&root);

        let tag_path = tags_root.join(&rel);
        if !tag_path.exists() {
            eprintln!("skip: no H4 tags at {}", tag_path.display());
            return;
        }
        let tag = blam_tags::TagFile::read(&tag_path).expect("read H4 sound tag");
        let events = h4_event_names(&tag);
        assert!(!events.is_empty(), "no event names on the H4 sound tag");
        eprintln!("events: {events:?}");

        let banks = WwiseBanks::open_pc(tags_root).expect("open Wwise banks");
        let mut resolved = 0usize;
        for (_label, name) in &events {
            let pcm = banks.resolve(name).expect("resolve event");
            assert!(pcm.frame_count() > 0, "'{name}' decoded to nothing");
            eprintln!(
                "  {name} -> {}ch {}Hz {} frames",
                pcm.channels,
                pcm.sample_rate,
                pcm.frame_count()
            );
            resolved += 1;
        }
        assert!(resolved > 0);
    }

    /// Coverage audit (skip-if-absent): walk *every* `.sound` tag under a game's
    /// tags tree, compute each permutation's `fmod bank subsound id hash` exactly
    /// as the sound player does, and check it resolves in the FMOD banks. Reports
    /// id-coverage and, for id-misses, whether the legacy name lookup would have
    /// found *anything* — so a miss is attributed to a genuinely absent subsound
    /// vs. a hash/reconstruction gap. Run with:
    ///   SND_TAGS_ROOT=/Users/camden/Halo/haloreach_mcc/tags \
    ///     cargo test fmod_id_resolves_every_permutation -- --ignored --nocapture
    #[test]
    #[ignore]
    fn fmod_id_resolves_every_permutation() {
        use blam_tags::audio::{fmod_bank_subsound_id_hash, fmod_pitch_range_folder, SoundBanks};

        let root = std::env::var("SND_TAGS_ROOT")
            .unwrap_or_else(|_| "/Users/camden/Halo/haloreach_mcc/tags".to_owned());
        let tags_root = std::path::Path::new(&root);
        if !tags_root.exists() {
            eprintln!("skip: no tags at {}", tags_root.display());
            return;
        }
        let banks = SoundBanks::open_pc(tags_root).expect("open FMOD banks");

        // Recursively collect every .sound tag.
        let mut sound_tags = Vec::new();
        let mut stack = vec![tags_root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "sound") {
                    sound_tags.push(p);
                }
            }
        }
        eprintln!("scanning {} .sound tags under {}", sound_tags.len(), root);

        let (mut perms, mut by_id, mut by_name, mut id_miss_name_hit, mut absent, mut no_pr) =
            (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        let mut hash_gaps: Vec<String> = Vec::new();

        for tag_path in &sound_tags {
            let Ok(tag) = blam_tags::TagFile::read(tag_path) else {
                continue;
            };
            let tag_root = tag.root();
            let Some(pitch_ranges) = find_block_field(&tag_root, "pitch range") else {
                no_pr += 1;
                continue;
            };
            // Tag rel path (backslash, no extension) — the hash input's tag part.
            let rel = tag_path
                .strip_prefix(tags_root)
                .unwrap_or(tag_path)
                .with_extension("")
                .to_string_lossy()
                .replace('/', "\\");
            let multi_pr = pitch_ranges.len() > 1;
            for pr_index in 0..pitch_ranges.len() {
                let Some(pr) = pitch_ranges.element(pr_index) else {
                    continue;
                };
                let pr_name = find_full_field_name(&pr, "name")
                    .and_then(|full| pr.read_string_id(full))
                    .unwrap_or_default();
                let folder = fmod_pitch_range_folder(&pr_name, multi_pr);
                let Some(permutations) = find_block_field(&pr, "permutation") else {
                    continue;
                };
                for perm_index in 0..permutations.len() {
                    let Some(perm) = permutations.element(perm_index) else {
                        continue;
                    };
                    let Some(name) = find_full_field_name(&perm, "name")
                        .and_then(|full| perm.read_string_id(full))
                        .filter(|n| !n.is_empty())
                    else {
                        continue;
                    };
                    perms += 1;
                    let id = fmod_bank_subsound_id_hash(&rel, folder, &name);
                    let id_hit = banks.resolve_by_id(id).is_some();
                    let name_hit = banks.resolve(&name).is_some();
                    if id_hit {
                        by_id += 1;
                    }
                    if name_hit {
                        by_name += 1;
                    }
                    if !id_hit {
                        if name_hit {
                            id_miss_name_hit += 1;
                            if hash_gaps.len() < 30 {
                                hash_gaps.push(format!("{rel}\\{}#{perm_index} :: {name}", pr_name));
                            }
                        } else {
                            absent += 1;
                        }
                    }
                }
            }
        }

        eprintln!("permutations: {perms}");
        eprintln!(
            "  resolved by id  : {by_id} ({:.2}%)",
            100.0 * by_id as f64 / perms.max(1) as f64
        );
        eprintln!("  resolved by name: {by_name} (legacy, ambiguous)");
        eprintln!("  id-miss, name-hit (potential hash gap): {id_miss_name_hit}");
        eprintln!("  id-miss, name-miss (subsound absent from bank): {absent}");
        eprintln!("  tags without a pitch-range block (Wwise/classic): {no_pr}");
        if !hash_gaps.is_empty() {
            eprintln!("  sample hash gaps:");
            for g in &hash_gaps {
                eprintln!("    {g}");
            }
        }
    }

    /// H2 investigation (skip-if-absent): walk the classic `.sound` tag and dump
    /// every non-empty `data` field with its path + size + first bytes, to locate
    /// the inline audio and characterize its framing. Point at a tag with
    /// `SND_TAG` (rel path incl. extension), default an Opus one.
    /// `SND_TAG=sound/ui/pickup_health.sound cargo test -p baboon h2_dump_data -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn h2_dump_data_fields() {
        fn walk(
            st: &blam_tags::TagStruct<'_>,
            path: &str,
            out: &mut Vec<(String, usize, Vec<u8>)>,
            depth: usize,
        ) {
            if depth > 14 {
                return;
            }
            for field in st.fields() {
                let seg = {
                    let n = field.name();
                    if n.is_empty() {
                        "?".to_owned()
                    } else {
                        n.to_owned()
                    }
                };
                let p = format!("{path}/{seg}");
                if let Some(data) = field.as_data() {
                    if !data.is_empty() {
                        out.push((p.clone(), data.len(), data[..data.len().min(24)].to_vec()));
                    }
                } else if let Some(block) = field.as_block() {
                    for i in 0..block.len() {
                        if let Some(el) = block.element(i) {
                            walk(&el, &format!("{p}[{i}]"), out, depth + 1);
                        }
                    }
                } else if let Some(s) = field.as_struct() {
                    walk(&s, &p, out, depth + 1);
                } else if let Some(arr) = field.as_array() {
                    for (i, el) in arr.iter().enumerate() {
                        walk(&el, &format!("{p}<{i}>"), out, depth + 1);
                    }
                }
            }
        }

        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel =
            std::env::var("SND_TAG").unwrap_or_else(|_| "sound/ui/pickup_health.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no H2 tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");

        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read H2 sound tag");
        let mut out = Vec::new();
        walk(&tag.root(), "", &mut out, 0);
        eprintln!("=== {rel}: {} non-empty data field(s) ===", out.len());
        for (p, len, head) in &out {
            let hex: String = head
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ascii: String = head
                .iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            eprintln!("  {len:>8}B  {p}\n            {hex}  |{ascii}|");
        }
        // Re-walk to grab the full bytes of the largest data field and write it
        // out for framing analysis.
        let mut biggest: Option<Vec<u8>> = None;
        fn grab(st: &blam_tags::TagStruct<'_>, best: &mut Option<Vec<u8>>, depth: usize) {
            if depth > 14 {
                return;
            }
            for field in st.fields() {
                if let Some(data) = field.as_data() {
                    if best.as_ref().map_or(true, |b| data.len() > b.len()) {
                        *best = Some(data.to_vec());
                    }
                } else if let Some(block) = field.as_block() {
                    for i in 0..block.len() {
                        if let Some(el) = block.element(i) {
                            grab(&el, best, depth + 1);
                        }
                    }
                } else if let Some(s) = field.as_struct() {
                    grab(&s, best, depth + 1);
                } else if let Some(arr) = field.as_array() {
                    for el in arr.iter() {
                        grab(&el, best, depth + 1);
                    }
                }
            }
        }
        grab(&tag.root(), &mut biggest, 0);
        if let Some(bytes) = biggest {
            let stem = std::path::Path::new(&rel)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("h2");
            let out_path = format!("/tmp/h2_{stem}.bin");
            std::fs::write(&out_path, &bytes).unwrap();
            eprintln!("wrote {} ({} bytes)", out_path, bytes.len());
        }
    }

    /// Regression (skip-if-absent): clearing a classic Halo CE tag_reference to
    /// NONE — or saving a freshly-created reference — must reset the inline
    /// group + path-length words. When the reference's sub-chunk payload is
    /// emptied (`TagReferenceData::to_bytes(None)` yields no bytes), the classic
    /// encoder used to leave the stale on-disk path length in place while
    /// writing no trailing path, so re-decoding hit "unexpected EOF reading
    /// tag_reference path: need N bytes, have M" and `write_atomic` failed
    /// verification — corrupting Save As / new-tag saves. This drives the exact
    /// Baboon load→edit→save path (read_tag_at_path → apply_field_edit →
    /// write_atomic) that the field reported.
    #[test]
    #[ignore]
    fn ce_shader_model_clear_reference_saves() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let tag_path = std::path::Path::new(
            "/Users/camden/Halo/haloce_mcc/tags/characters/crewman/shaders/crewman_body.shader_model",
        );
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"soso");
        let out = std::env::temp_dir().join("baboon_ce_clear_ref.shader_model");
        let load = || {
            crate::source::read_tag_at_path(tag_path, Some("haloce_mcc"), Some(defs), group)
                .expect("read CE shader_model tag")
        };

        // Save As of the unmodified tag round-trips.
        load()
            .write_atomic(&out)
            .expect("Save As of unmodified tag");

        // Setting a reference to a new path round-trips (the pre-existing path).
        let mut edited = load();

        apply_field_edit(
            &mut edited,
            "maps/base map",
            "weapons\\smg\\bitmaps\\smg.bitmap",
        )
        .expect("set base map");
        edited.write_atomic(&out).expect("save after set");

        // Clearing a long-path reference to NONE round-trips (the regression).
        let mut cleared = load();
        apply_field_edit(&mut cleared, "maps/base map", "none").expect("clear base map");
        cleared
            .write_atomic(&out)
            .expect("save after clear-to-none must verify");

        // The cleared reference reads back as a null reference — an empty path,
        // the same shape a genuine stock null (e.g. the detail map) decodes to,
        // which Baboon renders as NONE.
        let reread = crate::source::read_tag_at_path(&out, Some("haloce_mcc"), Some(defs), group)
            .expect("reread cleared tag");
        let root = reread.root();
        let base = root
            .field_path("maps/base map")
            .and_then(|f| f.value())
            .expect("base map field present");
        match base {
            TagFieldData::TagReference(r) => {
                let path = r.group_tag_and_name.as_ref().map(|(_, p)| p.as_str());
                assert!(
                    path.is_none_or(str::is_empty),
                    "cleared ref should have no path, got {path:?}"
                );
            }
            other => panic!("expected tag reference, got {other:?}"),
        }
    }

    /// Classic Halo CE inline audio (skip-if-absent): read the classic `.sound`
    /// tag, extract the permutation's inline `samples` exactly as the player
    /// does, and decode the Ogg Vorbis — the path `AudioState` takes for a
    /// `PlayInline` action.
    #[test]
    #[ignore]
    fn ce_inline_permutation_extracts_and_decodes() {
        use blam_tags::audio::decode_ogg_vorbis;
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let tag_path = std::path::Path::new(
            "/Users/camden/Halo/haloce_mcc/tags/sound/sinomatixx_music/b40_extraction_music.sound",
        );
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(tag_path, Some("haloce_mcc"), Some(defs), group)
            .expect("read CE sound tag");
        let bytes = inline_permutation_samples(&tag, 0, 0).expect("inline samples present");
        assert!(
            bytes.starts_with(b"OggS"),
            "CE samples should be an Ogg stream"
        );
        let pcm = decode_ogg_vorbis(&bytes).expect("decode CE ogg");
        eprintln!(
            "CE inline: {} bytes -> {} frames {}ch {}Hz",
            bytes.len(),
            pcm.frame_count(),
            pcm.channels,
            pcm.sample_rate
        );
        assert!(pcm.frame_count() > 0);
    }

    /// Classic Halo CE Xbox-ADPCM weapon sound (skip-if-absent). CE `.sound`
    /// tags aren't always Ogg — weapon/effect sounds are frequently
    /// `format = xbox adpcm`, which has no `OggS` header. Regression for the
    /// `ogg header: NoCapturePatternFound` failure: the codec must come from the
    /// tag's `format` field, and the row must decode via the ADPCM path.
    #[test]
    #[ignore]
    fn ce_inline_xbox_adpcm_extracts_and_decodes() {
        use super::audio::InlineCodec;
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let tag_path = std::path::Path::new(
            "/Users/camden/Halo/haloce_mcc/tags/sound/sfx/weapons/sniper rifle/fire.sound",
        );
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");

        let tag = crate::source::read_tag_at_path(tag_path, Some("haloce_mcc"), Some(defs), group)
            .expect("read CE sound tag");

        // The tag reports Xbox-ADPCM, mono, 22050 Hz — and carries no Ogg stream.
        let (codec, channels, sample_rate) = ce_codec_params(&tag);
        assert!(
            matches!(codec, InlineCodec::XboxAdpcm),
            "sniper fire.sound is xbox adpcm, got {codec:?}"
        );
        assert_eq!((channels, sample_rate), (1, 22_050));

        // The player's row must inherit that codec (not assume Ogg).
        let rows = sound_permutation_rows(&tag);
        assert!(matches!(
            rows.first().map(|r| &r.kind),
            Some(RowKind::InlineCe {
                codec: InlineCodec::XboxAdpcm,
                ..
            })
        ));

        let bytes = inline_permutation_samples(&tag, 0, 0).expect("inline samples present");
        assert!(!bytes.starts_with(b"OggS"), "adpcm stream, not Ogg");
        let pcm = super::audio::decode_inline(codec, &bytes, channels, sample_rate)
            .expect("decode CE xbox adpcm");
        eprintln!(
            "CE xbox-adpcm: {} bytes -> {} frames {}ch {}Hz",
            bytes.len(),
            pcm.frame_count(),
            pcm.channels,
            pcm.sample_rate
        );
        assert!(pcm.frame_count() > 0);
    }

    /// End-to-end extraction (skip-if-absent): read a real CE `.sound`, build
    /// the same rows the player builds, and run the actual
    /// `AudioState::run_extract` for both WAV (decoded) and raw `.ogg`
    /// (passthrough), validating each output.
    #[test]
    #[ignore]
    fn ce_extract_writes_wav_and_raw_ogg() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let tag_path = std::path::Path::new(
            "/Users/camden/Halo/haloce_mcc/tags/sound/sinomatixx_music/b40_extraction_music.sound",
        );
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(tag_path, Some("haloce_mcc"), Some(defs), group)
            .expect("read CE sound tag");
        let rows = sound_permutation_rows(&tag);
        assert!(!rows.is_empty(), "CE tag should have permutations");

        // Decoded WAV.
        let wav_dir = std::env::temp_dir().join("baboon_ce_extract_wav");
        let _ = std::fs::remove_dir_all(&wav_dir);

        let items = build_extract_items(&tag, &rows, None, &wav_dir, false, None);
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(ExtractRequest {
            items,
            tags_root: None,
            label: "ce".to_owned(),
        });
        let wav = std::fs::read(wav_dir.join(format!("{}.wav", sanitize_component(&rows[0].name))))
            .expect("wav written");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert!(wav.len() > 44, "wav should carry samples");

        // Raw .ogg passthrough should be byte-identical to the inline samples.
        let ogg_dir = std::env::temp_dir().join("baboon_ce_extract_ogg");
        let _ = std::fs::remove_dir_all(&ogg_dir);
        let items = build_extract_items(&tag, &rows, None, &ogg_dir, true, None);
        audio.run_extract(ExtractRequest {
            items,
            tags_root: None,
            label: "ce".to_owned(),
        });
        let ogg = std::fs::read(ogg_dir.join(format!("{}.ogg", sanitize_component(&rows[0].name))))
            .expect("ogg written");
        assert!(ogg.starts_with(b"OggS"), "raw passthrough should be an Ogg");
        let inline =
            inline_permutation_samples(&tag, rows[0].pr_index, rows[0].perm_index).unwrap();
        assert_eq!(ogg, inline, "raw passthrough must be verbatim tag bytes");

        let _ = std::fs::remove_dir_all(&wav_dir);

        let _ = std::fs::remove_dir_all(&ogg_dir);
    }

    /// Whole-tag H2 extraction end-to-end (skip-if-absent): build the same rows
    /// the player builds and run the real `AudioState::run_extract`, validating a
    /// decoded WAV lands for the inline (Opus/ADPCM/PCM) path. `SND_TAG` overrides.
    #[test]
    #[ignore]
    fn h2_extract_writes_wav() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel =
            std::env::var("SND_TAG").unwrap_or_else(|_| "sound/ui/pickup_health.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no H2 tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read H2 sound tag");
        let rows = sound_permutation_rows(&tag);
        assert!(!rows.is_empty(), "H2 tag should have permutations");
        let h2_params = Some(h2_codec_params(&tag));
        let dir = std::env::temp_dir().join("baboon_h2_extract");
        let _ = std::fs::remove_dir_all(&dir);
        let items = build_extract_items(&tag, &rows, h2_params, &dir, false, None);
        let count = items.len();
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(ExtractRequest {
            items,
            tags_root: None,
            label: "h2".to_owned(),
        });
        // At least one WAV should have been written with a valid RIFF header.
        let mut found = 0usize;
        for entry in walkdir(&dir) {
            let bytes = std::fs::read(&entry).unwrap();
            assert_eq!(&bytes[0..4], b"RIFF");
            assert_eq!(&bytes[8..12], b"WAVE");
            found += 1;
        }
        assert!(found > 0 && found <= count, "wrote {found}/{count} wavs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Whole-tag H3/Reach bank extraction end-to-end (skip-if-absent): run the
    /// real `run_extract` Bank path (resolve subsound → decode → WAV) for every
    /// permutation. `SND_TAGS_ROOT`/`SND_TAG` override for Reach/ODST.
    #[test]
    #[ignore]
    fn bank_extract_writes_wav() {
        let root = std::env::var("SND_TAGS_ROOT")
            .unwrap_or_else(|_| "/Users/camden/Halo/halo3_mcc/tags".to_owned());
        let rel = std::env::var("SND_TAG")
            .unwrap_or_else(|_| "sound/visual_fx/ambient_vehicle_destroyed_large.sound".to_owned());
        let tags_root = std::path::Path::new(&root);
        let tag_path = tags_root.join(&rel);
        if !tag_path.exists() {
            eprintln!("skip: no bank tags at {}", tag_path.display());
            return;
        }
        let tag = blam_tags::TagFile::read(&tag_path).expect("read sound tag");
        let rows = sound_permutation_rows(&tag);
        assert!(!rows.is_empty());
        let dir = std::env::temp_dir().join("baboon_bank_extract");
        let _ = std::fs::remove_dir_all(&dir);
        let items = build_extract_items(&tag, &rows, None, &dir, false, None);
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(ExtractRequest {
            items,
            tags_root: Some(tags_root.to_path_buf()),
            label: "bank".to_owned(),
        });
        let mut found = 0usize;
        for entry in walkdir(&dir) {
            let bytes = std::fs::read(&entry).unwrap();
            assert_eq!(&bytes[0..4], b"RIFF");
            found += 1;
        }
        assert!(found > 0, "wrote no bank WAVs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DIAGNOSTIC (skip-if-absent): dump the full structure of a real H2 sound
    /// tag — every block/element and each data field's size — plus h2_blobs and
    /// decoded durations, to find why long sounds show short. Point via SND_TAG.
    #[test]
    #[ignore]
    fn h2_dump_structure() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");

        let rel = std::env::var("SND_TAG").unwrap_or_else(|_| "sound/loop.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read tag");
        let (codec, channels, rate) = h2_codec_params(&tag);
        eprintln!("codec={codec:?} channels={channels} rate={rate}");
        let root = tag.root();

        // Permutation names + count.
        if let Some(prs) = find_block_field(&root, "pitch range") {
            for p in 0..prs.len() {
                let pr = prs.element(p).unwrap();
                if let Some(perms) = find_block_field(&pr, "permutation") {
                    eprintln!("pitch range {p}: {} permutations", perms.len());
                    for pm in 0..perms.len() {
                        let perm = perms.element(pm).unwrap();
                        let name = find_full_field_name(&perm, "name")
                            .and_then(|f| perm.read_string_id(f))
                            .unwrap_or_default();
                        // dump any int/enum fields that look like chunk/sample counts
                        let fields: Vec<String> = perm
                            .field_names()
                            .filter(|n| {
                                let c = clean_field_name(n).to_ascii_lowercase();
                                c.contains("sample")
                                    || c.contains("count")
                                    || c.contains("chunk")
                                    || c.contains("first")
                                    || c.contains("index")
                            })
                            .map(|n| {
                                let v = perm
                                    .read_int_any(n)
                                    .map(|x| x.to_string())
                                    .unwrap_or_default();
                                format!("{}={v}", clean_field_name(n))
                            })
                            .collect();
                        eprintln!("  perm {pm} '{name}': {}", fields.join(" "));
                    }
                }
            }
        }

        // Walk extra-info → language perm info → raw info block, dumping data sizes
        // AND any nested blocks (chunked samples live in a nested block).
        for field in root.fields() {
            let Some(block) = field.as_block() else {
                continue;
            };
            for i in 0..block.len() {
                let el = block.element(i).unwrap();
                let Some(lpi) = find_block_field(&el, "language permutation info") else {
                    continue;
                };
                eprintln!(
                    "extra-info[{i}] '{}': lpi={}",
                    clean_field_name(field.name()),
                    lpi.len()
                );
                for j in 0..lpi.len() {
                    let lel = lpi.element(j).unwrap();
                    let Some(raw) = find_block_field(&lel, "raw info block") else {
                        continue;
                    };
                    eprintln!("  lpi[{j}]: raw info block={}", raw.len());
                    for k in 0..raw.len() {
                        let rel = raw.element(k).unwrap();
                        let mut parts = Vec::new();
                        for f in rel.fields() {
                            if let Some(d) = f.as_data() {
                                parts.push(format!(
                                    "data'{}'={}B",
                                    clean_field_name(f.name()),
                                    d.len()
                                ));
                            } else if let Some(b) = f.as_block() {
                                parts.push(format!(
                                    "block'{}'x{}",
                                    clean_field_name(f.name()),
                                    b.len()
                                ));
                                // dump nested block element int values (chunk offsets/sizes)
                                for m in 0..b.len().min(20) {
                                    if let Some(be) = b.element(m) {
                                        let ints: Vec<String> = be
                                            .field_names()
                                            .filter_map(|n| {
                                                be.read_int_any(n).map(|v| v.to_string())
                                            })
                                            .collect();
                                        let datas: Vec<String> = be
                                            .fields()
                                            .filter_map(|bf| {
                                                bf.as_data().map(|d| format!("data={}B", d.len()))
                                            })
                                            .collect();
                                        if !ints.is_empty() || !datas.is_empty() {
                                            parts.push(format!(
                                                "[{m}:{} {}]",
                                                ints.join(","),
                                                datas.join(",")
                                            ));
                                        }
                                    }
                                }
                            } else {
                                let c = clean_field_name(f.name()).to_ascii_lowercase();
                                if c.contains("sample")
                                    || c.contains("count")
                                    || c.contains("size")
                                    || c.contains("compression")
                                    || c.contains("index")
                                {
                                    if let Some(v) = rel.read_int_any(f.name()) {
                                        parts.push(format!("{}={v}", clean_field_name(f.name())));
                                    }
                                }
                            }
                        }
                        eprintln!("    raw[{k}]: {}", parts.join(" "));
                    }
                }
            }
        }

        let (count, _) = h2_blobs(&tag, Some(0));
        eprintln!("h2_blobs count={count}");
        for idx in 0..count {
            let (_, blob) = h2_blobs(&tag, Some(idx));
            let offs = h2_blob_chunk_offsets(&tag, idx);
            if let Some(b) = blob {
                let old = super::audio::decode_inline(codec, &b, channels, rate)
                    .map(|p| p.duration_secs())
                    .unwrap_or(0.0);
                match super::audio::decode_inline_chunked(codec, &b, &offs, channels, rate) {
                    Ok(pcm) => eprintln!(
                        "blob{idx}: {}B, {} chunks -> {:.2}s (was {:.2}s single-chunk)",
                        b.len(),
                        offs.len().max(1),
                        pcm.duration_secs(),
                        old
                    ),
                    Err(e) => eprintln!("blob{idx}: {}B decode err: {e}", b.len()),
                }
            }
        }

        // Verify: decode blob0's chunks independently by slicing at chunk offsets.
        let (_, blob0) = h2_blobs(&tag, Some(0));
        if let Some(b) = blob0 {
            let offs = [0usize, 19234, 38487, b.len()];
            let mut sum = 0.0f32;
            for w in offs.windows(2) {
                if w[1] <= b.len() && w[0] < w[1] {
                    match super::audio::decode_inline(codec, &b[w[0]..w[1]], channels, rate) {
                        Ok(p) => {
                            sum += p.duration_secs();
                            eprintln!(
                                "  chunk [{}..{}] {}B -> {} frames = {:.2}s",
                                w[0],
                                w[1],
                                w[1] - w[0],
                                p.frame_count(),
                                p.duration_secs()
                            );
                        }
                        Err(e) => eprintln!("  chunk [{}..{}] err: {e}", w[0], w[1]),
                    }
                }
            }
            eprintln!("  blob0 all chunks summed = {sum:.2}s");
        }
    }

    /// Regression (skip-if-absent): H2 splits inline audio into per-chunk streams
    /// (`sound_permutation_chunk_block`); the decoder must concatenate all chunks,
    /// not stop at the first. For any multi-chunk blob, the chunked decode must be
    /// longer than the single-stream decode. Default tag = the custom `loop.sound`.
    #[test]
    #[ignore]
    fn h2_chunked_decode_is_full_length() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel = std::env::var("SND_TAG").unwrap_or_else(|_| "sound/loop.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read tag");
        let (codec, channels, rate) = h2_codec_params(&tag);
        let (count, _) = h2_blobs(&tag, Some(0));
        let mut checked_multichunk = false;
        for idx in 0..count {
            let (_, blob) = h2_blobs(&tag, Some(idx));
            let offs = h2_blob_chunk_offsets(&tag, idx);
            let Some(bytes) = blob else { continue };
            if offs.len() <= 1 {
                continue; // single-chunk: chunked == single, nothing to prove
            }
            checked_multichunk = true;
            let single = super::audio::decode_inline(codec, &bytes, channels, rate)
                .map(|p| p.frame_count())
                .unwrap_or(0);
            let full = super::audio::decode_inline_chunked(codec, &bytes, &offs, channels, rate)
                .expect("chunked decode")
                .frame_count();
            assert!(
                full > single,
                "blob{idx}: chunked {full} !> single {single} ({} chunks)",
                offs.len()
            );
        }
        assert!(
            checked_multichunk,
            "expected at least one multi-chunk blob to test"
        );
    }

    /// Per-language bank plumbing (skip-if-absent): the FMOD languages are
    /// discovered from the `.fsb` names, and opening a specific language + the
    /// shared sfx bank still resolves an SFX permutation (language bank first,
    /// sfx fallback). Proves `open_pc_language` doesn't break default resolution.
    #[test]
    #[ignore]
    fn fmod_language_selection_resolves() {
        use blam_tags::audio::SoundBanks;
        let tags_root = std::path::Path::new("/Users/camden/Halo/halo3_mcc/tags");
        if !tags_root.join("../fmod/pc/sfx.fsb").exists() {
            eprintln!("skip: no H3 fmod banks");
            return;
        }
        let langs = SoundBanks::available_languages(tags_root);

        eprintln!("H3 languages: {langs:?}");
        assert!(
            langs.iter().any(|l| l == "french"),
            "expected localized .fsb languages, got {langs:?}"
        );
        assert!(!langs.iter().any(|l| l == "sfx"), "sfx must be excluded");
        // Open a specific language + sfx; an SFX permutation still resolves.
        let banks =
            SoundBanks::open_pc_language(tags_root, Some("french")).expect("open french+sfx");
        let tag = blam_tags::TagFile::read(
            &tags_root.join("sound/visual_fx/ambient_vehicle_destroyed_large.sound"),
        )
        .expect("read sfx sound tag");
        let rows = sound_permutation_rows(&tag);
        let resolved = rows
            .iter()
            .filter(|r| banks.resolve(&r.name).is_some())
            .count();
        assert!(resolved > 0, "no permutations resolved in french+sfx banks");
    }

    /// Recursively collect files under `dir` (small test helper).
    fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walkdir(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    /// Classic Halo 2 inline audio (skip-if-absent): read the tag, extract the
    /// first inline audio blob + codec params exactly as the player does, and
    /// decode via the matching codec (Opus or Xbox-ADPCM). Point at a tag with
    /// `SND_TAG`; default an Opus one.
    #[test]
    #[ignore]
    fn h2_inline_extracts_and_decodes() {
        use super::audio::InlineCodec;
        use blam_tags::audio::{decode_opus, decode_xbox_adpcm};
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel =
            std::env::var("SND_TAG").unwrap_or_else(|_| "sound/ui/pickup_health.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no H2 tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read H2 sound tag");
        let (count, blob) = h2_blobs(&tag, Some(0));
        assert!(count > 0, "no H2 inline audio blobs found");
        let bytes = blob.expect("blob 0");
        let (codec, channels, sample_rate) = h2_codec_params(&tag);
        let (codec_name, pcm) = match codec {
            InlineCodec::Opus => ("opus", decode_opus(&bytes, channels)),
            InlineCodec::XboxAdpcm => (
                "xbox-adpcm",
                decode_xbox_adpcm(&bytes, channels, sample_rate),
            ),
            InlineCodec::Pcm { big_endian } => (
                "pcm",
                blam_tags::audio::decode_pcm(&bytes, channels, sample_rate, big_endian),
            ),
            InlineCodec::OggVorbis => unreachable!("H2 is opus/adpcm/pcm"),
        };
        let pcm = pcm.expect("decode H2 inline");
        eprintln!(
            "H2 {rel}: {count} blob(s) codec={codec_name} ch={channels} {sample_rate}Hz \
             -> {} frames ({:.2}s)",
            pcm.frame_count(),
            pcm.duration_secs()
        );
        assert!(pcm.frame_count() > 0);
    }

    #[test]
    fn sound_classes_summary_reads_modern_and_classic_layouts() {
        // Modern (Reach): scalar distances nested under "distance parameters".
        let mut tag = TagFile::new("definitions/haloreach_mcc/sound_classes.json").unwrap();
        add_block_element(&mut tag, "sound classes").unwrap();
        let classes = tag
            .root()
            .field("sound classes")
            .and_then(|field| field.as_block())
            .unwrap();
        let element = classes.element(0).unwrap();
        assert!(
            element.descend("distance parameters").is_some(),
            "Reach nests distances under `distance parameters`"
        );
        assert_ne!(
            sound_class_distance_row(&element).near,
            "—",
            "Reach `minimum distance` field name should resolve"
        );

        // Classic (H3): `distance bounds` real_bounds directly on the entry.
        let mut tag = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        add_block_element(&mut tag, "sound classes").unwrap();
        let classes = tag
            .root()
            .field("sound classes")
            .and_then(|field| field.as_block())
            .unwrap();
        let element = classes.element(0).unwrap();
        assert!(
            element.descend("distance parameters").is_none(),
            "H3 has no `distance parameters` struct"
        );

        assert!(
            element.field("distance bounds").is_some(),
            "H3 keeps `distance bounds` directly on the entry"
        );
        assert_ne!(sound_class_distance_row(&element).near, "—");
    }

    #[test]
    fn material_effects_summary_walks_effects_and_materials_cross_game() {
        // CE: effect → `materials` block with `effect` + `sound` tag references.
        let mut tag = TagFile::new("definitions/haloce_mcc/material_effects.json").unwrap();
        add_block_element(&mut tag, "effects").unwrap();
        add_block_element(&mut tag, "effects[0]/materials").unwrap();
        let materials = tag
            .root()
            .field_path("effects[0]/materials")
            .and_then(|field| field.as_block())
            .unwrap();
        let material = materials.element(0).unwrap();
        assert!(find_full_field_name(&material, "effect").is_some());
        assert!(find_full_field_name(&material, "sound").is_some());

        // Modern (H3): effect → `sounds` block; materials use a `tag (effect or
        // sound)` reference and a `material name` string_id.
        let mut tag = TagFile::new("definitions/halo3_mcc/material_effects.json").unwrap();
        add_block_element(&mut tag, "effects").unwrap();
        let effects = tag
            .root()
            .field("effects")
            .and_then(|field| field.as_block())
            .unwrap();
        let effect = effects.element(0).unwrap();
        let labels: Vec<String> = block_fields(&effect)
            .into_iter()
            .map(|(label, _)| label.to_ascii_lowercase())
            .collect();
        assert!(
            labels.iter().any(|label| label.contains("sound")),
            "modern effect has a `sounds` material sub-block"
        );
        assert!(
            labels.iter().any(|label| label.contains("old")),
            "modern effect still declares the deprecated `old materials` block"
        );
        add_block_element(&mut tag, "effects[0]/sounds").unwrap();
        let sounds = tag
            .root()
            .field_path("effects[0]/sounds")
            .and_then(|field| field.as_block())
            .unwrap();
        let material = sounds.element(0).unwrap();
        assert!(
            material
                .field_names()
                .any(|name| name.contains("tag (effect or sound)")),
            "modern material carries a `tag (effect or sound)` reference"
        );
        assert!(
            find_field_name_containing(&material, "material name").is_some(),
            "modern material carries a `material name` field"
        );
    }

    #[test]
    fn dialogue_summary_detects_direct_vs_nested_and_classic() {
        // Classic CE: no vocalizations block (flat per-context fields).
        let tag = TagFile::new("definitions/haloce_mcc/dialogue.json").unwrap();
        assert!(
            find_block_field(&tag.root(), "vocali").is_none(),
            "CE has no vocalizations block"
        );

        // H3/ODST: `sound` reference directly on the vocalization.
        let mut tag = TagFile::new("definitions/halo3_mcc/dialogue.json").unwrap();
        add_block_element(&mut tag, "vocalizations").unwrap();
        let vocals = tag
            .root()
            .field("vocalizations")
            .and_then(|field| field.as_block())
            .unwrap();
        let vocal = vocals.element(0).unwrap();
        assert!(
            find_full_field_name(&vocal, "sound").is_some(),
            "H3 keeps `sound` directly on the vocalization"
        );
        assert!(
            find_block_field(&vocal, "stimul").is_none(),
            "H3 has no stimuli sub-block"
        );

        // Reach/H4/H2A: `sound` nested under a per-vocalization `stimuli` block.
        let mut tag = TagFile::new("definitions/haloreach_mcc/dialogue.json").unwrap();
        add_block_element(&mut tag, "vocalizations").unwrap();
        let vocals = tag
            .root()
            .field("vocalizations")
            .and_then(|field| field.as_block())
            .unwrap();
        let vocal = vocals.element(0).unwrap();
        assert!(
            find_block_field(&vocal, "stimul").is_some(),
            "Reach nests sounds under a `stimuli` block"
        );
        assert!(
            find_full_field_name(&vocal, "sound").is_none(),
            "Reach vocalization has no direct `sound` field"
        );
    }

    #[test]
    fn field_meta_uses_foundation_marker_semantics() {
        // Foundation semantics (adopted from `TagFieldNameInfo`): `*` = read-only,
        // `!` = hidden/expert-only (Baboon's `advanced` gate). Presence-tested, so
        // order and combination don't matter — the old `ends_with` parser dropped
        // `*` on `angle*!`.
        let ro = field_display_meta("a position*");
        assert!(ro.read_only && !ro.advanced, "'*' => read-only");

        let hidden = field_display_meta("activity!");
        assert!(hidden.advanced && !hidden.read_only, "'!' => hidden/advanced");

        let both = field_display_meta("angle*!");
        assert!(both.read_only, "combined: '*' still read-only");
        assert!(both.advanced, "combined: '!' still hidden");
        assert_eq!(both.label, "angle");

        let both_rev = field_display_meta("aabb center!*");
        assert!(both_rev.read_only && both_rev.advanced, "order-independent");
        assert_eq!(both_rev.label, "aabb center");
    }

    #[test]
    fn field_meta_separates_range_from_unit_and_suffix_shows_both() {
        // Range in the unit slot: unit is empty, range captured; suffix shows
        // the type (no unit) followed by the range.
        let m = field_display_meta("acceleration scale:[0,+inf]#marine 1.0, grunt 1.4");
        assert_eq!(m.label, "acceleration scale");
        assert_eq!(m.unit, None);
        assert_eq!(m.range.as_deref(), Some("[0,+inf]"));
        assert_eq!(m.help.as_deref(), Some("marine 1.0, grunt 1.4"));
        assert_eq!(field_suffix(&m, "real"), "real [0,+inf]");

        // Range bare in the name (no colon): pulled out of the label.
        let m = field_display_meta("max sounds per tag [1,16]#max sounds");

        assert_eq!(m.label, "max sounds per tag");
        assert_eq!(m.range.as_deref(), Some("[1,16]"));
        assert_eq!(field_suffix(&m, "long_integer"), "long integer [1,16]");

        // Real unit, no range: unit wins over the type, no range appended.
        let m = field_display_meta("preemption time:ms#replaces after this many ms");
        assert_eq!(m.unit.as_deref(), Some("ms"));
        assert_eq!(m.range, None);
        assert_eq!(field_suffix(&m, "short_integer"), "ms");

        // Unit AND range together: unit first, then range.
        let m = field_display_meta("auto-exposure delay:[0.1-1]seconds#how long");
        assert_eq!(m.unit.as_deref(), Some("seconds"));
        assert_eq!(m.range.as_deref(), Some("[0.1-1]"));
        assert_eq!(field_suffix(&m, "real"), "seconds [0.1-1]");
    }

    #[test]
    fn new_tags_strip_doc_strings_and_explanations_on_write() {
        // The engine strips explanation fields + cleans field names when building
        // a layout from JSON, so a freshly-created tag's embedded blay matches
        // shipped tags — no `#help`/`:units` text, no explanation bodies.
        let tag = TagFile::new("definitions/haloreach_mcc/sound_classes.json").unwrap();
        let bytes = tag.write_to_bytes().unwrap();
        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(
            !contains(b"attenuating"),
            "must not embed explanation/help text"
        );
        assert!(
            !contains(b"world units"),
            "must not embed `:units` annotations"
        );
        // And it must still round-trip cleanly.
        TagFile::read_from_bytes(&bytes).expect("stripped tag must parse");
    }

    #[test]
    fn block_to_tsv_exports_header_and_one_row_per_element() {
        let mut tag = TagFile::new("definitions/halo2_mcc/model.json").unwrap();
        let mut dirty = false;
        for name in ["alpha", "beta"] {
            apply_model_variant_ops(
                &mut tag,
                vec![ModelVariantOp::Create {
                    name: name.to_owned(),
                    regions: Vec::new(),
                }],
                &mut dirty,
            );
        }
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        let tsv = super::block_to_tsv(&variants, &TagNameIndex::default());

        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 element rows");
        assert!(
            lines[0].split('\t').any(|col| col == "name"),
            "header should include the `name` column"
        );
        assert!(tsv.contains("alpha") && tsv.contains("beta"));
    }

    #[test]
    fn model_variant_ops_create_update_and_drop_regions() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/model.json")).unwrap();
        let mut dirty = false;

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Create {
                name: "test".to_owned(),
                regions: vec![ModelVariantRegionChoice {
                    region_name: "body".to_owned(),
                    permutation_name: "default".to_owned(),
                }],
            }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Created model variant 'test'"));
        assert!(dirty);
        assert_variant(&tag, 0, "test", "body", "default");

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Update {
                variant_index: 0,
                regions: vec![ModelVariantRegionChoice {
                    region_name: "head".to_owned(),
                    permutation_name: "damaged".to_owned(),
                }],
            }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Updated model variant 0"));
        assert_variant(&tag, 0, "test", "head", "damaged");

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Drop { variant_index: 0 }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Deleted model variant 0"));
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(variants.len(), 0);
    }

    #[test]
    fn h2_render_model_marker_translation_and_rotation_are_editable() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/render_model.json")).unwrap();
        tag.container = test_halo2_render_model_container();
        {
            let mut root = tag.root_mut();

            let mut field = root.field_path_mut("marker groups").unwrap();
            let mut marker_groups = field.as_block_mut().unwrap();
            marker_groups.add_element();
        }
        {
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut("marker groups[0]/markers").unwrap();
            let mut markers = field.as_block_mut().unwrap();
            markers.add_element();
        }

        let mut dirty = false;
        let status = apply_pending_edits(
            &mut tag,
            vec![
                PendingFieldEdit {
                    path: "marker groups[0]/markers[0]/translation".to_owned(),
                    input: "-0.27, 0, 0.73".to_owned(),
                },
                PendingFieldEdit {
                    path: "marker groups[0]/markers[0]/rotation".to_owned(),
                    input: "-0.38, 0, -0.92, 0".to_owned(),
                },
            ],
            &mut dirty,
        );

        assert_eq!(
            status.as_deref(),
            Some("Edited marker groups[0]/markers[0]/rotation")
        );
        assert!(dirty);
        let root = tag.root();
        let translation = root
            .field_path("marker groups[0]/markers[0]/translation")
            .unwrap()
            .value()
            .unwrap();
        let TagFieldData::RealPoint3d(translation) = translation else {
            panic!("translation should be a real point 3d");
        };
        assert!((translation.x + 0.27).abs() < 0.0001);
        assert!((translation.y - 0.0).abs() < 0.0001);
        assert!((translation.z - 0.73).abs() < 0.0001);

        let rotation = root
            .field_path("marker groups[0]/markers[0]/rotation")
            .unwrap()
            .value()
            .unwrap();
        let TagFieldData::RealQuaternion(rotation) = rotation else {
            panic!("rotation should be a real quaternion");
        };
        assert!((rotation.i + 0.38).abs() < 0.0001);
        assert!((rotation.j - 0.0).abs() < 0.0001);
        assert!((rotation.k + 0.92).abs() < 0.0001);
        assert!((rotation.w - 0.0).abs() < 0.0001);
        assert_h2_render_model_write_atomic_verifies(&tag);
    }

    fn test_halo2_render_model_container() -> blam_tags::file::TagContainer {
        let mut header = vec![0; 64];
        header[36..40].copy_from_slice(b"edom");
        header[56..58].copy_from_slice(&0u16.to_le_bytes());
        header[60..64].copy_from_slice(b"!MLB");
        blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header,
        }
    }

    fn assert_h2_render_model_write_atomic_verifies(tag: &TagFile) {
        let mut path = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "baboon_h2_render_model_marker_{}_{}.render_model",
            std::process::id(),
            stamp
        ));
        let _ = std::fs::remove_file(&path);
        tag.write_atomic(&path).unwrap_or_else(|error| {
            panic!(
                "write_atomic verification failed for {}: {error}",
                path.display()
            )
        });
        let _ = std::fs::remove_file(&path);
    }

    fn assert_variant(
        tag: &TagFile,
        variant_index: usize,
        variant_name: &str,
        region_name: &str,
        permutation_name: &str,
    ) {
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        let variant = variants.element(variant_index).unwrap();
        assert_eq!(
            variant.read_string_id("name").as_deref(),
            Some(variant_name)
        );
        let regions = variant
            .field("regions")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(regions.len(), 1);
        let region = regions.element(0).unwrap();
        assert_eq!(
            region.read_string_id("region name").as_deref(),
            Some(region_name)
        );
        let permutations = region
            .field("permutations")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(permutations.len(), 1);
        let permutation = permutations.element(0).unwrap();
        assert_eq!(
            permutation.read_string_id("permutation name").as_deref(),
            Some(permutation_name)
        );
    }
}

#[cfg(test)]
mod tag_diff_tests {
    use super::*;

    #[test]
    fn diff_detects_value_and_block_count_changes() {
        let names = TagNameIndex::default();
        let a = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        let mut b = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        // Two freshly-created identical tags must report no differences.
        let (diffs, truncated) = diff_tags(&a, &b, &names, 5000);
        assert!(diffs.is_empty(), "identical tags should have no diffs");
        assert!(!truncated);
        // Adding a block element to one shows up as an element-count difference.
        add_block_element(&mut b, "sound classes").unwrap();
        let (diffs, _) = diff_tags(&a, &b, &names, 5000);
        assert!(
            diffs.iter().any(|d| d.path.contains("sound classes")),
            "block element-count difference should be reported"
        );
    }
}
