# RenameKit

## TL;DR

RenameKit renames a batch of files safely, behind one small zero-dependency DLL. You describe the batch once — a folder, the files (or a simple `*.txt`-style pattern), and a list of rename rules — and RenameKit shows you the exact plan first, checks every new name for problems and clashes, and only then renames. Renaming happens in two phases through temporary names, so even swapping two files' names works, and if anything fails halfway everything is put back. A successful rename hands you a log that can undo the whole batch later — unless a file has changed since, in which case the undo refuses that file and tells you why.

```c
#include "renamekit.h"
#include <stdio.h>

int main(void) {
    const char *spec =
        "{\"dir\":\"C:\\\\photos\","
        " \"pattern\":\"*.JPG\","
        " \"rules\":[{\"type\":\"extension\",\"action\":\"lower\"},"
        "           {\"type\":\"numbering\",\"padding\":3}]}";

    char *plan = rk_preview(spec);          /* nothing changes yet */
    printf("%s\n", plan);                    /* check plan_valid    */
    rk_free_string(plan);

    char *log = rk_apply(spec);              /* keep this log!      */
    printf("%s\n", log);

    /* ...later, undo the whole batch from the log alone... */
    char *back = rk_rollback(log);
    printf("%s\n", back);
    rk_free_string(back);
    rk_free_string(log);
    return 0;
}
```

## Features

- **Preview before anything moves.** `rk_preview` returns the full plan — every old name, every new name, a status per file, and plain-word explanations — and touches nothing on disk.
- **Nine rule types, applied in order:** literal `replace` (with scope and case options), `prefix`, `suffix`, `case` (upper / lower / title / sentence), `trim`, `numbering`, `extension` (set / remove / lower), `insert`, `remove_range`, and `mtime_prefix` (the file's modification date, in UTC, formatted with `YYYY MM DD HH mm ss` tokens). The full rule reference is in `include/renamekit.h`.
- **Numbering that counts fairly.** Numbers are handed out in sorted order of the *original* names, not in the order the files happen to be listed.
- **Real validation.** Empty results, characters Windows forbids (`< > : " / \ | ? *` and control characters), reserved device names (CON, PRN, AUX, NUL, COM1–9, LPT1–9), names ending in a space or period, and names over 255 characters are all flagged per file before anything runs.
- **Collision detection that understands chains.** A target that is already taken is a problem — unless the file sitting there is itself being renamed away by the same plan, so A→B, B→C is allowed. Two files in one plan aiming at the same target are flagged too, case-insensitively by default (the `case_insensitive_fs` option, default true, controls this).
- **All-or-nothing by default.** An invalid plan refuses the whole job with nothing renamed. `"allow_partial": true` in the spec applies only the entries marked ok and lists the rest as skipped.
- **Two-phase apply.** Phase 1 moves every changing file to a unique temporary name in the same folder; phase 2 moves each temporary to its final name. Swaps (A↔B) and cycles are safe, and any failure rolls everything already done back before the log is returned.
- **Rollback from the log alone.** A successful apply returns a log recording each rename plus the file's size and modification stamp after the apply. `rk_rollback` (given the whole log, or just its `rollback_token`) renames everything back — and refuses, per file, any file whose size or modification time no longer matches the log, because that file has changed since the rename and undoing would hide the change.
- **Simple glob patterns.** `"pattern": "*.txt"` selects the folder's files with `*` (any run of characters) and `?` (exactly one character). That is the whole pattern language — there is deliberately no regular-expression rule; `replace` is a literal string replacement and is documented as such.
- **Zero dependencies, one small C ABI.** Three JSON-in / JSON-out calls (`rk_preview`, `rk_apply`, `rk_rollback`) plus version / last-error / free-string helpers. All strings are UTF-8; every returned string is caller-owned and freed with `rk_free_string`.

## Building

```
cargo build --release     # renamekit.dll / librenamekit.so / librenamekit.a
cargo test --locked       # the full test suite
```

Rust 1.80 or newer. There are no dependencies to fetch.

## Tests

33 tests, all running against real files in real temporary directories: every rule on its own (including character-based — not byte-based — ranges, and a leap-day check of the date conversion), rules combined in order, glob expansion, invalid names, duplicate sources, disk and in-plan collisions (including the case-insensitive comparison and the case-only rename), a chain (A→B, B→C) and a swap (A↔B) applied in one plan, an invalid plan refusing with nothing touched, `allow_partial`, a forced mid-apply failure (a directory sitting on a final target) rolling everything back with no temporary files left behind, rollback from the log and from the bare token, rollback refusing a file changed since apply and a file whose original name was taken, spec parse errors (including duplicate JSON keys), and the C ABI end to end.

## Limitations — the honest list

- **Extension changes do not convert anything.** This kit is rename-focused: setting a file's extension from `.txt` to `.md` changes its name only, never its contents. Format conversion is out of scope.
- **One folder, plain file names only.** Sources are files directly inside `dir`; there is no recursion, no renaming of folders, and no moving files between folders.
- **Windows name rules are checked even on other systems.** That is deliberate — a plan made on Linux is promised to be legal on Windows — but it means a name Linux would accept (say, one containing `:`) is reported invalid here.
- **Name length is counted in characters** (the 255 limit). Windows itself counts in UTF-16 code units; for names outside the basic multilingual plane the kit's count is slightly generous.
- **Rollback's change check is size + modification time.** A file rewritten with exactly the same size within the same timestamp tick could in principle pass the check; the log records nanosecond stamps where the filesystem provides them, which makes that window very small, but it is a stamp check, not a content hash.
- **A directory sitting on a target name** is reported as a warning on an otherwise-ok plan entry; the apply then fails on that entry and rolls the job back, rather than the preview refusing the plan.
- **Hand-written logs roll back unchecked.** The changed-file refusal only works when the log carries the size/mtime stamps a real `rk_apply` wrote; entries without stamps are trusted as given.
- **Not yet run on a real Windows machine.** The logic and the test suite run on the Linux host; Windows behavior of the shipped DLL is Unverified until an on-machine pass — the semantics that differ (case-insensitivity, reserved names) are policy in the code and covered by tests, but the DLL itself has not been field-tested.

## License

MIT. Part of the WinDevKit family — see `AGENTS.md` for build discipline and `ROADMAP.md` for what's next.
