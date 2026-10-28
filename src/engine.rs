//! Apply and rollback engine.
//!
//! Apply is TWO-PHASE: phase 1 renames every changing file to a
//! unique temporary name in the same directory, phase 2 renames
//! each temporary to its final name. Because no final name is
//! claimed until every source has moved out of the way, swaps
//! (A<->B) and cycles are safe. Any failure restores the original
//! layout (also two-phase) before the log is returned.
//!
//! A successful apply returns a log that is, by itself, everything
//! `rk_rollback` needs: the applied entries with the size and
//! modification stamp each file had right after the apply, plus a
//! `rollback_token` carrying the same entries as a JSON string.
//! Rollback refuses (per entry, and reports) any file whose size
//! or modification time no longer matches the log — that file is
//! not the file the apply produced any more.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::json;
use crate::platform;
use crate::{build_plan, json_escape, norm, parse_spec, Entry, Plan, Status, ERR_PARSE};

fn esc(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

fn err<T>(code: i32, msg: impl Into<String>) -> Result<T, (i32, String)> {
    Err((code, msg.into()))
}

/// One applied rename, as recorded in an apply log.
#[derive(Clone, Debug)]
pub struct AppliedEntry {
    pub from: String,
    pub to: String,
    pub full_from: String,
    pub full_to: String,
    pub size: Option<u64>,
    pub mtime_unix: Option<i64>,
    pub mtime_unix_nanos: Option<i64>,
}

fn applied_json(e: &AppliedEntry) -> String {
    let mut out = String::from("{\"from\":");
    out.push_str(&esc(&e.from));
    out.push_str(",\"to\":");
    out.push_str(&esc(&e.to));
    out.push_str(",\"full_from\":");
    out.push_str(&esc(&e.full_from));
    out.push_str(",\"full_to\":");
    out.push_str(&esc(&e.full_to));
    out.push_str(",\"size\":");
    out.push_str(
        &e.size
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".into()),
    );
    out.push_str(",\"mtime_unix\":");
    out.push_str(
        &e.mtime_unix
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".into()),
    );
    out.push_str(",\"mtime_unix_nanos\":");
    out.push_str(
        &e.mtime_unix_nanos
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".into()),
    );
    out.push('}');
    out
}

fn applied_array(entries: &[AppliedEntry]) -> String {
    let mut out = String::from("[");
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&applied_json(e));
    }
    out.push(']');
    out
}

fn stamp_of(path: &Path) -> AppliedStamp {
    match platform::stamp(path) {
        Ok(s) => AppliedStamp {
            size: Some(s.size),
            mtime_unix: Some(s.mtime_unix),
            mtime_unix_nanos: Some(s.mtime_unix_nanos),
        },
        Err(_) => AppliedStamp {
            size: None,
            mtime_unix: None,
            mtime_unix_nanos: None,
        },
    }
}

#[derive(Clone, Copy)]
struct AppliedStamp {
    size: Option<u64>,
    mtime_unix: Option<i64>,
    mtime_unix_nanos: Option<i64>,
}

/// A work item for the two-phase shuffle: a file currently at
/// `current` that must end up at `original` during a restore.
struct RestoreItem {
    entry: AppliedEntry,
    current: PathBuf,
    temp: Option<PathBuf>,
}

/// Two-phase restore shared by apply-failure rollback and
/// `rk_rollback`: phase 1 moves every item's current file to a
/// fresh temp name, phase 2 moves each temp to the original name.
/// Returns (restored entries, failure messages).
fn restore_two_phase(items: &mut [RestoreItem]) -> (Vec<AppliedEntry>, Vec<String>) {
    let mut failures: Vec<String> = Vec::new();
    // Phase 1: current -> temp.
    for item in items.iter_mut() {
        let dir = item
            .current
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let temp = platform::temp_path(&dir, "restore");
        match platform::rename(&item.current, &temp) {
            Ok(()) => item.temp = Some(temp),
            Err((_, msg)) => failures.push(msg),
        }
    }
    // Phase 2: temp -> original.
    let mut restored = Vec::new();
    for item in items.iter_mut() {
        if let Some(temp) = item.temp.take() {
            let original = PathBuf::from(&item.entry.full_from);
            match platform::rename(&temp, &original) {
                Ok(()) => restored.push(item.entry.clone()),
                Err((_, msg)) => {
                    failures.push(msg);
                    // Best effort: put it back where it was.
                    let _ = platform::rename(&temp, &item.current);
                    item.temp = None;
                }
            }
        }
    }
    (restored, failures)
}

