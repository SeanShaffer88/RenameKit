//! RenameKit — core logic: spec parsing, the rule pipeline, plan
//! building (preview + validation + collision detection). The
//! apply/rollback engine lives in `engine`; filesystem primitives
//! in `platform`; the C ABI in `ffi`. Zero dependencies.

pub mod engine;
pub mod ffi;
pub mod json;
pub mod platform;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub const VERSION: &str = "0.1.0";
pub const VERSION_MAJOR: u32 = 0;
pub const VERSION_MINOR: u32 = 1;
pub const VERSION_PATCH: u32 = 0;

// Error codes (mirror include/renamekit.h).
pub const RK_OK: i32 = 0;
pub const ERR_INVALID_ARG: i32 = 1;
pub const ERR_NOT_SUPPORTED: i32 = 2;
pub const ERR_NOT_FOUND: i32 = 3;
pub const ERR_ACCESS_DENIED: i32 = 4;
pub const ERR_PLATFORM: i32 = 5;
pub const ERR_INTERNAL: i32 = 6;
pub const ERR_PARSE: i32 = 7;

/// Escape a string for inclusion inside JSON double quotes.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn esc(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

fn err<T>(code: i32, msg: impl Into<String>) -> Result<T, (i32, String)> {
    Err((code, msg.into()))
}

// ---------------------------------------------------------------- spec

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Name,
    Ext,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseMode {
    Upper,
    Lower,
    Title,
    Sentence,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExtAction {
    Set(String),
    Remove,
    Lower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertAt {
    Index(usize),
    BeforeExt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberPos {
    Prefix,
    Suffix,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    Replace {
        find: String,
        replace: String,
        scope: Scope,
        match_case: bool,
    },
    Prefix {
        text: String,
    },
    Suffix {
        text: String,
    },
    Case {
        mode: CaseMode,
        scope: Scope,
    },
    Trim {
        spaces: bool,
        underscores: bool,
    },
    Numbering {
        start: i64,
        step: i64,
        padding: usize,
        position: NumberPos,
        separator: String,
    },
    Extension(ExtAction),
    Insert {
        text: String,
        at: InsertAt,
    },
    RemoveRange {
        start: usize,
        len: usize,
    },
    MtimePrefix {
        format: String,
        separator: String,
    },
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub dir: String,
    /// Explicit source names, in caller's order (mutually exclusive
    /// with `pattern`).
    pub files: Option<Vec<String>>,
    pub pattern: Option<String>,
    pub rules: Vec<Rule>,
    pub case_insensitive_fs: bool,
    pub allow_partial: bool,
}

impl Spec {
    pub fn dir_path(&self) -> PathBuf {
        PathBuf::from(&self.dir)
    }
    pub fn needs_mtime(&self) -> bool {
        self.rules
            .iter()
            .any(|r| matches!(r, Rule::MtimePrefix { .. }))
    }
}

fn check_keys(v: &json::Value, allowed: &[&str], where_: &str) -> Result<(), (i32, String)> {
    for k in v.keys() {
        if !allowed.contains(&k) {
            return err(ERR_PARSE, format!("{where_}: unknown key \"{k}\""));
        }
    }
    Ok(())
}

fn req_str<'a>(v: &'a json::Value, key: &str, where_: &str) -> Result<&'a str, (i32, String)> {
    match v.get(key) {
        Some(json::Value::Str(s)) => Ok(s),
        Some(other) => err(
            ERR_PARSE,
            format!(
                "{where_}: \"{key}\" must be a string, got {}",
                other.type_name()
            ),
        ),
        None => err(ERR_PARSE, format!("{where_}: missing required \"{key}\"")),
    }
}

fn opt_str<'a>(
    v: &'a json::Value,
    key: &str,
    default: &'a str,
    where_: &str,
) -> Result<&'a str, (i32, String)> {
    match v.get(key) {
        Some(json::Value::Str(s)) => Ok(s),
        Some(_) => err(ERR_PARSE, format!("{where_}: \"{key}\" must be a string")),
        None => Ok(default),
    }
}

