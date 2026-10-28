/* RenameKit.dll — safe batch rename and transform behind a
 * small C ABI: preview, validate and apply bulk renames with
 * collision detection and rollback.
 *
 *   char *plan = rk_preview(spec);          // nothing changes
 *   char *log  = rk_apply(spec);            // two-phase apply
 *   char *back = rk_rollback(log);          // undo from the log
 *
 * THE SPEC (one JSON object; rk_preview and rk_apply take the
 * same document):
 *
 *   {
 *     "dir": "C:\\photos",                  // required
 *     "files": ["a.txt", "b.txt"],          // EITHER this...
 *     "pattern": "*.txt",                   // ...OR this
 *     "rules": [ ... ],                     // applied in order
 *     "case_insensitive_fs": true,          // optional, default true
 *     "allow_partial": false                // optional, apply only
 *   }
 *
 *   dir      the folder the files live in. Every name is a
 *            PLAIN file name inside this folder — no paths, no
 *            subfolders, no "." or "..".
 *   files    source names, in the caller's order.
 *   pattern  a simple glob over the folder's FILES (folders are
 *            skipped), expanded in sorted order. Only * (any
 *            run of characters) and ? (exactly one character)
 *            are special — this is NOT a regular expression.
 *   case_insensitive_fs
 *            when true (the default, matching Windows), name
 *            comparisons — duplicate sources, targets that
 *            collide, glob matching — ignore letter case.
 *   allow_partial
 *            rk_apply only: when false (default) an invalid
 *            plan refuses the WHOLE job and nothing is renamed.
 *            When true, only the entries marked "ok" are
 *            applied and the rest are listed as skipped.
 *
 *   Unknown keys are an error (RK_ERR_PARSE), never ignored;
 *   duplicate keys in one JSON object are a parse error too.
 *   This kit has NO regex rule by design: "replace" is a
 *   literal string replacement, and nothing here pretends
 *   otherwise.
 *
 * RULES. Each rule is an object with a "type". Rules run in
 * array order; every rule sees the previous rule's output.
 * A name is handled as stem + extension, where the extension
 * is the part after the LAST dot and a leading dot does not
 * count (".gitignore" is all stem; "archive.tar.gz" has stem
 * "archive.tar" and extension "gz").
 *
 *   {"type":"replace","find":"x","replace":"y",
 *    "scope":"name|ext|both","match_case":true}
 *            Literal (never regex) replace of EVERY occurrence.
 *            "find" is required and must not be empty; "replace"
 *            defaults to "" (delete); "scope" defaults to
 *            "both" (the whole file name); "match_case" defaults
 *            to true — false matches letter case insensitively.
 *   {"type":"prefix","text":"2024-"}
 *            Prepend text to the stem.
 *   {"type":"suffix","text":"-old"}
 *            Append text to the stem (before the extension).
 *   {"type":"case","mode":"upper|lower|title|sentence",
 *    "scope":"name|ext|both"}
 *            Change the letter case of the stem (scope defaults
 *            to "name", so the extension is untouched unless you
 *            ask). Title case capitalizes each word (words are
 *            separated by space, '_', '-' or '.') and lowercases
 *            the rest; sentence case lowercases everything, then
 *            capitalizes the first letter.
 *   {"type":"trim","spaces":true,"underscores":false}
 *            Trim characters from BOTH ENDS of the stem.
 *            "spaces" (spaces and tabs, default true) and
 *            "underscores" (default false) choose what is cut.
 *   {"type":"numbering","start":1,"step":1,"padding":3,
 *    "position":"suffix","separator":"_"}
 *            Add a sequence number. Numbers are assigned in
 *            SORTED ORDER OF THE ORIGINAL NAMES (case-insensitive
 *            sort, original name as tie-break), never in listing
 *            order. "position":"suffix" gives name_001,
 *            "prefix" gives 001_name (with the defaults).
 *            padding 0..=32; start/step may be any integers.
 *   {"type":"extension","action":"set","value":"md"}
 *   {"type":"extension","set":"md"}        (short form)
 *   {"type":"extension","action":"remove"}
 *   {"type":"extension","action":"lower"}
 *            Set (a leading dot in the value is stripped),
 *            remove, or lowercase the extension. NOTE: setting
 *            an extension only renames the file — it does NOT
 *            convert the file's contents. See README.
 *   {"type":"insert","text":"-v2","at":"before_ext"}
 *   {"type":"insert","text":"-v2","at":4}
 *            Insert text into the stem: at a CHARACTER index
 *            (0 = start; an index past the end appends), or
 *            "before_ext" (the default), which appends to the
 *            stem, just before the extension.
 *   {"type":"remove_range","start":0,"len":4}
 *            Remove len CHARACTERS from the stem starting at
 *            character index start. Out-of-range values are
 *            clamped — this rule never fails on a short name.
 *   {"type":"mtime_prefix","format":"YYYYMMDD","separator":"_"}
 *            Prepend the file's modification time (UTC), e.g.
 *            "20240131_report.txt". Format tokens YYYY MM DD
 *            HH mm ss are replaced (zero-padded); every other
 *            character is literal. Defaults: format "YYYYMMDD",
 *            separator "_". If a file's modification time
 *            cannot be read, that entry is "invalid".
 *
 * PREVIEW — rk_preview(spec) returns one JSON object:
 *
 *   {"dir":"...","entries":[
 *     {"from":"a.txt","to":"a-001.txt",
 *      "full_from":".../a.txt","full_to":".../a-001.txt",
 *      "status":"ok","issues":[]}],
 *    "summary":{"total":1,"ok":1,"unchanged":0,
 *      "collision_existing":0,"collision_in_plan":0,
 *      "invalid":0,"duplicate_source":0},
 *    "plan_valid":true}
 *
 *   status is exactly one of:
 *     ok                  ready to rename
 *     unchanged           the rules leave the name as it is
 *     collision_existing  the target name is taken by a file
 *                         that is NOT being renamed away by
 *                         this plan. A chain A->B, B->C is fine:
 *                         B's file moves away, so A may take B.
 *                         (A DIRECTORY sitting on a target is
 *                         reported in issues instead: apply
 *                         will fail on it and roll back.)
 *     collision_in_plan   two sources in this plan map to the
 *                         same target (compared case-
 *                         insensitively when the flag is set)
 *     invalid             the result breaks a Windows name
 *                         rule (empty, an illegal character
 *                         < > : " / \ | ? * or a control char,
 *                         a reserved device name CON PRN AUX
 *                         NUL COM1-9 LPT1-9, ending space or
 *                         period, longer than 255 characters)
 *                         or the source is not an existing file
 *     duplicate_source    the same source name is listed again
 *   issues carries plain-word explanations (may be non-empty
 *   on an "ok" entry as a warning). plan_valid is true only
 *   when every entry is ok or unchanged. Preview changes
 *   NOTHING on disk.
 *
 * APPLY — rk_apply(spec) recomputes the preview, then renames
 * in TWO PHASES: phase 1 moves every changing file to a unique
 * temporary name in the same folder; phase 2 moves each
 * temporary to its final name. Swaps (A<->B) and cycles are
 * therefore safe. It returns a log JSON object:
 *
 *   {"ok":true,"dir":"...","refused":false,"reason":null,
 *    "applied":[{"from":"a.txt","to":"a-001.txt",
 *      "full_from":"...","full_to":"...",
 *      "size":12,"mtime_unix":1700000000,
 *      "mtime_unix_nanos":1700000000000000000}],
 *    "failed":null,"rolled_back":false,"rollback_errors":[],
 *    "skipped":[],"rollback_token":"{\"entries\":[...]}",
 *    "plan":{ ...the preview object... }}
 *
 *   refused     true when the plan was invalid and
 *               allow_partial was false: NOTHING was renamed.
 *   failed      on a mid-apply failure, the entry that failed
 *               and the OS error. Everything else is rolled
 *               back (rolled_back true) before returning, so a
 *               failed apply leaves the folder as it was —
 *               unless a rollback step itself failed, which
 *               rollback_errors then reports and "applied"
 *               lists what is still under its final name.
 *   size / mtime_unix / mtime_unix_nanos
 *               each applied file's size and modification
 *               stamp recorded right after the apply. This is
 *               what rk_rollback checks before undoing.
 *   rollback_token
 *               the applied list as a self-contained JSON
 *               string. Keep the whole log, or just the token;
 *               either one is enough for rk_rollback.
 *
 * ROLLBACK — rk_rollback(log) takes the full apply log, OR an
 * object carrying "rollback_token", OR a bare token document
 * ({"entries":[...]}). It renames final -> original for every
 * applied entry, in reverse order, also two-phase, and
 * returns:
 *
 *   {"ok":true,"restored":[ ...applied-shaped entries... ],
 *    "refused":[{"from":..,"to":..,"reason":"..."}],
 *    "failed":[{"from":..,"to":..,"error":"..."}]}
 *
 *   An entry is REFUSED (left exactly where it is, and
 *   reported) when its file no longer exists, when its size or
 *   modification time differs from the log (the file changed
 *   since apply — undoing would hide that change), or when its
 *   original name is now occupied by a file that is not part
 *   of this rollback. Other entries still roll back. Entries
 *   recorded without stamps (logs written by hand) are rolled
 *   back unchecked — the refusal protection only exists when
 *   the log carries the stamps a real apply wrote.
 *
 * Return-value rules for all three JSON calls: a result
 * string (caller-owned, free with rk_free_string) or NULL on
 * a hard failure (null/non-UTF-8 input, malformed JSON, an
 * unknown key or rule, a missing directory), with the thread-
 * local rk_last_error() / rk_last_error_code() set. A REFUSED
 * or FAILED apply is not a hard failure: it returns its log
 * with ok:false. rk_version()'s string is static — do NOT
 * free it.
 */
