# Saved-file compatibility fixtures

Synthetic samples of every file format Baboon saves and reads back. Nothing
here came from a real user or install: paths, ids, keys and blobs are made up,
in the shapes the readers expect. Overlay and history `bytes` blobs are
placeholders, not parseable tags.

The tests feed each sample through the real reader:

- `src/app/tests/compat_fixtures.rs`: sessions, preferences, projects,
  Campaign Evolved identities, the duplicate ledger, keyword sidecars
- `src/app/tests/compat_keys.rs`: entry key spellings, index root keys, the
  index database, legacy JSON indexes
- `src/window_state.rs` (`compat_window_state_sample`) and `src/app/chimp/session.rs`
  (`compat_chimp_recovery_sample`): window state, Chimp recovery

Run them with `cargo test compat_`. They need no environment variables.

## Regenerating

```
python3 testdata/compat/gen_samples.py
```

The script rewrites `samples/` in place (SQLite files are written with a `.sql`
dump beside them, for review). It is deterministic: running it on an unchanged
script leaves `git status` clean. When a format changes, add a new sample for
the new shape and keep the old ones, since every file an older build wrote must
still load.

## What is where

| Sample | Reader |
| --- | --- |
| `prefs/` | `prefs.json`: `src/app/prefs.rs` (`prefs_from_value`, `prefs_to_value`) |
| `last_session/` | `last_session.json` v1 to v6: `parse_last_session`. `v2_kits_8f30d04.json` is a shape one commit wrote and today refuses; `v99_unknown_version.json` is refused |
| `window_state/` | `window-state.json`: `src/window_state.rs`, `schema_version` must be 1 |
| `index/` | `indexes.sqlite3`: `src/source/index.rs` |
| `legacy_index/` | `{game}_index.json`, `{game}_reverse_dependencies.json`: migration reads in `src/source/index.rs` |
| `keywords/` | `{game}_keywords.json`: `src/app/keywords.rs`; `*.corrupt.json` is cut short mid-write |
| `ledger/` | `campaign_duplicates.json`: `src/app/controller/created_tags.rs` |
| `project/` | `.baboon` projects and recovery files: `src/app/project.rs`; `rejected.*` must be refused |
| `chimp/` | Chimp recovery folder and manifest: `src/app/chimp/session.rs` (`compat_chimp_recovery_sample`) |
| `backups_*.manifest.json` | container backup manifest, written but never read back |
| `tag_keys.json` | one entry key of every kind and spelling |

Behaviour the samples pin that changed on purpose:

- A profile or alias for a game this build does not support is kept and written
  back unchanged (`prefs.unknown_game.json`).
- A ledger row with an unknown `origin` loads as unrecognized and is kept; a row
  of an unknown shape is kept raw; a ledger that cannot be parsed is left as it
  is (`ledger/`).
- An unparseable keyword sidecar is moved aside to `<name>.unreadable` before a
  new one is written.
- A Campaign Evolved container tag with a dot in its name keeps it in its
  identity; the identity an older build wrote still resolves when unique
  (`user_project.dotted_identities.baboon`).