fn opt_bool(
    v: &json::Value,
    key: &str,
    default: bool,
    where_: &str,
) -> Result<bool, (i32, String)> {
    match v.get(key) {
        Some(json::Value::Bool(b)) => Ok(*b),
        Some(_) => err(ERR_PARSE, format!("{where_}: \"{key}\" must be a boolean")),
        None => Ok(default),
    }
}

fn opt_i64(v: &json::Value, key: &str, default: i64, where_: &str) -> Result<i64, (i32, String)> {
    match v.get(key) {
        Some(x) => x
            .as_i64()
            .ok_or_else(|| (ERR_PARSE, format!("{where_}: \"{key}\" must be an integer"))),
        None => Ok(default),
    }
}

fn parse_scope(v: &json::Value, where_: &str, default: Scope) -> Result<Scope, (i32, String)> {
    match v.get("scope") {
        None => Ok(default),
        Some(json::Value::Str(s)) => match s.as_str() {
            "name" => Ok(Scope::Name),
            "ext" => Ok(Scope::Ext),
            "both" => Ok(Scope::Both),
            other => err(
                ERR_PARSE,
                format!("{where_}: \"scope\" must be name|ext|both, got \"{other}\""),
            ),
        },
        Some(_) => err(ERR_PARSE, format!("{where_}: \"scope\" must be a string")),
    }
}