fn entry_to_applied(e: &Entry) -> AppliedEntry {
    AppliedEntry {
        from: e.from.clone(),
        to: e.to.clone(),
        full_from: e.full_from.clone(),
        full_to: e.full_to.clone(),
        size: None,
        mtime_unix: None,
        mtime_unix_nanos: None,
    }
}

struct LogParts<'a> {
    plan: &'a Plan,
    ok: bool,
    refused: bool,
    reason: Option<&'a str>,
    applied: &'a [AppliedEntry],
    failed: Option<(&'a Entry, &'a str)>,
    rolled_back: bool,
    rollback_errors: &'a [String],
    skipped: &'a [&'a Entry],
}

fn log_json(parts: LogParts) -> String {
    let LogParts {
        plan,
        ok,
        refused,
        reason,
        applied,
        failed,
        rolled_back,
        rollback_errors,
        skipped,
    } = parts;
    let mut out = String::from("{\"ok\":");
    out.push_str(if ok { "true" } else { "false" });
    out.push_str(",\"dir\":");
    out.push_str(&esc(&plan.dir));
    out.push_str(",\"refused\":");
    out.push_str(if refused { "true" } else { "false" });
    out.push_str(",\"reason\":");
    match reason {
        Some(r) => out.push_str(&esc(r)),
        None => out.push_str("null"),
    }
    out.push_str(",\"applied\":");
    out.push_str(&applied_array(applied));
    out.push_str(",\"failed\":");
    match failed {
        Some((e, msg)) => {
            out.push_str("{\"from\":");
            out.push_str(&esc(&e.from));
            out.push_str(",\"to\":");
            out.push_str(&esc(&e.to));
            out.push_str(",\"error\":");
            out.push_str(&esc(msg));
            out.push('}');
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"rolled_back\":");
    out.push_str(if rolled_back { "true" } else { "false" });
    out.push_str(",\"rollback_errors\":[");
    for (i, m) in rollback_errors.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&esc(m));
    }
    out.push_str("],\"skipped\":[");
    for (i, e) in skipped.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"from\":");
        out.push_str(&esc(&e.from));
        out.push_str(",\"to\":");
        out.push_str(&esc(&e.to));
        out.push_str(",\"status\":");
        out.push_str(&esc(e.status.as_str()));
        out.push('}');
    }
    out.push_str("],\"rollback_token\":");
    // The token is the applied list as a self-contained JSON
    // document (carried as a string, so the log stays one level
    // of plain JSON a caller can store and hand back verbatim).
    let token = format!("{{\"entries\":{}}}", applied_array(applied));
    out.push_str(&esc(&token));
    out.push_str(",\"plan\":");
    out.push_str(&crate::plan_to_json(plan));
    out.push('}');
    out
}

