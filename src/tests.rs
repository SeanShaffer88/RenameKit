//! Integration-style tests: every test works in a real
//! temporary directory with real files, and drives the same
//! JSON-in / JSON-out functions the C ABI wraps.

use super::*;
use crate::engine;
use crate::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("renamekit-test-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn write(&self, name: &str, content: &str) {
        fs::write(self.0.join(name), content).unwrap();
    }
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).unwrap()
    }
    fn exists(&self, name: &str) -> bool {
        self.0.join(name).exists()
    }
    fn is_dir(&self, name: &str) -> bool {
        self.0.join(name).is_dir()
    }
    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
    fn spec(&self, files: &[&str], rules: &str) -> String {
        let files_json = files
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"dir\":\"{}\",\"files\":[{}],\"rules\":{}}}",
            self.0.display().to_string().replace('\\', "\\\\"),
            files_json,
            rules
        )
    }
    fn spec_pattern(&self, pattern: &str, rules: &str) -> String {
        format!(
            "{{\"dir\":\"{}\",\"pattern\":\"{}\",\"rules\":{}}}",
            self.0.display().to_string().replace('\\', "\\\\"),
            pattern,
            rules
        )
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn preview(td: &TempDir, files: &[&str], rules: &str) -> json::Value {
    json::parse(&preview_json(&td.spec(files, rules)).unwrap()).unwrap()
}

fn entry<'a>(plan: &'a json::Value, from: &str) -> &'a json::Value {
    plan.get("entries")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e.get("from").unwrap().as_str() == Some(from))
        .unwrap()
}

