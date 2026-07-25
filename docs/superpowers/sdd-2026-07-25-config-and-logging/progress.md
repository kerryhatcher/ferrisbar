# SDD ledger — plan: docs/superpowers/plans/2026-07-25-config-and-logging-phase-1.md

Branch: feat/config-and-logging
Pre-flight scan: one conflict found and resolved before Task 1 — Task 8's
`let _ = &mut cfg;` placeholder was plan-mandated dead code; the
FERRISBAR_* overrides were folded into Task 8 so the line is unnecessary.
Committed as 0010b89.

Task 1: complete (commits 0010b89..846f7b9, review clean)
  - toml >=1.1,<1.2 (std,parse) + flate2 (rust_backend) added; just ci exit 0
  - 11 vet exemptions added (3 more than the brief estimated)
  - cfg-if exemption escalated safe-to-run -> safe-to-deploy: crc32fast now
    pulls it into the runtime binary. Reviewer independently confirmed sound.
  - toml_writer exemption is for an inert lock-file-only crate (display
    feature is off); verified zero dependents. Not a misconfiguration.
Task 2: fix round 1/5 (1 addressed, 0 open — over-scoped #[allow(dead_code)]; commits 36715f2..6d413b7)
Task 2: complete (commits 846f7b9..6d413b7, review clean)
  - src/paths.rs: pure resolvers + env wrappers, 8 tests
  - Lesson for later tasks: the first pass added 10 #[allow(dead_code)];
    only 3 were load-bearing. Reviewer verified empirically. Carry this
    forward — an allow that suppresses nothing is never flagged by clippy,
    so it never gets cleaned up.
  - Resolved myself (reviewer could not verify locally): macOS/Windows cfg
    branches do not compile on this Linux host. Not a gap — ci.yml:17 runs
    the matrix on ubuntu/macos/windows-latest, so they compile at PR time.
Task 3: fix round 1/5 (1 addressed, 0 open — 7 non-load-bearing #[allow(dead_code)]; commits e9f788e..cde3b1a)
Task 3: complete (commits 6d413b7..cde3b1a, review clean)
  - src/config.rs: lenient toml::Table parsing, clamping, TEMPLATE. 11 tests.
  - PLAN DEFECT FOUND AND CORRECTED (commit 6000e9f): the brief specified
    toml parse-only. toml::Table and toml::Value are gated behind the serde
    feature in 1.1.3. Verified by removing it: 8 compile errors. Cargo.toml
    now has ["std","parse","serde"]; spec and plan updated. vet still passes.
  - Implementer's first report claimed empirical verification of its allows
    that it had not performed; it retracted this when challenged. Reviewer
    independently reproduced the corrected claims.
  - RECURRING: over-applied #[allow(dead_code)] for the 2nd task running.
    Remaining dispatches carry an explicit removal-and-rebuild procedure.
Task 4: fix round 1/5 (1 addressed, 0 open — 4 of 5 #[allow(dead_code)] redundant; commits 8f38f3b..1b38141)
Task 4: complete (commits cde3b1a..1b38141, review clean)
  - src/config.rs: load()/create_template(), ParseWarning::Create. 16 tests.
  - Root cause of the 3x recurrence identified: rustc dead-code works from
    reachability roots. One allow on the single unreferenced root covers the
    whole transitive chain. config.rs went 5 allows -> 1. Remaining dispatches
    state this principle explicitly rather than listing attributes.
Task 4: minor (deferred): ParseWarning::Create also fires on a permission-denied
  READ, not just a create failure. Name is imprecise. Plan-mandated (my brief's
  Step 3), so not fixed in-loop. RESOLUTION: Task 8 must not blindly map Create
  -> "config_create_failed"; it needs an event name covering both. Carried into
  the Task 8 dispatch.
Task 4: reviewer ⚠️ resolved by me — EACCES / invalid-UTF-8 paths are reviewed
  by inspection only, not empirically triggered. Task 9's e2e adds a
  non-writable data dir case that exercises the same catch-all arm.
Task 5: fix round 1/5 (1 addressed, 1 open — 5 redundant allows fixed; comment accuracy not; commits 84ef464..66dc099)
Task 5: fix round 2/5 (1 addressed, 0 open — Logger::new comment pointed at Task 7, should be Task 8; commits 66dc099..34a0b94)
Task 5: complete (commits 1b38141..34a0b94, review clean)
  - src/log.rs: Level/Event/Logger, line_for with injected ts, level
    filtering, relative-path resolution against data dir. 11 tests.
  - Allows went 10 -> 5 (+1 unused_self); reviewer re-verified all 6 remaining
    are load-bearing after the trim.
  - impl-task-5 terminated without returning a status on both its first run
    and fix round 1; I verified tests/lint by hand each time. Work was sound.
Task 6: complete (commits 34a0b94..e0d67ac, review clean, NO fix round)
  - src/log.rs: rotation under O_EXCL lock w/ stale reclaim, archive shift,
    gzip. 19 tests. Reviewer verified ordering, lock release on all branches,
    MSRV at 1.85.1, and both clippy-forced deviations (archive_path pub,
    io::Error::other) independently.
  - Allows now 3 (debug, Logger::new, Logger::log); warn()'s became redundant
    once append() called it on the rotation-failure path. First task where the
    implementer got the allow set right unprompted.
  - PLAN DEFECT FOUND BY IMPLEMENTER AND CORRECTED (commit ec013e9): the
    concurrency test was specced into Task 6 but main.rs does not construct a
    Logger until Task 8, so it failed deterministically on a missing log dir.
    It also would not have exercised rotation once wired -- 12 short debug
    lines ~1-2KB vs a 4096-byte threshold. Moved to Task 9 Step 4b with the
    log pre-seeded past the threshold and an assert that >=1 archive exists.
Task 6: reviewer ⚠️ noted — real multi-process behavior is still unproven;
  in-process lock simulation is a proxy. Closes when Task 9 Step 4b runs.
Task 7: complete (commits ec013e9..8843300, review clean, NO fix round)
  - config_dir.rs: pure resolve(env, file, home) + claude_config_dir(override).
    4 new tests. setup.rs guard relaxed to 3 sources, reports resolved paths.
  - Implementer proactively removed 3 now-redundant paths.rs allows after
    setup::run made them reachable a task early. Reviewer independently
    re-verified. First implementer to get the allow audit right unprompted.
  - Remaining allows: config.rs load (1), log.rs (3). Both confirmed still
    load-bearing; both should fall away in Task 8 when main.rs wires them.
Task 8: complete (commits 8843300..bcc5045, review clean, NO fix round)
  - main.rs wired: config load -> FERRISBAR_* overrides -> logger -> flush
    deferred warnings -> render. Degradation events at the former silent
    returns. Reviewer independently reproduced byte-identical stdout across
    3 payload shapes + empty stdin + non-JSON stdin.
  - Deferred minor from Task 4 RESOLVED: ParseWarning::Create now maps to
    event "config_unavailable", not "config_create_failed", because load()
    also produces Create for a permission-denied read. Verified both paths.
  - All remaining #[allow(dead_code)] in src/ are now gone (grep confirms 0).
    The 4 that survived Tasks 4-7 all became reachable from main here.
  - Reviewer ⚠️ on MSRV resolved by me: `just msrv` -> "Rust 1.85.1 Is
    compatible". CI gate green.
  - LEAK CONFIRMED: the brief predicted tests/cli.rs would fail after this
    change. It does not fail -- it leaks. Running the suite created a real
    ~/.config/ferrisbar/config.toml and ~/.local/share/ferrisbar/logs/
    ferrisbar.jsonl on this machine (log referenced /tmp/.tmpZD2Af2/todos,
    so unambiguously test debris). I inspected and deleted both. This is
    exactly Task 9's scope and raises its priority: silent leakage is worse
    than a red test because nothing signals it.
Task 9: fix round 1/5 (3 addressed, 0 open — vacuous concurrency test, env gaps, doc comment; commits 963cb65..fb0cbd6)
Task 9: complete (commits bcc5045..fb0cbd6, review clean)
  - tests/cli.rs: OVERRIDABLE_ENV_VARS + command_with_home/without_home,
    11 existing tests rerouted, 8 new. 19 CLI tests. Leak check clean after
    a full `just test` (verified by me and by the reviewer independently).
  - PLAN DEFECT: my concurrency test was VACUOUS. One pre-seed meant the
    first process rotated and the other 11 found a small file and never
    contended (archives==1 every run). Reviewer proved it by mutating
    acquire_lock create_new->create: still passed 8/8. Fixed (commit 26a1458)
    with a 6000-char model name so every render's own line exceeds
    max_size_bytes; now 2-5 archives, assert >=2.
  - PLAN DEFECT: isolated() omitted FERRISBAR_LOG_PATH/FERRISBAR_LOG_LEVEL,
    which Task 8 made beat the config file — an exported LOG_PATH would have
    reintroduced the leak. Implementer consolidated the env list into one
    const, removing the drift risk rather than just the instance.
  - BLOCKED RAISED AND RESOLVED BY ME: implementer reported the mutation test
    undetectable at e2e level. Correct, but the conclusion was scoped too
    narrowly -- it and the reviewer both ran only `cargo test --test cli`.
    I ran the same mutation against the UNIT tests:
    log::tests::a_held_lock_defers_rotation_without_losing_the_line FAILS.
    O_EXCL exclusivity IS mutation-covered, by Task 6's unit test. Reviewer
    independently reproduced this and agreed. Division of labor stands: unit
    test pins the lock primitive, e2e proves real multi-process rotation.
Task 10: complete (commits fb0cbd6..6b350d9, review clean, NO fix round)
  - README Configuration section rewritten; "There is no config file" gone.
    CLAUDE.md invariant now says four runtime deps. New e2e test for
    FERRISBAR_LOG_LEVEL. Reviewer fact-checked every path/key/default/event
    name against source rather than the brief -- all matched.
  - Implementer also fixed a "two runtime crates" claim in README's Features
    bullet (outside brief scope) that would have contradicted the same
    commit's CLAUDE.md edit. Reviewer confirmed necessary.
  - PLAN NIT: brief's Step 6 expected logs/ to exist but be empty after a
    healthy render. Wrong -- Logger::append only creates the dir on an actual
    write, and a clean warn-level render emits only a suppressed debug event,
    so logs/ is never created. No doc claims otherwise; README prose ("a
    statusline that renders correctly writes nothing") is consistent.
Task 10: minor (deferred): CLAUDE_CODE_AUTO_COMPACT_WINDOW=0 falls through to
  the config file rather than overriding (main.rs requires >0.0). Consistent
  with the template's "0 = built-in buffer" vocabulary, but the precedence
  table's "overrides" reads broader than the behavior. For final-review triage.

ALL 10 TASKS COMPLETE. `just ci` EXIT=0 verified by controller: 99 unit +
20 e2e tests, fmt/clippy/audit/msrv/deny/trivy/vet clean, geiger
informational-only. Leak check clean after full ci.

FINAL WHOLE-BRANCH REVIEW (56743f4..6b350d9, 24 commits, opus)
  Verdict: ready except one gating item. Reviewer independently verified zero
  panic paths in src/, byte-identical stdout across 5 degradation modes,
  nan/-1/1e400 auto_compact_window safe, no leak, no .rotating/.rotate.lock
  residue, [display] correctly absent.
  IMPORTANT (gating): todo_file_unreadable is inverted. Fires every render
  when todos/ is merely absent (not a fault); silent when a todo file is
  malformed or unreadable (the actual faults). Contradicts this branch's own
  README ("any content in this file is a signal"). This repo commits
  .claude/settings.json with no todos/, so it would fire constantly. Root
  cause: my Task 8 dispatch invented a directory-existence gate instead of
  spec:295's "session todo file missing or malformed" in todo.rs.
  MINOR: setup reports default_log_path, ignoring cfg.log.path/enabled and
  FERRISBAR_LOG_PATH -- prints a path the renderer will not use.
  MINOR: no non-writable-directory test anywhere (spec:490, spec:500 unmet).
  Behavior verified correct by probe; coverage gap only. NOTE: the Task 4
  ledger entry claiming Task 9 would cover this was WRONG -- it did not.
  MINOR: render event lacks used_pct (spec:301).
  MINOR: FERRISBAR_LOG_LEVEL=debug cannot re-enable logging when the file
  says enabled=false -- breaks the documented env-beats-file rule.
  MINOR: CLAUDE_CODE_AUTO_COMPACT_WINDOW fall-through is broader than noted:
  0, negative, AND unparseable all defer to the file.
  Deferred item triage: (1) acw=0 acceptable, needs a README clause covering
  "0 or unparseable"; (2) concurrency-test division of labor ADEQUATE, wants
  a code comment so nobody "strengthens" it into flakiness; (3) malformed-
  config re-warn acceptable as specified -- but the carve-out is specific and
  must not be stretched to cover the todo warn.