fn parse_rule(v: &json::Value, idx: usize) -> Result<Rule, (i32, String)> {
    let where_ = format!("rules[{idx}]");
    if v.as_object().is_none() {
        return err(ERR_PARSE, format!("{where_}: a rule must be an object"));
    }
    let ty = req_str(v, "type", &where_)?;
    match ty {
        "replace" => {
            check_keys(
                v,
                &["type", "find", "replace", "scope", "match_case"],
                &where_,
            )?;
            let find = req_str(v, "find", &where_)?;
            if find.is_empty() {
                return err(ERR_PARSE, format!("{where_}: \"find\" must not be empty"));
            }
            Ok(Rule::Replace {
                find: find.to_string(),
                replace: opt_str(v, "replace", "", &where_)?.to_string(),
                scope: parse_scope(v, &where_, Scope::Both)?,
                match_case: opt_bool(v, "match_case", true, &where_)?,
            })
        }
        "prefix" => {
            check_keys(v, &["type", "text"], &where_)?;
            Ok(Rule::Prefix {
                text: req_str(v, "text", &where_)?.to_string(),
            })
        }
        "suffix" => {
            check_keys(v, &["type", "text"], &where_)?;
            Ok(Rule::Suffix {
                text: req_str(v, "text", &where_)?.to_string(),
            })
        }
        "case" => {
            check_keys(v, &["type", "mode", "scope"], &where_)?;
            let mode = match req_str(v, "mode", &where_)? {
                "upper" => CaseMode::Upper,
                "lower" => CaseMode::Lower,
                "title" => CaseMode::Title,
                "sentence" => CaseMode::Sentence,
                other => {
                    return err(
                        ERR_PARSE,
                        format!(
                            "{where_}: \"mode\" must be upper|lower|title|sentence, got \"{other}\""
                        ),
                    )
                }
            };
            Ok(Rule::Case {
                mode,
                scope: parse_scope(v, &where_, Scope::Name)?,
            })
        }
        "trim" => {
            check_keys(v, &["type", "spaces", "underscores"], &where_)?;
            Ok(Rule::Trim {
                spaces: opt_bool(v, "spaces", true, &where_)?,
                underscores: opt_bool(v, "underscores", false, &where_)?,
            })
        }
        "numbering" => {
            check_keys(
                v,
                &["type", "start", "step", "padding", "position", "separator"],
                &where_,
            )?;
            let padding = opt_i64(v, "padding", 3, &where_)?;
            if !(0..=32).contains(&padding) {
                return err(ERR_PARSE, format!("{where_}: \"padding\" must be 0..=32"));
            }
            let position = match opt_str(v, "position", "suffix", &where_)? {
                "prefix" => NumberPos::Prefix,
                "suffix" => NumberPos::Suffix,
                other => {
                    return err(
                        ERR_PARSE,
                        format!("{where_}: \"position\" must be prefix|suffix, got \"{other}\""),
                    )
                }
            };
            Ok(Rule::Numbering {
                start: opt_i64(v, "start", 1, &where_)?,
                step: opt_i64(v, "step", 1, &where_)?,
                padding: padding as usize,
                position,
                separator: opt_str(v, "separator", "_", &where_)?.to_string(),
            })
        }
        "extension" => {
            check_keys(v, &["type", "action", "value", "set"], &where_)?;
            if let Some(s) = v.get("set") {
                let val = s
                    .as_str()
                    .ok_or_else(|| (ERR_PARSE, format!("{where_}: \"set\" must be a string")))?;
                return Ok(Rule::Extension(ExtAction::Set(clean_ext(val, &where_)?)));
            }
            match req_str(v, "action", &where_)? {
                "set" => Ok(Rule::Extension(ExtAction::Set(clean_ext(
                    req_str(v, "value", &where_)?,
                    &where_,
                )?))),
                "remove" => Ok(Rule::Extension(ExtAction::Remove)),
                "lower" => Ok(Rule::Extension(ExtAction::Lower)),
                other => err(
                    ERR_PARSE,
                    format!("{where_}: \"action\" must be set|remove|lower, got \"{other}\""),
                ),
            }
        }
        "insert" => {
            check_keys(v, &["type", "text", "at"], &where_)?;
            let at = match v.get("at") {
                Some(json::Value::Str(s)) if s == "before_ext" => InsertAt::BeforeExt,
                Some(json::Value::Str(s)) => {
                    return err(
                        ERR_PARSE,
                        format!("{where_}: \"at\" as a string must be \"before_ext\", got \"{s}\""),
                    )
                }
                Some(x) => match x.as_i64() {
                    Some(n) if n >= 0 => InsertAt::Index(n as usize),
                    _ => {
                        return err(
                            ERR_PARSE,
                            format!(
                                "{where_}: \"at\" must be a non-negative index or \"before_ext\""
                            ),
                        )
                    }
                },
                None => InsertAt::BeforeExt,
            };
            Ok(Rule::Insert {
                text: req_str(v, "text", &where_)?.to_string(),
                at,
            })
        }
        "remove_range" => {
            check_keys(v, &["type", "start", "len"], &where_)?;
            let start = opt_i64(v, "start", -1, &where_)?;
            let len = opt_i64(v, "len", -1, &where_)?;
            if start < 0 || len < 0 {
                return err(
                    ERR_PARSE,
                    format!("{where_}: \"start\" and \"len\" are required and must be >= 0"),
                );
            }
            Ok(Rule::RemoveRange {
                start: start as usize,
                len: len as usize,
            })
        }
        "mtime_prefix" => {
            check_keys(v, &["type", "format", "separator"], &where_)?;
            Ok(Rule::MtimePrefix {
                format: opt_str(v, "format", "YYYYMMDD", &where_)?.to_string(),
                separator: opt_str(v, "separator", "_", &where_)?.to_string(),
            })
        }
        other => err(
            ERR_PARSE,
            format!("{where_}: unknown rule type \"{other}\""),
        ),
    }
}

fn clean_ext(v: &str, where_: &str) -> Result<String, (i32, String)> {
    let s = v.strip_prefix('.').unwrap_or(v);
    if s.is_empty() {
        return err(
            ERR_PARSE,
            format!("{where_}: extension value must not be empty"),
        );
    }
    Ok(s.to_string())
}

