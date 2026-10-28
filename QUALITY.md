# RenameKit — Quality Scorecard

Scored under The Elite Software Master Prompt v1.1, §17. Every score names its evidence; a score without evidence would be a Fail. This file is updated in the same change as the work it scores.

**State at v0.1.0: total 82/100 — below the release bar (total ≥ 85, no dimension below 3, no Fail).** The gap is named honestly: the kit is gate-green and fully tested on the Linux host, but it has never run on a real Windows machine, and that single fact caps Correctness, Robustness and UX until the owner's on-machine pass. Nothing in this scorecard claims otherwise.

| Dimension | Score (0–5) | Verdict | Evidence |
|---|---|---|---|
| Correctness (weight 14) | 4 | Waiver | Full WinDevKit gate green locally (fmt + clippy both targets + full suite); 33/33 tests pass, including a one-plan swap and chain, a forced phase-2 failure with full rollback, and rollback refusals. Waiver: Windows runtime behavior is Unverified — no on-machine pass yet; follow-up is ROADMAP item 1. |
| Security & privacy (12) | 4 | Pass | Zero dependencies, no network, no telemetry; spec input is bounded (JSON depth cap 128, duplicate keys rejected, unknown keys rejected, plain-names-only sources so a spec cannot point outside `dir`); rollback refuses files changed since apply. No secrets handled anywhere. |
| Robustness (11) | 4 | Waiver | Two-phase apply with two-phase rollback on failure, tested with an engineered phase-2 failure (directory on a target) — original layout restored, no temp files left. All FFI entry points are catch_unwind-wrapped. Waiver: failure drills beyond that one engineered case (locked file on Windows, disk full) are Unverified until the on-machine pass. |
| Feature completeness (9) | 5 | Pass | Every PARITY.md row is Complete, Deferred, or Rejected with reason; the one deliberate rejection (regex_lite) is a refusal to fake a capability, recorded there and in the header. |
| Performance (8) | 3 | Waiver | Design is linear in the number of files plus one directory listing reused for all collision checks; no benchmarks recorded. Waiver: no measured budget exists for very large folders (10k+ files); follow-up — measure at the on-machine pass before any performance claim is made. |
| Maintainability (7) | 4 | Pass | Small module split (json / lib planning / engine / platform / ffi, each single-responsibility), AGENTS.md records the load-bearing contracts (two-phase, chain set ordering, numbering rank), ROADMAP and PARITY current. |
| Modularity (6) | 5 | Pass | Planning is pure with respect to the rule pipeline and separated from the engine: preview and apply share `build_plan`, so they can never disagree about validity; the C ABI is a thin wrapper over `preview_json` / `apply_json` / `rollback_json`, all directly testable from Rust. |
| UX & accessibility (6) | 3 | Waiver | This is a library, not a window: its user surface is the JSON and the plain-word issue messages ("a file named X already exists…", "changed size since apply"). Waiver: no host app has consumed those messages yet, so their clarity in a real UI is Unverified. |
| Scalability (5) | 3 | Waiver | Bounded inputs (JSON depth cap, padding cap, one listing held in memory). Waiver: no 10x exercise recorded; a 10k-file plan is the follow-up drill alongside Performance. |
| Idiomatic quality (5) | 4 | Pass | rustfmt-clean and clippy-clean with `-D warnings` on host and windows-msvc targets via the family gate; no unsafe code outside the FFI boundary wrappers. |
| Test & corpus evidence (5) | 4 | Pass | 33 tests, all against real temporary directories and real files — no mocks of the filesystem the kit exists to rename on. Known-value vectors for the civil-date conversion (epoch, 2000-02-29 leap day). Coverage percentage is not measured by tooling; test-count + gate evidence is the recorded substitute. |
| Self-documentation (4) | 4 | Pass | The header documents every export, every rule field and default, both JSON shapes, and every status value; module docs state each module's one job and its contracts. |
| Documentation (4) | 4 | Pass | README with TL;DR, a C example, honest limitations (including "extension change does not convert content" and the stamp-check caveat), build and test instructions; ROADMAP names what is not planned and why. |
| Diagnostics & supportability (4) | 4 | Pass | Thread-local last-error with family error codes on every hard failure; apply/rollback logs are self-describing JSON (applied / failed / refused / skipped / rollback_errors), so a host can show or store exactly what happened. |

Weighted total: **82/100** (Correctness 11.2, Security 9.6, Robustness 8.8, Completeness 9.0, Performance 4.8, Maintainability 5.6, Modularity 6.0, UX 3.6, Scalability 3.0, Idiomatic 4.0, Test evidence 4.0, Self-documentation 3.2, Documentation 3.2, Diagnostics 3.2).

## What moves the score

1. The on-machine Windows pass (ROADMAP item 1) — closes the Correctness and Robustness waivers and is the single biggest step: expected to lift the total past the release bar.
2. A recorded large-folder run (10k files: preview + apply timings) — closes the Performance and Scalability waivers.
3. A host app consuming the plan/log JSON in a real UI — closes the UX waiver.
