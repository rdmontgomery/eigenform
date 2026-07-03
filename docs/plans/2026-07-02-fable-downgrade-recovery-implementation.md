# Fable→Opus Downgrade Recovery — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** When a Claude Code session is downgraded Fable→Opus by the safety guardrail, detect it, fork a fresh Fable session truncated to before the offending prompt, stage a model-suggested restatement in the input, auto-open it in the Furnace — and never auto-send.

**Architecture:** A *pure* detector in the `forest` crate rides the existing `/api/forest` snapshot as a new `downgrade` field (cheap, recomputed on the 3s poll / SSE — no side effects). The expensive, side-effecting recovery (fork + rephrase) lives behind a new `POST /api/session/:uuid/recover-downgrade` route that mirrors the existing `fork_route` and reuses `surgery::fork_before`. The rephrase shells out to headless `claude -p` via an **injectable command** so tests use a stub. The web client badges every downgraded session and, for the *active* session only, fires recovery once and opens the branch with the restatement pre-filled.

**Tech Stack:** Rust (`eigenform-forest`, `eigenform-daemon`, `eigenform-surgery`), axum, serde_json; TypeScript/vitest webterm client.

**Design:** `docs/plans/2026-07-02-fable-downgrade-recovery-design.md`. **Spike:** `notes/spikes/10-resume-model-derivation.md` (resume follows the truncated transcript onto Fable; no `--model` force required for correctness).

**Key facts from spikes (do not re-derive):**
- A downgrade/interrupt notice is an `assistant` row with `message.model == "<synthetic>"` carrying a human-readable string.
- **Session-limit fallback produces the same Fable→Opus transition** (`You've hit your session limit …`), so detection MUST match the specific guardrail wording, not the model jump.
- No real guardrail string exists in local transcripts yet → the marker constant is a **scrubbed-in placeholder**; capture the true string from the first live occurrence.
- `surgery::fork_before(src, turn)` already truncates to the completed-turn boundary *before* `turn` (dropping `turn` and everything after) and mints a new uuid. `POST /api/session/:uuid/fork` + `fork_session` already wrap it.

---

## Task 0: Teach surgery the snake_case `session_id` (claude 2.1.199+) — unblocks the fork path

**Why this is first:** `claude 2.1.199` (the version in use) started writing a second,
snake_case `session_id` field on assistant rows alongside the existing camelCase
`sessionId`. `surgery::rewrite_session_fields` only treats `sessionId` as an id-bearing key;
any *other* key whose value equals the session id is flagged as a stray and the rewrite
**refuses** (spike-07 guard). So `fork_before → finish → rewrite_session_id` now errors on any
2.1.199 session — breaking the existing per-turn fork **and** this feature's `recover-downgrade`
route (both share that path). The corpus property test already fails on this. Fix it before
building anything on top of the fork.

**Files:**
- Modify: `crates/surgery/src/lib.rs` (`rewrite_session_fields`, ~line 83 — the key check)
- Test: inline `#[cfg(test)]` unit test in `crates/surgery/src/lib.rs`; the corpus test
  (`crates/surgery/tests/corpus.rs`) greens automatically once fixed.
- Create: `notes/spikes/11-session-id-snake-case.md` (record the version + field change).

**Step 1: Write the failing unit test**

Add to the surgery crate's tests (match the existing test-module style in the file):

