# RenameKit Roadmap

## v0.1.0 (current)

The safe batch rename engine behind a C ABI: one JSON spec (folder + explicit file list or simple `*`/`?` glob + ordered rules), a preview plan with per-entry status (ok / unchanged / collision_existing / collision_in_plan / invalid / duplicate_source), Windows name validation, chain-aware collision detection, all-or-nothing apply (or `allow_partial`), two-phase rename through temporary names so swaps and cycles are safe, automatic rollback on any mid-apply failure, and a post-apply log — self-sufficient for `rk_rollback`, which refuses entries whose size or mtime changed since apply. Nine rule types: replace (literal), prefix, suffix, case, trim, numbering, extension, insert, remove_range, mtime_prefix.

## Next, in order

1. **On-machine verification**: run the suite and a field pass of the built DLL on a real Windows machine (case-only renames on a real case-insensitive NTFS volume are the case the Linux host can only simulate). Nothing above is called Windows-verified until this happens.
2. **Content-hash rollback check (opt-in)**: record a hash of each applied file in the log so rollback can refuse on content, not only size+mtime. Needs a zero-dependency hash implementation and a spec flag; the stamp check stays the default.
3. **Per-file rule branches**: a rule that applies only to names matching a glob, so one plan can treat different groups differently (today one rule set applies to every source, which is why some swaps need the numbering trick).
4. **Dry-run diff export**: the plan as a standalone before/after text table for hosts that want to show it in their own UI without parsing JSON themselves.

## Explicitly not planned

- Regular expressions: `replace` stays literal. A regex rule would need a dependency or a hand-rolled engine, and the header will not fake one under a name like `regex_lite`.
- Format conversion: setting an extension renames; it never converts content. Conversion belongs to a converter kit.
- Recursion and cross-folder moves: one folder, plain names — that boundary is what keeps the two-phase guarantee simple to reason about.
