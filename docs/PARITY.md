# RenameKit — Parity

What the v0.1 brief asked for, and where each item stands. Verdicts: Complete (implemented and tested), Deferred (roadmap, with reason), Rejected (not doing, with reason).

| Brief item | Verdict | Evidence |
|---|---|---|
| Preview plan: per-entry from/to/full paths, status, issues, summary counts, `plan_valid` | Complete | `rk_preview`; statuses ok / unchanged / collision_existing / collision_in_plan / invalid / duplicate_source; tests in `src/tests.rs` assert each status |
| Rule: replace — literal, scope name/ext/both, `match_case` | Complete | `rule_replace_scopes_and_case` |
| Rule: regex_lite | Rejected | Deliberately not faked: no regex engine exists in the kit and none is claimed. An unknown rule type — including `regex_lite` — is a parse error (`spec_errors_are_parse_errors`). A real regex rule needs a dependency justification first |
| Rule: prefix / suffix (before extension) | Complete | `rule_prefix_and_suffix` |
| Rule: case — upper / lower / title / sentence | Complete | `rule_case_modes` |
| Rule: trim — spaces / underscores options | Complete | `rule_trim` |
| Rule: numbering — start/step/padding, prefix or suffix, separator, assigned in sorted order of original names | Complete | `rule_numbering_sorted_order` (listed b,a,c still numbers a,b,c) |
| Rule: extension — set / remove / lower | Complete | `rule_extension_set_remove_lower` |
| Rule: insert — at character index or before_ext | Complete | `rule_insert_and_remove_range` |
| Rule: remove_range — character-based, bounds-safe | Complete | Same test, including a multi-byte character case |
| Rule: mtime_prefix — format from unix mtime (UTC), zero-dependency civil-date conversion | Complete | `civil_date_known_values` (epoch, leap day 2000-02-29, a known 2023 timestamp) + `rule_mtime_prefix_uses_files_mtime` |
| Sources: explicit file list OR simple `*`/`?` glob implemented in-kit | Complete | `glob_pattern_expansion` (folders named like files are skipped) |
| `case_insensitive_fs` option, default true | Complete | `collision_in_plan_and_case_insensitive` runs the same plan with the flag on and off |
| Validation: empty result, illegal Windows characters, reserved names, result > 255 characters | Complete | `invalid_names_are_flagged` (also trailing period and missing source) |
| Collision: target exists on disk and is not renamed away by this plan | Complete | `collision_existing` |
| Collision: chain A→B, B→C is valid | Complete | `chain_within_one_plan_is_ok_and_applies` |
| Collision: two sources mapping to one target | Complete | `collision_in_plan_and_case_insensitive` |
| Duplicate source listed twice | Complete | `duplicate_source_flagged` |
| Apply refuses an invalid plan whole (nothing touched) unless `allow_partial` | Complete | `apply_refuses_invalid_plan_and_touches_nothing`, `apply_allow_partial` |
| Two-phase apply (temp names first), swaps and cycles safe | Complete | `swap_cycle_applies_via_two_phase` swaps file contents between 1.txt and 2.txt in one plan |
| Mid-apply failure rolls back everything already done | Complete | `apply_failure_rolls_back_everything` (directory pre-created on a final target; original layout restored, no temp files left) |
| Success log includes a rollback token; rollback works from the returned log alone | Complete | `rollback_from_log_restores_names` (whole log, and the bare token wrapped on its own) |
| Rollback refuses entries changed since apply (size + mtime recorded in the log) | Complete | `rollback_refuses_file_changed_since_apply`; also refuses an original name re-occupied by another file |
| Transform / format conversion | Rejected (out of scope) | Rename only: an extension change never touches content. Stated in README limitations; conversion belongs to a converter kit |
| Windows DLL behavior on a real Windows machine | Deferred | The suite runs on the Linux host; semantics that differ are policy in code and unit-tested, but no on-machine pass has run yet. ROADMAP item 1 |

## Test count

33 tests (29 RenameKit + 4 JSON engine), all against real files in real temporary directories. Gate: `../windevkit/tools/gate.sh`.
