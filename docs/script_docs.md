# Halo Script documentation database

`script_docs.sqlite3` is Baboon's generated, read-only scripting reference for
Halo CE, Halo 2, Halo 2 Anniversary Multiplayer, Halo 3, Halo 3: ODST,
Halo: Reach, and Halo 4.

The original `hs_doc*.txt` and `.hsc` inputs are intentionally not committed.
Their filenames and SHA-256 hashes are recorded in the `source_files` table.

Regenerate the database from an input directory containing the `h1`, `h2`,
`h2amp`, `h3`, `h3odst`, `hreach`, and `h4` folders:

```powershell
cargo run --bin build_script_docs -- C:\path\to\script_test docs\script_docs.sqlite3
```

The importer rebuilds the database from scratch in stable order. Function
descriptions and signatures come from the global documents. Up to three short,
authentic usages are extracted from the supplied HSC files. Entries with no
matching usage retain their documented signature as the syntax fallback.