```rust
#[test]
fn rewrites_both_camel_and_snake_session_id() {
    // claude 2.1.199 assistant rows carry BOTH keys with the same value.
    let line = r#"{"type":"assistant","uuid":"a1","session_id":"OLD","sessionId":"OLD"}"#;
    let out = rewrite_session_id(line, "OLD", "NEW").expect("must not flag a stray");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("sessionId").unwrap(), "NEW");
    assert_eq!(v.get("session_id").unwrap(), "NEW");
}

#[test]
fn still_flags_a_genuinely_stray_occurrence() {
    // A non-id key whose value equals the session id must STILL be refused (spike-07 guard).
    let line = r#"{"type":"assistant","uuid":"a1","sessionId":"OLD","note":"OLD"}"#;
    assert!(rewrite_session_id(line, "OLD", "NEW").is_err());
}

#[test]
fn pre_2_1_199_rows_with_only_sessionid_still_rewrite() {
    // BACKWARDS COMPAT: older sessions (no snake_case field) must be unaffected — the
    // widened key set adds a case, it never removes the sessionId case.
    let line = r#"{"type":"assistant","uuid":"a1","sessionId":"OLD"}"#;
    let out = rewrite_session_id(line, "OLD", "NEW").unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("sessionId").unwrap(), "NEW");
}
```

**Backwards compatibility:** the fix only *widens* the set of id-bearing keys (adds
`session_id` alongside `sessionId`); it removes nothing. A pre-2.1.199 transcript has no
snake_case field, so the new case never fires on it and behaviour is identical. This is
guaranteed live by the corpus test, which runs the guarded swap over sessions spanning every
local version (2.1.138 → 2.1.199) — a regression on old transcripts would fail it. The
`pre_2_1_199_…` unit test pins the same guarantee in isolation.

**Step 2: Run to verify it fails**

Run: `cargo test -p eigenform-surgery rewrites_both_camel_and_snake`
Expected: FAIL — `StrayOccurrence` for the `session_id` key.

**Step 3: Minimal implementation**

In `rewrite_session_fields` (crates/surgery/src/lib.rs), widen the swappable-key check from a
single `"sessionId"` to both spellings:

```rust
        Value::String(s) => {
            if s == old {
                if key == Some("sessionId") || key == Some("session_id") {
                    *s = new.to_string();
                } else {
                    *stray = true;
                }
            }
        }
```

Update the doc-comment on `rewrite_session_id` / `rewrite_session_fields` to note both keys are
rewritten (claude 2.1.199+ writes snake_case `session_id` too).

**Step 4: Run to verify pass + baseline green**

Run: `cargo test -p eigenform-surgery`
Expected: the two new unit tests PASS **and** `corpus_round_trips_and_guards_cleanly_across_versions`
PASS (it was failing on this exact row).

**Step 5: Spike note + commit**

Write `notes/spikes/11-session-id-snake-case.md`: Status CONFIRMED, `claude --version` 2.1.199,
Date 2026-07-02. Claim: 2.1.199 adds a snake_case `session_id` on assistant rows alongside
`sessionId`; surgery must rewrite both or forks of 2.1.199 sessions fail. Evidence: only 2.1.199
carries it across the local corpus (all prior versions back to 2.1.138 have `sessionId` only).

```bash
git add crates/surgery/src/lib.rs notes/spikes/11-session-id-snake-case.md
git commit -m "surgery: rewrite snake_case session_id too (claude 2.1.199); greens corpus + fork"
```

---

## Task 1: Pure downgrade detector (forest crate)

