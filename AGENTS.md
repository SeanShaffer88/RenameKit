# RenameKit — agent notes

- The two-phase contract is load-bearing: phase 1 moves every changing file to a unique temp name in the same folder before any final name is claimed. That is what makes swaps and cycles safe — do not "optimize" it into direct renames for the common case.
- A failed apply must leave the folder as it was: rollback inside `engine::apply_json` is also two-phase (a completed final's target may be another entry's original name), and the log says `rolled_back:false` with the still-applied entries listed whenever a rollback step itself failed — never claim a rollback that didn't happen.
- Rollback refusal is per entry and stamp-based: size or mtime differing from the log means the file changed since apply and must be left alone and reported. Do not weaken it into "rename anyway" and do not strengthen it into refusing the whole batch — other entries still roll back.
- Chain semantics live in the planner: a target held by a source that is itself moving away is NOT `collision_existing`. The `changing` set in `build_plan` is computed from entries still `ok` at that point — moving that computation after statuses change breaks chains silently.
- Numbering rank is sorted order of the ORIGINAL names, computed before any rule runs. Numbering in listing order was the bug this design exists to prevent.
- There is NO regex rule and there must not be a fake one: `replace` is literal, the glob is `*`/`?` only. If regex is ever added it needs a real engine and a dependency justification.
- Extension rules rename only; nothing in this kit reads or converts file contents (the apply stamps read metadata, never content).
- The JSON engine (src/json.rs) is the family design — duplicate keys in one object are a PARSE ERROR by design; do not "fix" that into last-wins.
- Zero dependencies; filesystem access is `std::fs` in src/platform.rs only. Windows name rules (illegal chars, reserved names, 255) are policy in `lib.rs::validate_name`, applied on every OS on purpose.
- Gate: WinDevKit tools/gate.sh (fmt + clippy both targets + full tests). Check/clippy do not link — CI's real Windows build is the link gate.