fn status_of(plan: &json::Value, from: &str) -> String {
    entry(plan, from)
        .get("status")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

fn to_of(plan: &json::Value, from: &str) -> String {
    entry(plan, from)
        .get("to")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

// ------------------------------------------------------------ rules

#[test]
fn rule_replace_scopes_and_case() {
    let td = TempDir::new("replace");
    td.write("foofoo.TXT", "x");
    // both, case-sensitive
    let p = preview(
        &td,
        &["foofoo.TXT"],
        r#"[{"type":"replace","find":"foo","replace":"bar"}]"#,
    );
    assert_eq!(to_of(&p, "foofoo.TXT"), "barbar.TXT");
    // name scope only
    let p = preview(
        &td,
        &["foofoo.TXT"],
        r#"[{"type":"replace","find":"TXT","replace":"md","scope":"name"}]"#,
    );
    assert_eq!(to_of(&p, "foofoo.TXT"), "foofoo.TXT");
    // ext scope
    let p = preview(
        &td,
        &["foofoo.TXT"],
        r#"[{"type":"replace","find":"TXT","replace":"md","scope":"ext"}]"#,
    );
    assert_eq!(to_of(&p, "foofoo.TXT"), "foofoo.md");
    // match_case false
    td.write("Hello.txt", "x");
    let p = preview(
        &td,
        &["Hello.txt"],
        r#"[{"type":"replace","find":"hello","replace":"bye","match_case":false}]"#,
    );
    assert_eq!(to_of(&p, "Hello.txt"), "bye.txt");
    // case-sensitive miss leaves it unchanged
    let p = preview(
        &td,
        &["Hello.txt"],
        r#"[{"type":"replace","find":"hello","replace":"bye"}]"#,
    );
    assert_eq!(status_of(&p, "Hello.txt"), "unchanged");
}

#[test]
fn rule_prefix_and_suffix() {
    let td = TempDir::new("prefix");
    td.write("a.txt", "x");
    let p = preview(
        &td,
        &["a.txt"],
        r#"[{"type":"prefix","text":"pre-"},{"type":"suffix","text":"-suf"}]"#,
    );
    assert_eq!(to_of(&p, "a.txt"), "pre-a-suf.txt");
}

#[test]
fn rule_case_modes() {
    let td = TempDir::new("case");
    td.write("hello_world-foo bar.TXT", "x");
    let p = preview(
        &td,
        &["hello_world-foo bar.TXT"],
        r#"[{"type":"case","mode":"upper"}]"#,
    );
    assert_eq!(
        to_of(&p, "hello_world-foo bar.TXT"),
        "HELLO_WORLD-FOO BAR.TXT"
    );
    let p = preview(
        &td,
        &["hello_world-foo bar.TXT"],
        r#"[{"type":"case","mode":"title"}]"#,
    );
    assert_eq!(
        to_of(&p, "hello_world-foo bar.TXT"),
        "Hello_World-Foo Bar.TXT"
    );
    td.write("hELLO WORLD.txt", "x");
    let p = preview(
        &td,
        &["hELLO WORLD.txt"],
        r#"[{"type":"case","mode":"sentence"}]"#,
    );
    assert_eq!(to_of(&p, "hELLO WORLD.txt"), "Hello world.txt");
    let p = preview(
        &td,
        &["hELLO WORLD.txt"],
        r#"[{"type":"case","mode":"lower","scope":"both"}]"#,
    );
    assert_eq!(to_of(&p, "hELLO WORLD.txt"), "hello world.txt");
}

#[test]
fn rule_trim() {
    let td = TempDir::new("trim");
    td.write("__  spaced  __.txt", "x");
    let p = preview(
        &td,
        &["__  spaced  __.txt"],
        r#"[{"type":"trim","spaces":true,"underscores":true}]"#,
    );
    assert_eq!(to_of(&p, "__  spaced  __.txt"), "spaced.txt");
    // defaults: spaces only — the ends are underscores, so
    // nothing is trimmed and the entry is unchanged
    let p = preview(&td, &["__  spaced  __.txt"], r#"[{"type":"trim"}]"#);
    assert_eq!(status_of(&p, "__  spaced  __.txt"), "unchanged");
}

#[test]
fn rule_numbering_sorted_order() {
    let td = TempDir::new("numbering");
    for f in ["b.txt", "a.txt", "c.txt"] {
        td.write(f, "x");
    }
    // Listed b,a,c — numbers follow SORTED original order a,b,c.
    let p = preview(
        &td,
        &["b.txt", "a.txt", "c.txt"],
        r#"[{"type":"numbering","start":1,"step":1,"padding":3,"position":"suffix","separator":"_"}]"#,
    );
    assert_eq!(to_of(&p, "a.txt"), "a_001.txt");
    assert_eq!(to_of(&p, "b.txt"), "b_002.txt");
    assert_eq!(to_of(&p, "c.txt"), "c_003.txt");
    // prefix position, custom start/step/padding
    let p = preview(
        &td,
        &["b.txt", "a.txt", "c.txt"],
        r#"[{"type":"numbering","start":10,"step":5,"padding":2,"position":"prefix","separator":"-"}]"#,
    );
    assert_eq!(to_of(&p, "a.txt"), "10-a.txt");
    assert_eq!(to_of(&p, "b.txt"), "15-b.txt");
    assert_eq!(to_of(&p, "c.txt"), "20-c.txt");
}

#[test]
fn rule_extension_set_remove_lower() {
    let td = TempDir::new("ext");
    td.write("a.TXT", "x");
    let p = preview(
        &td,
        &["a.TXT"],
        r#"[{"type":"extension","action":"lower"}]"#,
    );
    assert_eq!(to_of(&p, "a.TXT"), "a.txt");
    let p = preview(
        &td,
        &["a.TXT"],
        r#"[{"type":"extension","action":"set","value":".md"}]"#,
    );
    assert_eq!(to_of(&p, "a.TXT"), "a.md");
    let p = preview(&td, &["a.TXT"], r#"[{"type":"extension","set":"rst"}]"#);
    assert_eq!(to_of(&p, "a.TXT"), "a.rst");
    let p = preview(
        &td,
        &["a.TXT"],
        r#"[{"type":"extension","action":"remove"}]"#,
    );
    assert_eq!(to_of(&p, "a.TXT"), "a");
}

#[test]
fn rule_insert_and_remove_range() {
    let td = TempDir::new("insert");
    td.write("abcdef.txt", "x");
    let p = preview(
        &td,
        &["abcdef.txt"],
        r#"[{"type":"insert","text":"XX","at":2}]"#,
    );
    assert_eq!(to_of(&p, "abcdef.txt"), "abXXcdef.txt");
    let p = preview(
        &td,
        &["abcdef.txt"],
        r#"[{"type":"insert","text":"-v2","at":"before_ext"}]"#,
    );
    assert_eq!(to_of(&p, "abcdef.txt"), "abcdef-v2.txt");
    // index past the end appends to the stem, never panics
    let p = preview(
        &td,
        &["abcdef.txt"],
        r#"[{"type":"insert","text":"Z","at":99}]"#,
    );
    assert_eq!(to_of(&p, "abcdef.txt"), "abcdefZ.txt");
    let p = preview(
        &td,
        &["abcdef.txt"],
        r#"[{"type":"remove_range","start":1,"len":3}]"#,
    );
    assert_eq!(to_of(&p, "abcdef.txt"), "aef.txt");
    // out-of-range remove_range is a clamped no-op
    let p = preview(
        &td,
        &["abcdef.txt"],
        r#"[{"type":"remove_range","start":50,"len":10}]"#,
    );
    assert_eq!(status_of(&p, "abcdef.txt"), "unchanged");
    // char-based, not byte-based: é is one character
    td.write("aébc.txt", "x");
    let p = preview(
        &td,
        &["aébc.txt"],
        r#"[{"type":"remove_range","start":1,"len":1}]"#,
    );
    assert_eq!(to_of(&p, "aébc.txt"), "abc.txt");
}

#[test]
fn civil_date_known_values() {
    assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
    assert_eq!(civil_from_unix(951_782_400), (2000, 2, 29, 0, 0, 0)); // leap day
    assert_eq!(civil_from_unix(1_700_000_000), (2023, 11, 14, 22, 13, 20));
    assert_eq!(format_mtime("YYYYMMDD", 0), "19700101");
    assert_eq!(
        format_mtime("YYYY-MM-DD_HHmmss", 951_782_400),
        "2000-02-29_000000"
    );
}

#[test]
fn rule_mtime_prefix_uses_files_mtime() {
    let td = TempDir::new("mtime");
    td.write("report.txt", "x");
    let secs = platform::stamp(&td.0.join("report.txt"))
        .unwrap()
        .mtime_unix;
    let expected = format!("{}_report.txt", format_mtime("YYYYMMDD", secs));
    let p = preview(&td, &["report.txt"], r#"[{"type":"mtime_prefix"}]"#);
    assert_eq!(to_of(&p, "report.txt"), expected);
    assert_eq!(expected.len(), "19700101_report.txt".len());
}

#[test]
fn rules_apply_in_order() {
    let td = TempDir::new("order");
    td.write("a.txt", "x");
    // Prefix first, then replace sees the prefix.
    let p = preview(
        &td,
        &["a.txt"],
        r#"[{"type":"prefix","text":"xx"},{"type":"replace","find":"xx","replace":"yy"}]"#,
    );
    assert_eq!(to_of(&p, "a.txt"), "yya.txt");
    // Reversed order: replace runs before the prefix exists.
    let p = preview(
        &td,
        &["a.txt"],
        r#"[{"type":"replace","find":"xx","replace":"yy"},{"type":"prefix","text":"xx"}]"#,
    );
    assert_eq!(to_of(&p, "a.txt"), "xxa.txt");
}

// ------------------------------------------------------------ glob

#[test]
fn glob_pattern_expansion() {
    let td = TempDir::new("glob");
    td.write("a.txt", "1");
    td.write("b.txt", "2");
    td.write("c.md", "3");
    fs::create_dir(td.0.join("d.txt")).unwrap(); // a directory: skipped
    let plan = json::parse(&preview_json(&td.spec_pattern("*.txt", "[]")).unwrap()).unwrap();
    let froms: Vec<&str> = plan
        .get("entries")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.get("from").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(froms, vec!["a.txt", "b.txt"]);
    // ? matches exactly one character
    let plan = json::parse(&preview_json(&td.spec_pattern("?.md", "[]")).unwrap()).unwrap();
    assert_eq!(
        plan.get("summary").unwrap().get("total").unwrap().as_i64(),
        Some(1)
    );
    // glob_match unit checks
    assert!(glob_match("a*c", "abbbc", true));
    assert!(glob_match("*.TXT", "x.txt", true)); // ci flag on
    assert!(!glob_match("*.TXT", "x.txt", false));
    assert!(!glob_match("a?", "a", true));
    assert!(glob_match("*", "anything at all", true));
}

// ------------------------------------------------------------ validation & collisions

#[test]
fn invalid_names_are_flagged() {
    let td = TempDir::new("invalid");
    td.write("a.txt", "x");
    // illegal character via prefix
    let p = preview(&td, &["a.txt"], r#"[{"type":"prefix","text":"bad<"}]"#);
    assert_eq!(status_of(&p, "a.txt"), "invalid");
    assert_eq!(p.get("plan_valid").unwrap().as_bool(), Some(false));
    // reserved device name
    let p = preview(
        &td,
        &["a.txt"],
        r#"[{"type":"replace","find":"a","replace":"CON","scope":"name"}]"#,
    );
    assert_eq!(status_of(&p, "a.txt"), "invalid");
    // empty result
    let p = preview(
        &td,
        &["a.txt"],
        r#"[{"type":"replace","find":"a.txt","replace":""}]"#,
    );
    assert_eq!(status_of(&p, "a.txt"), "invalid");
    // >255 characters
    let long = "x".repeat(300);
    let rules = format!(r#"[{{"type":"prefix","text":"{long}"}}]"#);
    let p = preview(&td, &["a.txt"], &rules);
    assert_eq!(status_of(&p, "a.txt"), "invalid");
    // trailing period
    let p = preview(
        &td,
        &["a.txt"],
        r#"[{"type":"suffix","text":"."},{"type":"extension","action":"remove"}]"#,
    );
    assert_eq!(status_of(&p, "a.txt"), "invalid");
    // missing source file
    let p = preview(&td, &["ghost.txt"], "[]");
    assert_eq!(status_of(&p, "ghost.txt"), "invalid");
}

#[test]
fn collision_existing() {
    let td = TempDir::new("collide");
    td.write("a.txt", "A");
    td.write("taken.txt", "T");
    // a -> taken.txt : taken exists and is not in the plan
    let p = preview(
        &td,
        &["a.txt"],
        r#"[{"type":"replace","find":"a","replace":"taken","scope":"name"}]"#,
    );
    assert_eq!(status_of(&p, "a.txt"), "collision_existing");
    // And when the target file IS in the plan but does not move,
    // the verdict is the same (it is an obstacle, not a chain).
    td.write("x.txt", "X");
    td.write("b.txt", "B");
    let p = preview(
        &td,
        &["x.txt", "b.txt"],
        r#"[{"type":"replace","find":"x","replace":"b","scope":"name"}]"#,
    );
    assert_eq!(status_of(&p, "x.txt"), "collision_existing");
    assert_eq!(status_of(&p, "b.txt"), "unchanged");
}

/// Rules that produce a chain / a swap in ONE plan: strip the
/// leading digit, then re-number. Numbering counts in sorted
/// order of the original names, so start/step choose the targets:
/// start=2, step=1 maps 1->2, 2->3 (a chain); start=2, step=-1
/// maps 1->2, 2->1 (a swap).
const STRIP_DIGIT: &str = r#"[{"type":"remove_range","start":0,"len":1}"#;
fn renumber_rules(start: i64, step: i64) -> String {
    format!(
        "{STRIP_DIGIT},{{\"type\":\"numbering\",\"start\":{start},\"step\":{step},\"padding\":1,\"position\":\"prefix\",\"separator\":\"\"}}]"
    )
}

#[test]
fn chain_within_one_plan_is_ok_and_applies() {
    let td = TempDir::new("chainplan");
    td.write("1.txt", "ONE");
    td.write("2.txt", "TWO");
    let rules = renumber_rules(2, 1); // 1->2, 2->3
    let p = preview(&td, &["1.txt", "2.txt"], &rules);
    assert_eq!(to_of(&p, "1.txt"), "2.txt");
    assert_eq!(to_of(&p, "2.txt"), "3.txt");
    // 2.txt is 1.txt's target, but it is being renamed away.
    assert_eq!(status_of(&p, "1.txt"), "ok");
    assert_eq!(status_of(&p, "2.txt"), "ok");
    assert_eq!(p.get("plan_valid").unwrap().as_bool(), Some(true));
    let log = engine::apply_json(&td.spec(&["1.txt", "2.txt"], &rules)).unwrap();
    assert!(log.contains("\"ok\":true"), "{log}");
    assert_eq!(td.read("2.txt"), "ONE");
    assert_eq!(td.read("3.txt"), "TWO");
    assert!(!td.exists("1.txt"));
}

#[test]
fn collision_in_plan_and_case_insensitive() {
    let td = TempDir::new("inplan");
    td.write("a.txt", "A");
    td.write("b.txt", "B");
    // both -> same.txt
    let p = preview(
        &td,
        &["a.txt", "b.txt"],
        r#"[{"type":"replace","find":"a","replace":"same","scope":"name"},{"type":"replace","find":"b","replace":"same","scope":"name"}]"#,
    );
    assert_eq!(status_of(&p, "a.txt"), "collision_in_plan");
    assert_eq!(status_of(&p, "b.txt"), "collision_in_plan");
    // Case-insensitive: A.txt and a.txt targets collide when ci on.
    td.write("UP.txt", "U");
    td.write("low.txt", "L");
    let rules = r#"[{"type":"case","mode":"lower","scope":"both"}]"#;
    let spec_ci = format!(
        "{{\"dir\":\"{}\",\"files\":[\"UP.txt\"],\"rules\":{},\"case_insensitive_fs\":true}}",
        json_escape(&td.0.display().to_string()),
        rules
    );
    // UP.txt -> up.txt; no clash in-plan alone. Now map two files
    // to case-variants of one target:
    let p = preview(
        &td,
        &["UP.txt", "low.txt"],
        r#"[{"type":"replace","find":"UP","replace":"Same","scope":"name"},{"type":"replace","find":"low","replace":"SAME","scope":"name"}]"#,
    );
    assert_eq!(status_of(&p, "UP.txt"), "collision_in_plan");
    assert_eq!(status_of(&p, "low.txt"), "collision_in_plan");
    // Same plan with case_insensitive_fs:false -> targets differ.
    let spec_cs = format!(
        "{{\"dir\":\"{}\",\"files\":[\"UP.txt\",\"low.txt\"],\"rules\":{},\"case_insensitive_fs\":false}}",
        json_escape(&td.0.display().to_string()),
        r#"[{"type":"replace","find":"UP","replace":"Same","scope":"name"},{"type":"replace","find":"low","replace":"SAME","scope":"name"}]"#
    );
    let p = json::parse(&preview_json(&spec_cs).unwrap()).unwrap();
    assert_eq!(status_of(&p, "UP.txt"), "ok");
    assert_eq!(status_of(&p, "low.txt"), "ok");
    let _ = spec_ci;
}

#[test]
fn duplicate_source_flagged() {
    let td = TempDir::new("dup");
    td.write("a.txt", "A");
    let p = preview(&td, &["a.txt", "a.txt"], "[]");
    let entries = p.get("entries").unwrap().as_array().unwrap();
    assert_eq!(
        entries[0].get("status").unwrap().as_str(),
        Some("unchanged")
    );
    assert_eq!(
        entries[1].get("status").unwrap().as_str(),
        Some("duplicate_source")
    );
    assert_eq!(p.get("plan_valid").unwrap().as_bool(), Some(false));
}

// ------------------------------------------------------------ apply

#[test]
fn preview_changes_nothing() {
    let td = TempDir::new("nochange");
    td.write("a.txt", "A");
    let _ = preview(&td, &["a.txt"], r#"[{"type":"prefix","text":"z"}]"#);
    assert_eq!(td.names(), vec!["a.txt".to_string()]);
}

#[test]
fn apply_basic_and_log_shape() {
    let td = TempDir::new("apply");
    td.write("a.txt", "hello");
    td.write("b.txt", "world");
    let log_text = engine::apply_json(&td.spec(
        &["a.txt", "b.txt"],
        r#"[{"type":"numbering","padding":2,"position":"suffix","separator":"-"}]"#,
    ))
    .unwrap();
    let log = json::parse(&log_text).unwrap();
    assert_eq!(log.get("ok").unwrap().as_bool(), Some(true));
    assert_eq!(td.read("a-01.txt"), "hello");
    assert_eq!(td.read("b-02.txt"), "world");
    let applied = log.get("applied").unwrap().as_array().unwrap();
    assert_eq!(applied.len(), 2);
    assert_eq!(applied[0].get("size").unwrap().as_i64(), Some(5));
    assert!(
        applied[0]
            .get("mtime_unix_nanos")
            .unwrap()
            .as_i64()
            .unwrap()
            > 0
    );
    assert!(log
        .get("rollback_token")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("\"entries\""));
}

#[test]
fn apply_refuses_invalid_plan_and_touches_nothing() {
    let td = TempDir::new("refuse");
    td.write("a.txt", "A");
    td.write("taken.txt", "T");
    let log = json::parse(
        &engine::apply_json(&td.spec(
            &["a.txt"],
            r#"[{"type":"replace","find":"a","replace":"taken","scope":"name"}]"#,
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(log.get("ok").unwrap().as_bool(), Some(false));
    assert_eq!(log.get("refused").unwrap().as_bool(), Some(true));
    assert_eq!(td.read("a.txt"), "A");
    assert_eq!(td.read("taken.txt"), "T");
}

#[test]
fn apply_allow_partial() {
    let td = TempDir::new("partial");
    td.write("a.txt", "A");
    td.write("b.txt", "B");
    td.write("taken.txt", "T");
    let spec = format!(
        "{{\"dir\":\"{}\",\"files\":[\"a.txt\",\"b.txt\"],\"allow_partial\":true,\"rules\":[{{\"type\":\"replace\",\"find\":\"a\",\"replace\":\"taken\",\"scope\":\"name\"}},{{\"type\":\"suffix\",\"text\":\"-new\"}}]}}",
        json_escape(&td.0.display().to_string())
    );
    // a -> taken-new.txt (no collision after suffix), b -> b-new.txt
    let log = json::parse(&engine::apply_json(&spec).unwrap()).unwrap();
    assert_eq!(log.get("ok").unwrap().as_bool(), Some(true));
    assert!(td.exists("taken-new.txt"));
    // Now a true partial: one ok, one colliding.
    td.write("c.txt", "C");
    let spec2 = format!(
        "{{\"dir\":\"{}\",\"files\":[\"b-new.txt\",\"c.txt\"],\"allow_partial\":true,\"rules\":[{{\"type\":\"replace\",\"find\":\"c\",\"replace\":\"taken\",\"scope\":\"name\"}}]}}",
        json_escape(&td.0.display().to_string())
    );
    let log = json::parse(&engine::apply_json(&spec2).unwrap()).unwrap();
    assert_eq!(log.get("ok").unwrap().as_bool(), Some(true));
    assert_eq!(log.get("applied").unwrap().as_array().unwrap().len(), 0);
    assert_eq!(log.get("skipped").unwrap().as_array().unwrap().len(), 1);
    assert!(td.exists("c.txt")); // colliding entry left alone
}

#[test]
fn swap_cycle_applies_via_two_phase() {
    // A genuine A<->B swap in one plan (see renumber_rules):
    // 1.txt -> 2.txt and 2.txt -> 1.txt. Neither target is free
    // until phase 1 has moved BOTH files to temp names.
    let td = TempDir::new("swap");
    td.write("1.txt", "FIRST");
    td.write("2.txt", "SECOND");
    let rules = renumber_rules(2, -1);
    let p = preview(&td, &["1.txt", "2.txt"], &rules);
    assert_eq!(to_of(&p, "1.txt"), "2.txt");
    assert_eq!(to_of(&p, "2.txt"), "1.txt");
    assert_eq!(p.get("plan_valid").unwrap().as_bool(), Some(true));
    let log = engine::apply_json(&td.spec(&["1.txt", "2.txt"], &rules)).unwrap();
    assert!(log.contains("\"ok\":true"), "{log}");
    assert_eq!(td.read("1.txt"), "SECOND");
    assert_eq!(td.read("2.txt"), "FIRST");
    assert_eq!(td.names(), vec!["1.txt".to_string(), "2.txt".to_string()]);
}

#[test]
fn case_only_rename_is_ok_not_a_collision() {
    let td = TempDir::new("caseonly");
    td.write("ReadMe.txt", "R");
    let p = preview(
        &td,
        &["ReadMe.txt"],
        r#"[{"type":"case","mode":"lower","scope":"both"}]"#,
    );
    assert_eq!(status_of(&p, "ReadMe.txt"), "ok");
    let log = engine::apply_json(&td.spec(
        &["ReadMe.txt"],
        r#"[{"type":"case","mode":"lower","scope":"both"}]"#,
    ))
    .unwrap();
    assert!(log.contains("\"ok\":true"), "{log}");
    assert_eq!(td.read("readme.txt"), "R");
}

#[test]
fn apply_failure_rolls_back_everything() {
    let td = TempDir::new("failrollback");
    td.write("a.txt", "A");
    td.write("b.txt", "B");
    // A directory sits on b's final target: preview leaves the
    // entry ok (with a warning), phase 2 fails on it.
    fs::create_dir(td.0.join("b-new.txt")).unwrap();
    let log_text =
        engine::apply_json(&td.spec(&["a.txt", "b.txt"], r#"[{"type":"suffix","text":"-new"}]"#))
            .unwrap();
    let log = json::parse(&log_text).unwrap();
    assert_eq!(log.get("ok").unwrap().as_bool(), Some(false), "{log_text}");
    assert_eq!(
        log.get("rolled_back").unwrap().as_bool(),
        Some(true),
        "{log_text}"
    );
    // Original layout restored exactly; no temp files left behind.
    assert_eq!(td.read("a.txt"), "A");
    assert_eq!(td.read("b.txt"), "B");
    assert!(td.is_dir("b-new.txt"));
    assert_eq!(
        td.names(),
        vec![
            "a.txt".to_string(),
            "b-new.txt".to_string(),
            "b.txt".to_string()
        ]
    );
    assert!(log.get("failed").unwrap().get("from").is_some());
}

#[test]
fn rollback_from_log_restores_names() {
    let td = TempDir::new("rollback");
    td.write("a.txt", "A");
    td.write("b.txt", "BB");
    let log_text =
        engine::apply_json(&td.spec(&["a.txt", "b.txt"], r#"[{"type":"prefix","text":"new-"}]"#))
            .unwrap();
    assert!(td.exists("new-a.txt"));
    let back = json::parse(&engine::rollback_json(&log_text).unwrap()).unwrap();
    assert_eq!(back.get("ok").unwrap().as_bool(), Some(true));
    assert_eq!(back.get("restored").unwrap().as_array().unwrap().len(), 2);
    assert_eq!(td.read("a.txt"), "A");
    assert_eq!(td.read("b.txt"), "BB");
    // Rollback also works from the bare token string alone.
    let log = json::parse(&log_text).unwrap();
    let token = log
        .get("rollback_token")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    // Re-apply, then roll back using only {"rollback_token": ...}.
    let log2 =
        engine::apply_json(&td.spec(&["a.txt", "b.txt"], r#"[{"type":"prefix","text":"new-"}]"#))
            .unwrap();
    let _ = log2;
    let wrapped = format!("{{\"rollback_token\":\"{}\"}}", json_escape(&token));
    let back = json::parse(&engine::rollback_json(&wrapped).unwrap()).unwrap();
    assert_eq!(back.get("ok").unwrap().as_bool(), Some(true));
    assert_eq!(td.read("a.txt"), "A");
}

#[test]
fn rollback_refuses_file_changed_since_apply() {
    let td = TempDir::new("rollbackrefuse");
    td.write("a.txt", "A");
    td.write("b.txt", "B");
    let log_text =
        engine::apply_json(&td.spec(&["a.txt", "b.txt"], r#"[{"type":"prefix","text":"new-"}]"#))
            .unwrap();
    // Change one file after apply (size changes).
    td.write("new-a.txt", "A-CHANGED-LONGER");
    let back_text = engine::rollback_json(&log_text).unwrap();
    let back = json::parse(&back_text).unwrap();
    assert_eq!(
        back.get("ok").unwrap().as_bool(),
        Some(false),
        "{back_text}"
    );
    let refused = back.get("refused").unwrap().as_array().unwrap();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].get("to").unwrap().as_str(), Some("new-a.txt"));
    // The changed file is left alone; the untouched one rolls back.
    assert_eq!(td.read("new-a.txt"), "A-CHANGED-LONGER");
    assert_eq!(td.read("b.txt"), "B");
}

#[test]
fn rollback_refuses_when_original_name_occupied() {
    let td = TempDir::new("rollbackocc");
    td.write("a.txt", "A");
    let log_text =
        engine::apply_json(&td.spec(&["a.txt"], r#"[{"type":"prefix","text":"new-"}]"#)).unwrap();
    td.write("a.txt", "INTRUDER");
    let back = json::parse(&engine::rollback_json(&log_text).unwrap()).unwrap();
    assert_eq!(back.get("ok").unwrap().as_bool(), Some(false));
    assert_eq!(td.read("a.txt"), "INTRUDER");
    assert_eq!(td.read("new-a.txt"), "A");
}

// ------------------------------------------------------------ spec errors & FFI

#[test]
fn spec_errors_are_parse_errors() {
    let td = TempDir::new("specerr");
    td.write("a.txt", "A");
    // duplicate JSON key
    let bad = format!(
        "{{\"dir\":\"{}\",\"dir\":\"{}\",\"files\":[\"a.txt\"]}}",
        json_escape(&td.0.display().to_string()),
        json_escape(&td.0.display().to_string())
    );
    assert_eq!(preview_json(&bad).unwrap_err().0, ERR_PARSE);
    // unknown key
    let bad = format!(
        "{{\"dir\":\"{}\",\"files\":[\"a.txt\"],\"surprise\":1}}",
        json_escape(&td.0.display().to_string())
    );
    assert_eq!(preview_json(&bad).unwrap_err().0, ERR_PARSE);
    // both files and pattern
    let bad = format!(
        "{{\"dir\":\"{}\",\"files\":[\"a.txt\"],\"pattern\":\"*.txt\"}}",
        json_escape(&td.0.display().to_string())
    );
    assert_eq!(preview_json(&bad).unwrap_err().0, ERR_PARSE);
    // unknown rule type (and no regex_lite is offered)
    let bad = td.spec(&["a.txt"], r#"[{"type":"regex_lite","find":"x"}]"#);
    assert_eq!(preview_json(&bad).unwrap_err().0, ERR_PARSE);
    // path in a file name
    let bad = format!(
        "{{\"dir\":\"{}\",\"files\":[\"sub/a.txt\"]}}",
        json_escape(&td.0.display().to_string())
    );
    assert_eq!(preview_json(&bad).unwrap_err().0, ERR_PARSE);
    // missing directory
    let bad = "{\"dir\":\"/definitely/not/a/real/dir/renamekit\",\"files\":[\"a.txt\"]}";
    assert_eq!(preview_json(bad).unwrap_err().0, ERR_NOT_FOUND);
}

#[test]
fn ffi_end_to_end() {
    use std::ffi::{CStr, CString};
    let td = TempDir::new("ffi");
    td.write("a.txt", "A");
    let spec = CString::new(td.spec(&["a.txt"], r#"[{"type":"suffix","text":"-x"}]"#)).unwrap();
    let out = crate::ffi::rk_preview(spec.as_ptr());
    assert!(!out.is_null());
    let text = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
    assert!(text.contains("\"to\":\"a-x.txt\""), "{text}");
    crate::ffi::rk_free_string(out);
    // null input -> null + error code
    let out = crate::ffi::rk_preview(std::ptr::null());
    assert!(out.is_null());
    assert_eq!(crate::ffi::rk_last_error_code(), ERR_INVALID_ARG);
    // version
    let v = unsafe { CStr::from_ptr(crate::ffi::rk_version()) }
        .to_str()
        .unwrap();
    assert_eq!(v, "0.1.0");
}

#[test]
fn json_escape_still_works() {
    assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
}