**Files:**
- Modify: `crates/forest/src/lib.rs` (add `Downgrade`, `GUARDRAIL_MARKER`, `detect_downgrade`)
- Test: inline `#[cfg(test)]` module in the same file (match the crate's existing test style)

**Step 1: Write the failing tests**

Add fixtures + tests. Each fixture is a small JSONL string. `main-chain` = `isSidechain:false`.

```rust
#[cfg(test)]
mod downgrade_tests {
    use super::*;
    use std::io::Write;

    fn tmp_jsonl(body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f
    }

    // user(offending) -> synthetic guardrail notice -> opus assistant
    fn guardrail_fixture() -> String {
        [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"benign question"},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"a1","message":{"model":"claude-fable-5","role":"assistant","content":[{"type":"text","text":"ok"}]},"sessionId":"s"}"#,
            r#"{"type":"system","isSidechain":false,"subtype":"turn_duration","uuid":"sys1","sessionId":"s"}"#,
            r#"{"type":"user","isSidechain":false,"uuid":"u2","message":{"role":"user","content":"the offending prompt"},"sessionId":"s"}"#,
            &format!(r#"{{"type":"assistant","isSidechain":false,"uuid":"synth","message":{{"model":"<synthetic>","role":"assistant","content":[{{"type":"text","text":"{GUARDRAIL_MARKER}"}}]}},"sessionId":"s"}}"#),
            r#"{"type":"assistant","isSidechain":false,"uuid":"a2","message":{"model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"reply"}]},"sessionId":"s"}"#,
        ].join("\n") + "\n"
    }

    #[test]
    fn fires_on_guardrail_marker_targeting_offending_user_turn() {
        let f = tmp_jsonl(&guardrail_fixture());
        let d = detect_downgrade(f.path()).expect("should fire");
        assert_eq!(d.offending_turn, "u2"); // last main-chain user turn before the marker
    }

    #[test]
    fn session_limit_fallback_does_not_fire() {
        // same shape but the synthetic text is the session-limit notice, not the guardrail
        let body = [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"go"},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"synth","message":{"model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"You've hit your session limit · resets 4pm"}]},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"a2","message":{"model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"reply"}]},"sessionId":"s"}"#,
        ].join("\n") + "\n";
        let f = tmp_jsonl(&body);
        assert!(detect_downgrade(f.path()).is_none());
    }

    #[test]
    fn opus_subagent_sidechain_does_not_fire() {
        // an Opus subagent turn on a sidechain must be ignored entirely
        let body = [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"go"},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":true,"uuid":"sub","message":{"model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"subagent work"}]},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"a1","message":{"model":"claude-fable-5","role":"assistant","content":[{"type":"text","text":"still fable"}]},"sessionId":"s"}"#,
        ].join("\n") + "\n";
        let f = tmp_jsonl(&body);
        assert!(detect_downgrade(f.path()).is_none());
    }

    #[test]
    fn always_opus_session_does_not_fire() {
        let body = [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"go"},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"a1","message":{"model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"reply"}]},"sessionId":"s"}"#,
        ].join("\n") + "\n";
        let f = tmp_jsonl(&body);
        assert!(detect_downgrade(f.path()).is_none());
    }

    #[test]
    fn marker_with_no_prior_user_turn_does_not_fire() {
        let body = format!(
            r#"{{"type":"assistant","isSidechain":false,"uuid":"synth","message":{{"model":"<synthetic>","role":"assistant","content":[{{"type":"text","text":"{GUARDRAIL_MARKER}"}}]}},"sessionId":"s"}}"#
        ) + "\n";
        let f = tmp_jsonl(&body);
        assert!(detect_downgrade(f.path()).is_none());
    }
}
```

If `tempfile` isn't already a dev-dependency of the crate, add it: check `crates/forest/Cargo.toml` `[dev-dependencies]`; add `tempfile = "3"` if missing (it is already used elsewhere in the workspace).

**Step 2: Run tests to verify they fail**

Run: `cargo test -p eigenform-forest downgrade_tests`
Expected: FAIL — `cannot find function detect_downgrade` / `cannot find type Downgrade`.

**Step 3: Write the minimal implementation**

Add near the other public types in `crates/forest/src/lib.rs`:

```rust
/// A detected Fable→Opus **guardrail** downgrade in a session transcript.
#[derive(Debug, Clone)]
pub struct Downgrade {
    /// The main-chain user turn whose response tripped the guardrail — the
    /// `fork_before` target. Forking before it drops the offending prompt so the
    /// user can restate it on a fresh Fable branch.
    pub offending_turn: String,
}

/// The guardrail-downgrade notice string Claude Code writes as a `<synthetic>`
/// assistant turn.
///
/// ⚠ SCRUBBED IN. No real guardrail sample exists in local transcripts yet — see
/// `notes/spikes/10-resume-model-derivation.md` (only session-limit + API-error
/// synthetics were observed). Capture the true string from the first live
/// occurrence and replace this one line, recording it with `claude --version` in
/// a new spike. Matching is a substring test, so a stable fragment is enough.
const GUARDRAIL_MARKER: &str = "switched this session to a safer model"; // PLACEHOLDER

/// Scan a session JSONL for the first guardrail downgrade. Returns the offending
/// user turn, or `None`. Pure: reads the file, no side effects.
///
/// A downgrade notice is a **main-chain** (`isSidechain:false`) `assistant` row
/// whose `message.model == "<synthetic>"` and whose text contains
/// [`GUARDRAIL_MARKER`]. Sidechain (subagent) turns are ignored — an Opus
/// subagent is benign, not a downgrade of your thread. The offending turn is the
/// last main-chain `user` turn *before* that notice.
pub fn detect_downgrade(jsonl_path: &Path) -> Option<Downgrade> {
    let text = fs::read_to_string(jsonl_path).ok()?;
    let mut last_user: Option<String> = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("isSidechain").and_then(|b| b.as_bool()).unwrap_or(false) {
            continue; // subagent / sidechain — never a downgrade of the main thread
        }
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                if let Some(uuid) = v.get("uuid").and_then(|x| x.as_str()) {
                    last_user = Some(uuid.to_string());
                }
            }
            Some("assistant") => {
                let msg = v.get("message");
                let model = msg.and_then(|m| m.get("model")).and_then(|x| x.as_str());
                if model == Some("<synthetic>") && synthetic_text(msg).contains(GUARDRAIL_MARKER) {
                    return last_user.map(|offending_turn| Downgrade { offending_turn });
                }
            }
            _ => {}
        }
    }
    None
}

/// Concatenate the text blocks of an assistant `message.content` (string or array form).
fn synthetic_text(msg: Option<&serde_json::Value>) -> String {
    let Some(content) = msg.and_then(|m| m.get("content")) else { return String::new() };
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|x| x.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p eigenform-forest downgrade_tests`
Expected: PASS (5 tests).

**Step 5: Commit**

```bash
git add crates/forest/src/lib.rs crates/forest/Cargo.toml
git commit -m "forest: pure guardrail-downgrade detector (scrubbed-in marker)"
```

---

## Task 2: Surface `downgrade` on the forest snapshot

**Files:**
- Modify: `crates/forest/src/lib.rs` (`LiveSession` + `live_forest_with`)
- Modify: `crates/daemon/src/lib.rs:428` (`forest_json`)
- Test: `crates/daemon/tests/host_routes.rs` (add a `/api/forest` assertion) — inspect the file first to match its harness.

**Step 1: Add the field to `LiveSession`**

In `crates/forest/src/lib.rs`, add to the struct (after `spark`):

```rust
    /// Present iff a Fable→Opus guardrail downgrade was detected in this session.
    pub downgrade: Option<Downgrade>,
```

In `live_forest_with`, where the `LiveSession { … }` is built (around line 388), add:

```rust
            downgrade: detect_downgrade(&r.path),
```

Run: `cargo build -p eigenform-forest` — expect an error only if some other constructor of `LiveSession` exists; grep `LiveSession {` and fix each. Then `cargo test -p eigenform-forest` stays green.

**Step 2: Emit it in `forest_json`**

In `crates/daemon/src/lib.rs`, inside the `.map(|s| { serde_json::json!({ … }) })` in `forest_json`, add a field:

```rust
                    "downgrade": s.downgrade.as_ref().map(|d| serde_json::json!({
                        "offendingTurn": d.offending_turn,
                    })),
```

`null` when absent — the client treats `null`/absent identically.

**Step 3: Write a route test**

Read `crates/daemon/tests/host_routes.rs` first for the existing fixture-dir + request helpers. Add a test that writes a projects/sessions/state layout containing the Task 1 `guardrail_fixture()` transcript, GETs `/api/forest`, and asserts the matching row has `downgrade.offendingTurn == "u2"`; a clean session has `downgrade == null`.

**Step 4: Run**

Run: `cargo test -p eigenform-daemon forest`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/forest/src/lib.rs crates/daemon/src/lib.rs crates/daemon/tests/host_routes.rs
git commit -m "forest: carry downgrade marker on the /api/forest snapshot"
```

---

## Task 3: Injectable headless-claude rephraser (daemon)

**Files:**
- Modify: `crates/daemon/src/lib.rs` (`Config` + new `rephrase_prompt`)
- Modify: every `Config { … }` construction site (grep `Config {`) to set the new field — daemon `main`, all tests. Default is `vec!["claude".into(), "-p".into()]`.
- Test: `crates/daemon/tests/` new `rephrase.rs` using a stub script.

**Step 1: Extend `Config`**

Add to `Config` (in `crates/daemon/src/lib.rs`):

```rust
    /// Command that turns an offending prompt into a suggested restatement. The
    /// composed prompt is appended as the final argv entry; stdout is the
    /// suggestion. Default `["claude", "-p"]`; tests inject a stub. Keeping the
    /// daemon's only model call as a `claude` subprocess (never the API) matches
    /// the invariant that eigenform only ever runs `claude`.
    pub rephrase_cmd: Vec<String>,
```

**Step 2: Write the failing test**

`crates/daemon/tests/rephrase.rs`:

```rust
use std::io::Write;

// A stub that ignores everything and prints a canned suggestion, proving the
// command is injectable and stdout is captured.
fn stub_script() -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new().suffix(".sh").tempfile().unwrap();
    writeln!(f, "#!/bin/sh\necho 'restated: please advise on defensive hardening'").unwrap();
    let mut perms = std::fs::metadata(f.path()).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(f.path(), perms).unwrap();
    f
}

#[test]
fn rephrase_runs_the_injected_command_and_returns_stdout() {
    let stub = stub_script();
    let cmd = vec![stub.path().to_str().unwrap().to_string()];
    let out = eigenform_daemon::rephrase_prompt(&cmd, std::path::Path::new("/tmp"), "the offending prompt")
        .expect("stub succeeds");
    assert!(out.contains("restated:"), "got: {out}");
}
```

Make `rephrase_prompt` `pub` for the integration test (it's a thin, testable seam).

**Step 3: Implement**

```rust
/// Run `cfg.rephrase_cmd` in `cwd`, appending the restatement instruction + the
/// offending prompt as the final argv entry, and return trimmed stdout. Errors
/// (spawn failure, non-zero exit, empty output) bubble up so the caller can fall
/// back to the verbatim prompt.
pub fn rephrase_prompt(
    cmd: &[String],
    cwd: &Path,
    offending: &str,
) -> std::io::Result<String> {
    let (program, args) = cmd
        .split_first()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty rephrase_cmd"))?;
    let prompt = format!(
        "The prompt below caused an over-eager safety downgrade of a coding \
         session. Restate it to remove ambiguity and make the benign intent \
         explicit, preserving the actual ask. Return ONLY the restated prompt. \
         If the ask is genuinely disallowed, say so plainly instead.\n\n{offending}"
    );
    let output = std::process::Command::new(program)
        .args(args)
        .arg(&prompt)
        .current_dir(cwd)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("rephrase command exited {}", output.status),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "empty rephrase output"));
    }
    Ok(text)
}
```

Add `use std::path::Path;` if not already imported (it is).

**Step 4: Run**

Run: `cargo test -p eigenform-daemon rephrase`
Expected: PASS. Then `cargo build` to confirm every `Config {` site now compiles with the new field.

**Step 5: Commit**

```bash
git add crates/daemon/src/lib.rs crates/daemon/tests/rephrase.rs
git commit -m "daemon: injectable headless-claude rephraser seam"
```

---

## Task 4: `recover-downgrade` route

**Files:**
- Modify: `crates/daemon/src/lib.rs` (route registration line ~108, handler, helper)
- Test: `crates/daemon/tests/host_routes.rs`

**Step 1: Write the failing route test**

Using the same fixture harness as Task 2, `POST /api/session/<src>/recover-downgrade` with a config whose `rephrase_cmd` is the Task 3 stub script. Assert the response JSON has:
- `branchUuid` — a new uuid, and a `<branchUuid>.jsonl` now exists in the project dir,
- `stagedText` containing `"restated:"`,
- `offendingTurn == "u2"`,
- `note == null`.

Add a second test: config whose `rephrase_cmd` is `["false"]` (exits non-zero) → `stagedText` equals the **verbatim** offending prompt (`"the offending prompt"`) and `note` is non-null (the fallback path).

**Step 2: Register + implement**

Add the route (near line 108):

```rust
        .route("/api/session/:uuid/recover-downgrade", post(recover_downgrade_route))
```

Handler + helper:

```rust
/// `POST /api/session/:uuid/recover-downgrade` — detect the guardrail downgrade,
/// fork a Fable branch truncated to before the offending prompt (reusing
/// `fork_session` → `surgery::fork_before`), and stage a suggested restatement.
/// Never sends. Returns `{ branchUuid, stagedText, offendingTurn, note }`.
async fn recover_downgrade_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    match recover_downgrade(&state.config, &uuid) {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

fn recover_downgrade(
    cfg: &Config,
    src_uuid: &str,
) -> Result<serde_json::Value, (StatusCode, &'static str)> {
    let dir = cfg
        .projects_dir
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, "no projects dir configured"))?;
    let src_path = eigenform_forest::resolve(dir, src_uuid)
        .map_err(|_| (StatusCode::NOT_FOUND, "no such session"))?;
    let down = eigenform_forest::detect_downgrade(&src_path)
        .ok_or((StatusCode::UNPROCESSABLE_ENTITY, "no downgrade detected"))?;

    // Fork first (this is the load-bearing step). Reuses the existing primitive.
    let branch_uuid = fork_session(cfg, src_uuid, &down.offending_turn)?;

    // Pull the offending prompt's text for the rephraser and the verbatim fallback.
    let offending_text = user_turn_text(&src_path, &down.offending_turn).unwrap_or_default();
    let cwd = src_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let (staged_text, note) = match rephrase_prompt(&cfg.rephrase_cmd, &cwd, &offending_text) {
        Ok(t) => (t, serde_json::Value::Null),
        Err(_) => (
            offending_text,
            serde_json::Value::String("couldn't reach the rephraser — staged your prompt verbatim".into()),
        ),
    };

    Ok(serde_json::json!({
        "branchUuid": branch_uuid,
        "stagedText": staged_text,
        "offendingTurn": down.offending_turn,
        "note": note,
    }))
}