/// Parse a spec JSON document. Structural problems (bad JSON,
/// duplicate keys, unknown keys, wrong types, both `files` and
/// `pattern`, a source name containing a path separator) are
/// errors here; per-file problems are reported per entry in the
/// plan instead.
pub fn parse_spec(text: &str) -> Result<Spec, (i32, String)> {
    let v = json::parse(text).map_err(|e| (ERR_PARSE, format!("spec is not valid JSON: {e}")))?;
    if v.as_object().is_none() {
        return err(ERR_PARSE, "spec must be a JSON object");
    }
    check_keys(
        &v,
        &[
            "dir",
            "files",
            "pattern",
            "rules",
            "case_insensitive_fs",
            "allow_partial",
        ],
        "spec",
    )?;
    let dir = req_str(&v, "dir", "spec")?;
    if dir.is_empty() {
        return err(ERR_PARSE, "spec: \"dir\" must not be empty");
    }
    let files = match v.get("files") {
        Some(json::Value::Array(items)) => {
            let mut out = Vec::new();
            for (i, it) in items.iter().enumerate() {
                let s = it
                    .as_str()
                    .ok_or_else(|| (ERR_PARSE, format!("spec: files[{i}] must be a string")))?;
                if s.is_empty() || s.contains('/') || s.contains('\\') || s == "." || s == ".." {
                    return err(
                        ERR_PARSE,
                        format!("spec: files[{i}] \"{s}\" is not a plain file name"),
                    );
                }
                out.push(s.to_string());
            }
            Some(out)
        }
        Some(_) => return err(ERR_PARSE, "spec: \"files\" must be an array of strings"),
        None => None,
    };
    let pattern = match v.get("pattern") {
        Some(json::Value::Str(s)) => {
            if s.is_empty() || s.contains('/') || s.contains('\\') {
                return err(ERR_PARSE, "spec: \"pattern\" must be a plain name pattern");
            }
            Some(s.clone())
        }
        Some(_) => return err(ERR_PARSE, "spec: \"pattern\" must be a string"),
        None => None,
    };
    if files.is_some() == pattern.is_some() {
        return err(
            ERR_PARSE,
            "spec: give exactly one of \"files\" or \"pattern\"",
        );
    }
    let mut rules = Vec::new();
    match v.get("rules") {
        Some(json::Value::Array(items)) => {
            for (i, it) in items.iter().enumerate() {
                rules.push(parse_rule(it, i)?);
            }
        }
        Some(_) => return err(ERR_PARSE, "spec: \"rules\" must be an array"),
        None => {}
    }
    Ok(Spec {
        dir: dir.to_string(),
        files,
        pattern,
        rules,
        case_insensitive_fs: opt_bool(&v, "case_insensitive_fs", true, "spec")?,
        allow_partial: opt_bool(&v, "allow_partial", false, "spec")?,
    })
}

// ---------------------------------------------------------------- names

/// Split a file name into (stem, extension-without-dot). A leading
/// dot does not start an extension (".gitignore" is all stem); the
/// last dot otherwise does ("archive.tar.gz" -> "archive.tar","gz").
pub fn split_name(name: &str) -> (String, Option<String>) {
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), Some(name[i + 1..].to_string())),
        _ => (name.to_string(), None),
    }
}

pub fn join_name(stem: &str, ext: &Option<String>) -> String {
    match ext {
        Some(e) => format!("{stem}.{e}"),
        None => stem.to_string(),
    }
}

/// Comparison key honoring the case-insensitive-filesystem flag.
pub fn norm(s: &str, ci: bool) -> String {
    if ci {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

fn chars_eq(a: char, b: char, ci: bool) -> bool {
    a == b || (ci && a.to_lowercase().eq(b.to_lowercase()))
}

/// Simple `*` / `?` glob over one file name (no path separators).
/// `*` matches any run of characters (including empty), `?`
/// exactly one character. Matching is over Unicode characters.
pub fn glob_match(pattern: &str, name: &str, ci: bool) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    // Classic two-pointer wildcard match with star backtracking.
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut star_n) = (usize::MAX, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || chars_eq(p[pi], n[ni], ci)) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            star_n = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            star_n += 1;
            ni = star_n;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// ---------------------------------------------------------------- rules

fn lit_replace(hay: &str, find: &str, repl: &str, match_case: bool) -> String {
    if match_case {
        return hay.replace(find, repl);
    }
    let h: Vec<char> = hay.chars().collect();
    let f: Vec<char> = find.chars().collect();
    if f.is_empty() || f.len() > h.len() {
        return hay.to_string();
    }
    let mut out = String::new();
    let mut i = 0;
    while i < h.len() {
        if i + f.len() <= h.len()
            && (0..f.len()).all(|k| h[i + k].to_lowercase().eq(f[k].to_lowercase()))
        {
            out.push_str(repl);
            i += f.len();
        } else {
            out.push(h[i]);
            i += 1;
        }
    }
    out
}

fn apply_case(s: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Upper => s.to_uppercase(),
        CaseMode::Lower => s.to_lowercase(),
        CaseMode::Title => {
            let mut out = String::with_capacity(s.len());
            let mut word_start = true;
            for c in s.chars() {
                if matches!(c, ' ' | '_' | '-' | '.') {
                    word_start = true;
                    out.push(c);
                } else if word_start {
                    out.extend(c.to_uppercase());
                    word_start = false;
                } else {
                    out.extend(c.to_lowercase());
                }
            }
            out
        }
        CaseMode::Sentence => {
            let lower = s.to_lowercase();
            let mut out = String::with_capacity(lower.len());
            let mut done = false;
            for c in lower.chars() {
                if !done && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                    done = true;
                } else {
                    out.push(c);
                }
            }
            out
        }
    }
}