/// `rk_apply`'s work. Recomputes the preview; a plan that is not
/// valid refuses the whole job (nothing on disk is touched)
/// unless the spec set `allow_partial`, in which case only the
/// `ok` entries are applied.
pub fn apply_json(spec_text: &str) -> Result<String, (i32, String)> {
    let spec = parse_spec(spec_text)?;
    let plan = build_plan(&spec)?;
    let dir = spec.dir_path();
    let ci = spec.case_insensitive_fs;

    let changing: Vec<&Entry> = plan
        .entries
        .iter()
        .filter(|e| e.status == Status::Ok)
        .collect();
    let skipped: Vec<&Entry> = plan
        .entries
        .iter()
        .filter(|e| !matches!(e.status, Status::Ok | Status::Unchanged))
        .collect();

    if !plan.plan_valid && !spec.allow_partial {
        return Ok(log_json(LogParts {
            plan: &plan,
            ok: false,
            refused: true,
            reason: Some("the plan is not valid; nothing was renamed. Fix the plan or set \"allow_partial\":true to apply only the ok entries"),
            applied: &[],
            failed: None,
            rolled_back: false,
            rollback_errors: &[],
            skipped: &skipped,
        }));
    }

    // Phase 1: every changing file -> unique temp in the same dir.
    struct Work {
        entry: Entry,
        temp: PathBuf,
        done: bool, // reached its final name
    }
    let mut work: Vec<Work> = Vec::new();
    for e in &changing {
        let temp = platform::temp_path(&dir, "apply");
        if let Err((_, msg)) = platform::rename(Path::new(&e.full_from), &temp) {
            // Undo phase 1 for the files already moved: their own
            // original names are free, so a direct rename restores.
            let mut rb_errors = Vec::new();
            for w in work.iter().rev() {
                if let Err((_, m)) = platform::rename(&w.temp, Path::new(&w.entry.full_from)) {
                    rb_errors.push(m);
                }
            }
            let rolled = rb_errors.is_empty();
            return Ok(log_json(LogParts {
                plan: &plan,
                ok: false,
                refused: false,
                reason: Some("apply failed in phase 1 (moving files to temporary names); already-moved files were restored"),
                applied: &[],
                failed: Some((e, &msg)),
                rolled_back: rolled,
                rollback_errors: &rb_errors,
                skipped: &skipped,
            }));
        }
        work.push(Work {
            entry: (*e).clone(),
            temp,
            done: false,
        });
    }

    // Phase 2: temp -> final.
    let mut failed: Option<(Entry, String)> = None;
    for w in work.iter_mut() {
        if let Err((_, msg)) = platform::rename(&w.temp, Path::new(&w.entry.full_to)) {
            failed = Some((w.entry.clone(), msg));
            break;
        }
        w.done = true;
    }

    if let Some((fentry, msg)) = failed {
        // Roll everything back, two-phase: completed finals go to
        // fresh temps first (their target names may be another
        // entry's original name), then every temp goes home.
        let mut items: Vec<RestoreItem> = work
            .iter()
            .map(|w| RestoreItem {
                entry: entry_to_applied(&w.entry),
                current: if w.done {
                    PathBuf::from(&w.entry.full_to)
                } else {
                    w.temp.clone()
                },
                temp: None,
            })
            .collect();
        // Items still sitting on their phase-1 temp must not be
        // renamed temp->temp pointlessly; restore_two_phase moves
        // current->temp->original uniformly, which is correct for
        // both shapes, just one extra rename for those.
        let (restored, rb_errors) = restore_two_phase(&mut items);
        let rolled = rb_errors.is_empty() && restored.len() == work.len();
        let still_applied: Vec<AppliedEntry> = if rolled {
            Vec::new()
        } else {
            work.iter()
                .filter(|w| w.done)
                .filter(|w| {
                    !restored
                        .iter()
                        .any(|r| norm(&r.from, ci) == norm(&w.entry.from, ci))
                })
                .map(|w| {
                    let mut a = entry_to_applied(&w.entry);
                    let s = stamp_of(Path::new(&w.entry.full_to));
                    a.size = s.size;
                    a.mtime_unix = s.mtime_unix;
                    a.mtime_unix_nanos = s.mtime_unix_nanos;
                    a
                })
                .collect()
        };
        return Ok(log_json(LogParts {
            plan: &plan,
            ok: false,
            refused: false,
            reason: Some("apply failed in phase 2 (moving files to their final names); the job was rolled back"),
            applied: &still_applied,
            failed: Some((&fentry, &msg)),
            rolled_back: rolled,
            rollback_errors: &rb_errors,
            skipped: &skipped,
        }));
    }

    // Success: stamp each final file for the rollback log.
    let applied: Vec<AppliedEntry> = work
        .iter()
        .map(|w| {
            let mut a = entry_to_applied(&w.entry);
            let s = stamp_of(Path::new(&w.entry.full_to));
            a.size = s.size;
            a.mtime_unix = s.mtime_unix;
            a.mtime_unix_nanos = s.mtime_unix_nanos;
            a
        })
        .collect();
    Ok(log_json(LogParts {
        plan: &plan,
        ok: true,
        refused: false,
        reason: None,
        applied: &applied,
        failed: None,
        rolled_back: false,
        rollback_errors: &[],
        skipped: &skipped,
    }))
}

// ---------------------------------------------------------------- rollback

fn applied_from_value(v: &json::Value) -> Result<AppliedEntry, (i32, String)> {
    let get_s = |k: &str| -> Result<String, (i32, String)> {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| (ERR_PARSE, format!("rollback entry is missing \"{k}\"")))
    };
    Ok(AppliedEntry {
        from: get_s("from")?,
        to: get_s("to")?,
        full_from: get_s("full_from")?,
        full_to: get_s("full_to")?,
        size: v.get("size").and_then(|x| x.as_i64()).map(|n| n as u64),
        mtime_unix: v.get("mtime_unix").and_then(|x| x.as_i64()),
        mtime_unix_nanos: v.get("mtime_unix_nanos").and_then(|x| x.as_i64()),
    })
}