#ifndef RENAMEKIT_H
#define RENAMEKIT_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
#if defined(_WIN32) && defined(RK_BUILD)
#define RK_API __declspec(dllexport)
#elif defined(_WIN32)
#define RK_API __declspec(dllimport)
#else
#define RK_API
#endif
#define RK_OK 0
#define RK_ERR_INVALID_ARG 1
#define RK_ERR_NOT_SUPPORTED 2
#define RK_ERR_NOT_FOUND 3
#define RK_ERR_ACCESS_DENIED 4
#define RK_ERR_PLATFORM 5
#define RK_ERR_INTERNAL 6
#define RK_ERR_PARSE 7

/* Library version, e.g. "0.1.0". Static string: do NOT free. */
RK_API const char *rk_version(void);

/* Thread-local message for the last failing call on this
 * thread ("" when the last call succeeded). Static storage:
 * do NOT free. */
RK_API const char *rk_last_error(void);

/* Thread-local code (RK_*) for the last failing call. */
RK_API int rk_last_error_code(void);

/* Free a string returned by rk_preview / rk_apply /
 * rk_rollback. NULL is a no-op. Freeing anything else, or
 * freeing twice, is undefined. */
RK_API void rk_free_string(char *p);

/* Preview a rename spec: build the plan and return it as
 * JSON (shape above). Changes nothing on disk. Caller-owned
 * result; NULL + rk_last_error on a hard failure. */
RK_API char *rk_preview(const char *spec_json);

/* Apply a rename spec: recompute the plan, then rename
 * two-phase, or refuse / roll back as documented above.
 * Caller-owned log JSON; NULL + rk_last_error on a hard
 * failure (bad spec, missing directory). */
RK_API char *rk_apply(const char *spec_json);

/* Roll back a successful apply, from its log (or token)
 * alone. Caller-owned result JSON; NULL + rk_last_error on
 * a hard failure (bad input, no applied entries found). */
RK_API char *rk_rollback(const char *log_json);

#ifdef __cplusplus
}
#endif
#endif