fn format_number(n: i128, padding: usize) -> String {
    let s = n.to_string();
    if padding == 0 || s.len() >= padding {
        return s;
    }
    let zeros = "0".repeat(padding - s.len());
    if let Some(rest) = s.strip_prefix('-') {
        format!("-{zeros}{rest}")
    } else {
        format!("{zeros}{s}")
    }
}

/// (year, month, day, hour, minute, second) in UTC for a Unix
/// timestamp. Civil-date conversion is the standard
/// days-from-epoch algorithm (Howard Hinnant's), zero-dep.
pub fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // civil_from_days: shift epoch to 0000-03-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (
        year,
        m,
        d,
        (tod / 3600) as u32,
        ((tod % 3600) / 60) as u32,
        (tod % 60) as u32,
    )
}

/// Render an mtime with a format string. Tokens `YYYY MM DD HH mm
/// ss` are replaced (zero-padded); every other character is
/// literal. Longest tokens are matched first, left to right.
pub fn format_mtime(format: &str, secs: i64) -> String {
    let (y, mo, d, h, mi, s) = civil_from_unix(secs);
    let tokens: [(&str, String); 6] = [
        ("YYYY", format!("{y:04}")),
        ("MM", format!("{mo:02}")),
        ("DD", format!("{d:02}")),
        ("HH", format!("{h:02}")),
        ("mm", format!("{mi:02}")),
        ("ss", format!("{s:02}")),
    ];
    let mut out = String::new();
    let mut rest = format;
    while !rest.is_empty() {
        let mut matched = false;
        for (tok, val) in &tokens {
            if let Some(r) = rest.strip_prefix(tok) {
                out.push_str(val);
                rest = r;
                matched = true;
                break;
            }
        }
        if !matched {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

struct Parts {
    stem: String,
    ext: Option<String>,
}

/// Apply every rule, in order, to one source name. `rank` is the
/// source's position in sorted order of the original names (the
/// numbering rule counts in that order, not in listing order).
/// `mtime` is the source's modification time when known.
fn apply_rules(spec: &Spec, name: &str, rank: usize, mtime: Option<i64>) -> (String, Vec<String>) {
    let (stem, ext) = split_name(name);
    let mut p = Parts { stem, ext };
    let mut issues = Vec::new();
    for rule in &spec.rules {
        match rule {
            Rule::Replace {
                find,
                replace,
                scope,
                match_case,
            } => match scope {
                Scope::Name => p.stem = lit_replace(&p.stem, find, replace, *match_case),
                Scope::Ext => {
                    if let Some(e) = &p.ext {
                        p.ext = Some(lit_replace(e, find, replace, *match_case));
                    }
                }
                Scope::Both => {
                    let whole =
                        lit_replace(&join_name(&p.stem, &p.ext), find, replace, *match_case);
                    let (s, e) = split_name(&whole);
                    p.stem = s;
                    p.ext = e;
                }
            },
            Rule::Prefix { text } => p.stem = format!("{text}{}", p.stem),
            Rule::Suffix { text } => p.stem = format!("{}{text}", p.stem),
            Rule::Case { mode, scope } => match scope {
                Scope::Name => p.stem = apply_case(&p.stem, *mode),
                Scope::Ext => {
                    if let Some(e) = &p.ext {
                        p.ext = Some(apply_case(e, *mode));
                    }
                }
                Scope::Both => {
                    p.stem = apply_case(&p.stem, *mode);
                    if let Some(e) = &p.ext {
                        p.ext = Some(apply_case(e, *mode));
                    }
                }
            },
            Rule::Trim {
                spaces,
                underscores,
            } => {
                let is_trim =
                    |c: char| (*spaces && (c == ' ' || c == '\t')) || (*underscores && c == '_');
                p.stem = p.stem.trim_matches(is_trim).to_string();
            }
            Rule::Numbering {
                start,
                step,
                padding,
                position,
                separator,
            } => {
                let n = *start as i128 + *step as i128 * rank as i128;
                let num = format_number(n, *padding);
                match position {
                    NumberPos::Prefix => p.stem = format!("{num}{separator}{}", p.stem),
                    NumberPos::Suffix => p.stem = format!("{}{separator}{num}", p.stem),
                }
            }
            Rule::Extension(action) => match action {
                ExtAction::Set(v) => p.ext = Some(v.clone()),
                ExtAction::Remove => p.ext = None,
                ExtAction::Lower => {
                    if let Some(e) = &p.ext {
                        p.ext = Some(e.to_lowercase());
                    }
                }
            },
            Rule::Insert { text, at } => match at {
                InsertAt::BeforeExt => p.stem.push_str(text),
                InsertAt::Index(idx) => {
                    let mut chars: Vec<char> = p.stem.chars().collect();
                    let at = (*idx).min(chars.len());
                    let tail: Vec<char> = chars.split_off(at);
                    chars.extend(text.chars());
                    chars.extend(tail);
                    p.stem = chars.into_iter().collect();
                }
            },
            Rule::RemoveRange { start, len } => {
                let chars: Vec<char> = p.stem.chars().collect();
                let start = (*start).min(chars.len());
                let end = start.saturating_add(*len).min(chars.len());
                p.stem = chars[..start].iter().chain(chars[end..].iter()).collect();
            }
            Rule::MtimePrefix { format, separator } => match mtime {
                Some(secs) => {
                    let stamp = format_mtime(format, secs);
                    p.stem = format!("{stamp}{separator}{}", p.stem);
                }
                None => issues.push(
                    "could not read the file's modification time for mtime_prefix".to_string(),
                ),
            },
        }
    }
    (join_name(&p.stem, &p.ext), issues)
}

// ---------------------------------------------------------------- plan

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Unchanged,
    CollisionExisting,
    CollisionInPlan,
    Invalid,
    DuplicateSource,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Unchanged => "unchanged",
            Status::CollisionExisting => "collision_existing",
            Status::CollisionInPlan => "collision_in_plan",
            Status::Invalid => "invalid",
            Status::DuplicateSource => "duplicate_source",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub from: String,
    pub to: String,
    pub full_from: String,
    pub full_to: String,
    pub status: Status,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub dir: String,
    pub entries: Vec<Entry>,
    pub plan_valid: bool,
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Windows name rules, reported in plain words. Returns one issue
/// per problem found (empty vec = the name is legal).
pub fn validate_name(name: &str) -> Vec<String> {
    let mut issues = Vec::new();
    if name.is_empty() {
        issues.push("the resulting name is empty".to_string());
        return issues;
    }
    if name.chars().count() > 255 {
        issues.push(format!(
            "the resulting name is {} characters; the limit is 255",
            name.chars().count()
        ));
    }
    for c in name.chars() {
        if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20 {
            issues.push(format!(
                "the resulting name contains '{c}', which Windows does not allow in a file name"
            ));
            break;
        }
    }
    if name.ends_with(' ') || name.ends_with('.') {
        issues.push(
            "the resulting name ends with a space or a period, which Windows does not allow"
                .to_string(),
        );
    }
    if name == "." || name == ".." {
        issues.push("the resulting name is a relative-path marker, not a file name".to_string());
    }
    // Reserved device names: the part before the first dot decides.
    let head = name.split('.').next().unwrap_or("");
    if RESERVED.contains(&head.to_uppercase().as_str()) {
        issues.push(format!(
            "\"{head}\" is a reserved Windows device name and cannot be used as a file name"
        ));
    }
    issues
}

/// Resolve the spec's source list (explicit files, or a glob over
/// the directory's files in sorted order).
fn resolve_sources(spec: &Spec) -> Result<Vec<String>, (i32, String)> {
    if let Some(files) = &spec.files {
        return Ok(files.clone());
    }
    let pattern = spec.pattern.as_deref().unwrap_or("");
    let dir = spec.dir_path();
    let mut out = Vec::new();
    for name in platform::list_names(&dir)? {
        if !dir.join(&name).is_file() {
            continue;
        }
        if glob_match(pattern, &name, spec.case_insensitive_fs) {
            out.push(name);
        }
    }
    Ok(out)
}

/// Build the full plan: apply the rules to every source, validate
/// every result, and detect duplicates and collisions. Reads the
/// directory but never changes anything on disk.
pub fn build_plan(spec: &Spec) -> Result<Plan, (i32, String)> {
    let dir = spec.dir_path();
    if !dir.is_dir() {
        return err(
            ERR_NOT_FOUND,
            format!(
                "directory \"{}\" does not exist or is not a directory",
                spec.dir
            ),
        );
    }
    let sources = resolve_sources(spec)?;
    let ci = spec.case_insensitive_fs;

    // Numbering rank: position in sorted order of the ORIGINAL
    // names (case-insensitive key, original as tie-break).
    let mut order: Vec<usize> = (0..sources.len()).collect();
    order.sort_by(|&a, &b| {
        norm(&sources[a], true)
            .cmp(&norm(&sources[b], true))
            .then_with(|| sources[a].cmp(&sources[b]))
    });
    let mut rank = vec![0usize; sources.len()];
    for (r, idx) in order.iter().enumerate() {
        rank[*idx] = r;
    }

    let need_mtime = spec.needs_mtime();
    let mut entries: Vec<Entry> = Vec::new();
    for (i, from) in sources.iter().enumerate() {
        let full_from = dir.join(from);
        let mtime = if need_mtime {
            platform::stamp(&full_from).ok().map(|s| s.mtime_unix)
        } else {
            None
        };
        let (to, mut issues) = apply_rules(spec, from, rank[i], mtime);
        let mut invalid = !issues.is_empty();
        // Source must be an existing file.
        if !full_from.exists() {
            issues.push(format!("source file \"{from}\" does not exist"));
            invalid = true;
        } else if !full_from.is_file() {
            issues.push(format!("source \"{from}\" is not a file"));
            invalid = true;
        }
        let name_issues = validate_name(&to);
        if !name_issues.is_empty() {
            invalid = true;
            issues.extend(name_issues);
        }
        entries.push(Entry {
            full_to: dir.join(&to).display().to_string(),
            full_from: full_from.display().to_string(),
            from: from.clone(),
            to,
            status: if invalid { Status::Invalid } else { Status::Ok },
            issues,
        });
    }

    // Duplicate sources (same name listed twice): the first is
    // planned normally, later copies are duplicate_source.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, e) in entries.iter_mut().enumerate() {
        let key = norm(&e.from, ci);
        match seen.entry(key) {
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(i);
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                e.status = Status::DuplicateSource;
                e.issues.push(format!(
                    "\"{}\" is listed more than once in the source list",
                    e.from
                ));
            }
        }
    }

    // Unchanged, then disk collisions, then in-plan collisions.
    for e in entries.iter_mut() {
        if e.status == Status::Ok && e.to == e.from {
            e.status = Status::Unchanged;
        }
    }
    // Names of sources that actually move away (their old name is
    // freed by this plan — the chain case A->B, B->C).
    let changing: HashSet<String> = entries
        .iter()
        .filter(|e| e.status == Status::Ok)
        .map(|e| norm(&e.from, ci))
        .collect();
    let disk: HashMap<String, String> = platform::list_names(&dir)?
        .into_iter()
        .map(|n| (norm(&n, ci), n))
        .collect();
    for e in entries.iter_mut() {
        if e.status != Status::Ok {
            continue;
        }
        if let Some(actual) = disk.get(&norm(&e.to, ci)) {
            if norm(actual, ci) == norm(&e.from, ci) {
                continue; // its own file (includes case-only renames)
            }
            let actual_path = dir.join(actual);
            if actual_path.is_dir() {
                e.issues.push(format!(
                    "the target \"{}\" is an existing directory; apply will fail on it and roll back",
                    e.to
                ));
                continue;
            }
            if changing.contains(&norm(actual, ci)) {
                continue; // that file is being renamed away by this plan
            }
            e.status = Status::CollisionExisting;
            e.issues.push(format!(
                "a file named \"{}\" already exists and is not being renamed by this plan",
                e.to
            ));
        }
    }
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if matches!(e.status, Status::Ok | Status::Unchanged) {
            groups.entry(norm(&e.to, ci)).or_default().push(i);
        }
    }
    for idxs in groups.values() {
        if idxs.len() < 2 {
            continue;
        }
        let any_changing = idxs.iter().any(|&i| entries[i].status == Status::Ok);
        for &i in idxs {
            if any_changing && entries[i].status != Status::Ok {
                continue; // the unchanged entry is the obstacle, not a collider
            }
            entries[i].status = Status::CollisionInPlan;
            let to = entries[i].to.clone();
            entries[i].issues.push(format!(
                "more than one file in this plan renames to \"{to}\""
            ));
        }
    }

    let plan_valid = entries
        .iter()
        .all(|e| matches!(e.status, Status::Ok | Status::Unchanged));
    Ok(Plan {
        dir: spec.dir.clone(),
        entries,
        plan_valid,
    })
}