/// Extract the applied entries from a whole apply log, from a
/// log's `rollback_token` string, or from a bare token document.
fn extract_applied(text: &str) -> Result<Vec<AppliedEntry>, (i32, String)> {
    let v = json::parse(text)
        .map_err(|e| (ERR_PARSE, format!("rollback input is not valid JSON: {e}")))?;
    let array = if let Some(a) = v.get("applied").and_then(|x| x.as_array()) {
        a.clone()
    } else if let Some(tok) = v.get("rollback_token").and_then(|x| x.as_str()) {
        let tv = json::parse(tok)
            .map_err(|e| (ERR_PARSE, format!("rollback_token is not valid JSON: {e}")))?;
        tv.get("entries")
            .and_then(|x| x.as_array())
            .cloned()
            .ok_or_else(|| {
                (
                    ERR_PARSE,
                    "rollback_token has no \"entries\" array".to_string(),
                )
            })?
    } else if let Some(a) = v.get("entries").and_then(|x| x.as_array()) {
        a.clone()
    } else {
        return err(
            ERR_PARSE,
            "rollback input has no \"applied\" array and no \"rollback_token\"",
        );
    };
    let mut out = Vec::new();
    for item in &array {
        out.push(applied_from_value(item)?);
    }
    Ok(out)
}

/// `rk_rollback`'s work: rename every applied entry's final name
/// back to its original name, in reverse order, two-phase.
/// Entries whose current file has a different size or mtime than
/// the log recorded are REFUSED (left in place and reported);
/// entries whose original name is now occupied by a file that is
/// not part of this rollback are refused too.
pub fn rollback_json(log_text: &str) -> Result<String, (i32, String)> {
    let applied = extract_applied(log_text)?;
    // The set of final names taking part (a file sitting on an
    // original name is fine when it is itself being rolled back).
    let finals: HashSet<String> = applied.iter().map(|e| e.full_to.to_lowercase()).collect();

    let mut refused: Vec<(AppliedEntry, String)> = Vec::new();
    let mut eligible: Vec<AppliedEntry> = Vec::new();
    for e in applied.iter().rev() {
        let full_to = Path::new(&e.full_to);
        if !full_to.is_file() {
            refused.push((
                e.clone(),
                format!("\"{}\" no longer exists as a file", e.to),
            ));
            continue;
        }
        if e.size.is_some() || e.mtime_unix_nanos.is_some() {
            match platform::stamp(full_to) {
                Ok(s) => {
                    if e.size.is_some() && e.size != Some(s.size) {
                        refused.push((
                            e.clone(),
                            format!(
                                "\"{}\" changed size since apply ({} -> {} bytes); refusing to roll it back",
                                e.to,
                                e.size.unwrap_or(0),
                                s.size
                            ),
                        ));
                        continue;
                    }
                    if e.mtime_unix_nanos.is_some()
                        && e.mtime_unix_nanos != Some(s.mtime_unix_nanos)
                    {
                        refused.push((
                            e.clone(),
                            format!(
                                "\"{}\" was modified since apply; refusing to roll it back",
                                e.to
                            ),
                        ));
                        continue;
                    }
                }
                Err((_, msg)) => {
                    refused.push((e.clone(), msg));
                    continue;
                }
            }
        }
        let full_from = Path::new(&e.full_from);
        if full_from.exists()
            && !finals.contains(&e.full_from.to_lowercase())
            && full_from != full_to
        {
            refused.push((
                e.clone(),
                format!(
                    "the original name \"{}\" is now occupied by another file",
                    e.from
                ),
            ));
            continue;
        }
        eligible.push(e.clone());
    }

    let mut items: Vec<RestoreItem> = eligible
        .iter()
        .map(|e| RestoreItem {
            entry: e.clone(),
            current: PathBuf::from(&e.full_to),
            temp: None,
        })
        .collect();
    let (restored, failures) = restore_two_phase(&mut items);
    let restored_froms: HashSet<String> = restored.iter().map(|r| r.full_from.clone()).collect();
    let failed_entries: Vec<(AppliedEntry, String)> = eligible
        .iter()
        .filter(|e| !restored_froms.contains(&e.full_from))
        .cloned()
        .zip(
            failures
                .iter()
                .cloned()
                .chain(std::iter::repeat_with(|| "restore failed".to_string())),
        )
        .collect();

    let ok = refused.is_empty() && failed_entries.is_empty();
    let mut out = String::from("{\"ok\":");
    out.push_str(if ok { "true" } else { "false" });
    out.push_str(",\"restored\":");
    out.push_str(&applied_array(&restored));
    out.push_str(",\"refused\":[");
    for (i, (e, why)) in refused.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"from\":");
        out.push_str(&esc(&e.from));
        out.push_str(",\"to\":");
        out.push_str(&esc(&e.to));
        out.push_str(",\"reason\":");
        out.push_str(&esc(why));
        out.push('}');
    }
    out.push_str("],\"failed\":[");
    for (i, (e, why)) in failed_entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"from\":");
        out.push_str(&esc(&e.from));
        out.push_str(",\"to\":");
        out.push_str(&esc(&e.to));
        out.push_str(",\"error\":");
        out.push_str(&esc(why));
        out.push('}');
    }
    out.push_str("]}");
    Ok(out)
}