/// The `message.content` text of the user turn with `uuid` in the JSONL at `path`.
fn user_turn_text(path: &Path, uuid: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if v.get("uuid").and_then(|x| x.as_str()) == Some(uuid) {
            let content = v.get("message")?.get("content")?;
            if let Some(s) = content.as_str() {
                return Some(s.to_string());
            }
            // content-array form: join text blocks
            return content.as_array().map(|bs| {
                bs.iter().filter_map(|b| b.get("text").and_then(|x| x.as_str()))
                    .collect::<Vec<_>>().join("")
            });
        }
    }
    None
}
```

Note the `?` in `user_turn_text` on a non-JSON line would early-return `None`; use `let Ok(v) = … else { continue };` instead so a stray opaque line doesn't abort the scan. Fix that when implementing.

**Step 3: Run**

Run: `cargo test -p eigenform-daemon recover`
Expected: PASS (both tests — success + verbatim-fallback).

**Step 4: Full backend gate**

Run: `cargo test` (workspace) and `cargo clippy --all-targets -- -D warnings`
Expected: green.

**Step 5: Commit**

```bash
git add crates/daemon/src/lib.rs crates/daemon/tests/host_routes.rs
git commit -m "daemon: POST recover-downgrade — fork + staged rephrase"
```

---

## Task 5: Client — downgrade badge on every affected session

**Files:**
- Modify: `webterm/src/types.ts` (`ForestItem`)
- Modify: `webterm/src/roster.ts` (carry the flag onto `RosterRow`) — inspect how `ForestItem`→`RosterRow` is built
- Modify: `webterm/src/shell.ts:1441` (`renderRailRow`) + `webterm/src/style.css`
- Test: `webterm/src/roster.test.ts`

**Step 1: Extend the type**

In `webterm/src/types.ts`, add to `ForestItem`:

```ts
  /** Present iff a Fable→Opus guardrail downgrade was detected. */
  downgrade?: { offendingTurn: string } | null;