pub fn plan_to_json(plan: &Plan) -> String {
    let count = |s: Status| plan.entries.iter().filter(|e| e.status == s).count();
    let mut out = String::from("{\"dir\":");
    out.push_str(&esc(&plan.dir));
    out.push_str(",\"entries\":[");
    for (i, e) in plan.entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"from\":");
        out.push_str(&esc(&e.from));
        out.push_str(",\"to\":");
        out.push_str(&esc(&e.to));
        out.push_str(",\"full_from\":");
        out.push_str(&esc(&e.full_from));
        out.push_str(",\"full_to\":");
        out.push_str(&esc(&e.full_to));
        out.push_str(",\"status\":");
        out.push_str(&esc(e.status.as_str()));
        out.push_str(",\"issues\":[");
        for (j, iss) in e.issues.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&esc(iss));
        }
        out.push_str("]}");
    }
    out.push_str("],\"summary\":{\"total\":");
    out.push_str(&plan.entries.len().to_string());
    out.push_str(",\"ok\":");
    out.push_str(&count(Status::Ok).to_string());
    out.push_str(",\"unchanged\":");
    out.push_str(&count(Status::Unchanged).to_string());
    out.push_str(",\"collision_existing\":");
    out.push_str(&count(Status::CollisionExisting).to_string());
    out.push_str(",\"collision_in_plan\":");
    out.push_str(&count(Status::CollisionInPlan).to_string());
    out.push_str(",\"invalid\":");
    out.push_str(&count(Status::Invalid).to_string());
    out.push_str(",\"duplicate_source\":");
    out.push_str(&count(Status::DuplicateSource).to_string());
    out.push_str("},\"plan_valid\":");
    out.push_str(if plan.plan_valid { "true" } else { "false" });
    out.push('}');
    out
}

/// `rk_preview`'s work: parse the spec, build the plan, return it
/// as JSON. Never touches the files themselves.
pub fn preview_json(spec_text: &str) -> Result<String, (i32, String)> {
    let spec = parse_spec(spec_text)?;
    let plan = build_plan(&spec)?;
    Ok(plan_to_json(&plan))
}