```

**Step 2: Carry it through the roster (test-first)**

Read `roster.ts` + `roster.test.ts`. If `RosterRow` is derived from `ForestItem`, add a `downgrade` field and a test asserting a `ForestItem` with `downgrade` yields a `RosterRow` with it set. Run: `npm test -- roster` → FAIL, then thread the field through → PASS.

**Step 3: Render the badge**

In `renderRailRow` (`shell.ts`), when `row.downgrade` is set, append a small chip (mirror the existing state-badge markup):

```ts
if (row.downgrade) {
  const chip = el("span", "rail-downgrade");
  chip.textContent = "fable→opus";
  chip.title = "Guardrail downgraded this session to Opus — a Fable retry can be staged";
  // append into the row's badge area
}
```

Add CSS in `style.css` (mirror an existing chip's tokens):

```css
.rail-downgrade {
  font-size: 9.5px;                /* match neighbouring meta chips */
  color: var(--st-running);        /* theme-adaptive amber; NOT a hardcoded hex */
  border: 1px solid currentColor;
  border-radius: 3px;
  padding: 0 3px;
  margin-left: 4px;
}
```

> Note: use the real design token `--st-running` (regenerated per color scheme by
> `deriveChrome`), never an invented `--warn`/hardcoded hex — the badge must adapt
> to light/dark and alternate schemes like every other rail chip.

**Step 4: Run**

Run: `npm test` (webterm) → PASS. Build check: `npm run build`.

**Step 5: Commit**

```bash
git add webterm/src/types.ts webterm/src/roster.ts webterm/src/shell.ts webterm/src/style.css webterm/src/roster.test.ts
git commit -m "webterm: badge sessions the guardrail downgraded"
```

---

## Task 6: Client — auto-recover the active session, staged not sent

**Files:**
- Modify: `webterm/src/shell.ts` (forest-poll handler ~1246; the `onFork` open path ~594)
- Test: a focused unit around the "fire once" gate if the logic is extractable; otherwise verify manually per Step 4.

**Behaviour:**
1. Keep a module-level `const recovered = new Set<string>()` of source uuids already handled (fires once per session).
2. In the forest-poll handler, after the snapshot lands: find the row for the **active tab's** session uuid. If it has `downgrade`, is live, and is not in `recovered`:
   - add it to `recovered`,
   - **skip** if the user typed into the active pty within the last 1500 ms (track `lastInputAt`; never eat a keystroke),
   - `POST /api/session/<uuid>/recover-downgrade`, read `{ branchUuid, stagedText, note }`,
   - open the branch as a new tab via the **existing** fork-open path (`openTabWithQuery('?session=<branchUuid>', { uuid: branchUuid, label: 'fable-retry' })`) + `refreshRoster()`,
   - deliver `stagedText` into the branch pty **without a trailing newline** using the same live-delivery mechanism the per-turn fork uses to inject an edited prompt (see `onFork` in `shell.ts:594` and the `fork_before` doc-comment in `surgery/src/lib.rs:285` — "delivered live into the resumed branch"). **Do NOT append `\n` / submit.**
   - if `note` is non-null, surface it as a transient toast/line near the input.

**Only the active session auto-recovers** (prime scope). Other downgraded sessions keep the Task 5 badge; clicking a badge MAY trigger the same recovery on demand (optional, YAGNI for now).

**Step 1–2:** If you can extract the decision (`shouldAutoRecover(row, activeUuid, recovered, lastInputAt, now) → boolean`) into a pure helper, write a vitest for it first (fires once; skips when recently typed; skips non-active). Otherwise proceed to wiring and rely on manual verification.

**Step 3: Wire it** per the behaviour above.

**Step 4: Manual verification (headless Chromium is blocked — see memory)**

Because a real guardrail downgrade can't be summoned on demand and the marker is scrubbed-in, verify with a **synthetic transcript**:
- Start a throwaway daemon against a temp projects dir (`eigenform daemon --port <p> --projects <dir> …`, matching the memory's throwaway-daemon recipe).
- Drop in a JSONL that ends with the Task 1 `guardrail_fixture()` shape but using the **current `GUARDRAIL_MARKER`** string.
- `curl localhost:<p>/api/forest` → confirm the row carries `downgrade.offendingTurn`.
- `curl -X POST localhost:<p>/api/session/<uuid>/recover-downgrade` with `rephrase_cmd` set to a stub → confirm `branchUuid`, `stagedText`, and that `<branchUuid>.jsonl` was written and ends on a completed Fable turn (`grep` the bundle/file).
- Confirm the branch does NOT contain the offending prompt (fork_before dropped it).

**Step 5: Commit**

```bash
git add webterm/src/shell.ts
git commit -m "webterm: auto-stage a Fable retry for the active downgraded session"
```

**Follow-ups (from the final full-branch review; none blocking merge):**
- The 500 ms seed settle (after `onSessionUuid`) is a heuristic. A more robust signal is the **first output frame after attach** (or "quiet for N ms after first output") rather than the `onSessionUuid` transcript event — harden later so the seed reliably lands after claude's `--resume` has painted its input line.
- **`recovered` dedupe is in-memory only** (`webterm/src/shell.ts`). After a page reload, re-activating a still-downgraded source session re-fires and forks a *second* `fable-retry` branch (safe — source untouched, nothing sent — but litters branches). Persist handled uuids, or skip when a branch of this source already exists. Cannot manifest until Task 7 activates the marker.
- **`rephrase_prompt` runs `claude -p` synchronously on a tokio worker** (`daemon/src/lib.rs`). Fine for a local single-user daemon (mirrors the existing blocking-fs routes), but a multi-second rephrase blocks that worker — move to `spawn_blocking` if it ever matters.

---

## Task 7: Capture the real marker (deferred, follow-up spike)

Not blocking. When a real Fable→Opus guardrail downgrade next occurs in a live
session, capture the exact `<synthetic>` notice string, replace `GUARDRAIL_MARKER`
(one line), and record it in a new `notes/spikes/12-guardrail-marker.md` (spike 11
is already taken by the session_id change) with `claude --version` per the
`vetting-claude-internals` habit. Until then the feature is wired end-to-end but
dormant on the placeholder string — which is the intended "vitality now,
reliability at steady state" posture.

---

## Definition of done

- `cargo test` (workspace) + `cargo clippy --all-targets -- -D warnings` green.
- `npm test` + `npm run build` (webterm) green.
- Synthetic-transcript manual check (Task 6 Step 4) passes end-to-end.
- Source sessions never mutated (copy-on-fork); no path auto-sends a prompt.
- The only thing standing between dormant and live is the one-line marker (Task 7).
```

