//! eigenform-codex: read OpenAI Codex CLI threads off disk, the way the forest reads
//! Claude Code sessions — so Codex workers Claude delegates to (`codex exec`) show up in
//! the rail and drawer instead of disappearing into a background shell.
//!
//! Read-only, like everything else here: nothing in this crate spawns `codex` or writes
//! under `$CODEX_HOME`. The on-disk shapes it depends on are pinned in
//! `notes/spikes/16-codex-rollout-schema.md`; liveness rides Codex's own writer lock
//! (`notes/spikes/14-codex-concurrent-resume.md`).
//!
//! Layout (`$CODEX_HOME`, default `~/.codex`):
//! - `sessions/YYYY/MM/DD/rollout-<ts>-<thread_id>.jsonl` — one append-only rollout per
//!   thread. Each line is `{timestamp, type, payload}`; `type` is `session_meta`,
//!   `response_item`, `event_msg`, `turn_context`, `compacted`, … Cold rollouts may be
//!   compressed to `.jsonl.zst`; those are skipped (not yet read).
//! - `thread-writer-locks/<thread_id>.lock` — `flock`ed by the one process writing that
//!   thread. Held = a live writer; its pid is read from `/proc/locks` without touching
//!   the lock (taking it, even briefly, could make a real writer fail to start).

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// Head window scanned for `session_meta` and the first user prompt.
const HEAD_WINDOW: u64 = 256 * 1024;
/// Tail window scanned for recency, turn state, and the current model.
const TAIL_WINDOW: u64 = 64 * 1024;
/// Max chars kept from the first prompt as a title.
const TITLE_CHARS: usize = 80;
/// Cap on tool input/output strings in the transcript JSON — matches the render crate.
const TOOL_CONTENT_BYTES: usize = 50 * 1024;
/// How deep under `sessions/` rollouts live (`YYYY/MM/DD/file`), plus slack.
const MAX_DEPTH: usize = 5;

/// `$CODEX_HOME`, else `~/.codex`. None when neither resolves.
pub fn codex_home() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(h));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex"))
}

/// A cheaply-enumerated rollout: thread id (from the file name) and path. No contents read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadStub {
    pub id: String,
    pub path: PathBuf,
}

/// The thread id encoded in a rollout file name: `rollout-<YYYY-MM-DDThh-mm-ss>-<id>.jsonl`.
/// A reverted thread's name appends `_<rollout_id>` after the stable thread id; the thread
/// id is what `codex resume` takes, so that's what is returned.
pub fn thread_id_from_file_name(name: &str) -> Option<&str> {
    let core = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    // 19-char timestamp, then `-`, then the id(s).
    if core.len() < 21 || core.as_bytes()[19] != b'-' {
        return None;
    }
    let ids = &core[20..];
    let thread = ids.split_once('_').map_or(ids, |(t, _)| t);
    (!thread.is_empty()).then_some(thread)
}

/// Every plain `.jsonl` rollout under `codex_home/sessions`. Empty if the dir is absent.
pub fn enumerate(codex_home: &Path) -> Vec<ThreadStub> {
    let mut out = Vec::new();
    walk(&codex_home.join("sessions"), 0, &mut out);
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<ThreadStub>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk(&path, depth + 1, out);
        } else if let Some(id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(thread_id_from_file_name)
        {
            out.push(ThreadStub {
                id: id.to_string(),
                path: path.clone(),
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    NotFound,
    Ambiguous(usize),
}

/// Resolve a thread `query` (full id or unique prefix) to its rollout. A thread that was
/// reverted can own several rollouts; the newest file (by name, which leads with its
/// timestamp) wins.
pub fn resolve(codex_home: &Path, query: &str) -> Result<ThreadStub, ResolveError> {
    if query.is_empty() {
        return Err(ResolveError::NotFound);
    }
    let mut matches: Vec<ThreadStub> = enumerate(codex_home)
        .into_iter()
        .filter(|s| s.id.starts_with(query))
        .collect();
    let distinct: std::collections::BTreeSet<&str> =
        matches.iter().map(|s| s.id.as_str()).collect();
    match distinct.len() {
        0 => Err(ResolveError::NotFound),
        1 => {
            matches.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
            Ok(matches.pop().expect("one match"))
        }
        n => Err(ResolveError::Ambiguous(n)),
    }
}

/// A thread's process state, from its writer lock and the rollout tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// A live writer, a turn in flight.
    Working,
    /// A live writer, last turn complete — an interactive `codex` awaiting input.
    Ready,
    /// No live writer; history.
    Recent,
}

impl ThreadState {
    /// Same vocabulary as `eigenform_forest::SessionState::as_str`.
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadState::Working => "working",
            ThreadState::Ready => "ready",
            ThreadState::Recent => "recent",
        }
    }
}

/// A rail row for one Codex thread.
#[derive(Debug, Clone)]
pub struct CodexThread {
    pub id: String,
    pub path: PathBuf,
    pub cwd: PathBuf,
    /// First user prompt (one line, trimmed). None before the first turn lands.
    pub title: Option<String>,
    /// Last timestamped line, else the file's mtime.
    pub recency: DateTime<Utc>,
    /// `session_meta.source`, flattened: `exec`, `cli`, `vscode`, `mcp`, `subagent`, …
    pub source: String,
    /// Launched non-interactively (`codex exec`, MCP server, a Codex-spawned subagent).
    pub headless: bool,
    /// Last `turn_context.model` seen.
    pub model: Option<String>,
    /// `session_meta.parent_thread_id` / `forked_from_id`, when Codex recorded one.
    pub parent: Option<String>,
    /// Number of user turns (the rail's `~N` count).
    pub turns: usize,
    /// Pid holding the thread's writer lock. None = no live writer.
    pub writer_pid: Option<u32>,
    pub state: ThreadState,
}

impl CodexThread {
    pub fn live(&self) -> bool {
        self.writer_pid.is_some()
    }
}

/// Everything about a thread that is a pure function of its file — memoized per
/// (path, mtime, len) so the forest tick doesn't re-read static rollouts.
#[derive(Debug, Clone)]
struct FileFacts {
    cwd: PathBuf,
    title: Option<String>,
    recency: DateTime<Utc>,
    source: String,
    model: Option<String>,
    parent: Option<String>,
    turns: usize,
    /// Last of task_started / task_complete / turn_aborted was task_started.
    in_turn: bool,
}

type FactsKey = (PathBuf, u128, u64);
static FACTS: Mutex<Option<HashMap<FactsKey, FileFacts>>> = Mutex::new(None);

fn file_key(path: &Path) -> Option<FactsKey> {
    let md = fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((path.to_path_buf(), mtime, md.len()))
}

fn facts(path: &Path) -> FileFacts {
    let key = file_key(path);
    if let Some(k) = &key {
        if let Some(hit) = FACTS.lock().ok().and_then(|g| g.as_ref()?.get(k).cloned()) {
            return hit;
        }
    }
    let f = read_facts(path);
    if let Some(k) = key {
        if let Ok(mut g) = FACTS.lock() {
            g.get_or_insert_with(HashMap::new).insert(k, f.clone());
        }
    }
    f
}

fn read_facts(path: &Path) -> FileFacts {
    let len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let head = read_range(path, 0, HEAD_WINDOW.min(len));
    let tail_start = len.saturating_sub(TAIL_WINDOW);
    let tail = if tail_start == 0 {
        head.clone()
    } else {
        read_range(path, tail_start, len - tail_start)
    };

    let mut cwd = PathBuf::new();
    let mut source = String::new();
    let mut parent = None;
    let mut title = None;
    let mut fallback_title = None;
    for v in complete_lines(&head, false) {
        let payload = &v["payload"];
        match v["type"].as_str() {
            Some("session_meta") if source.is_empty() => {
                cwd = PathBuf::from(payload["cwd"].as_str().unwrap_or_default());
                source = flatten_source(&payload["source"]);
                parent = payload["parent_thread_id"]
                    .as_str()
                    .or_else(|| payload["forked_from_id"].as_str())
                    .map(str::to_string);
            }
            Some("event_msg") if title.is_none() && payload["type"] == "user_message" => {
                title = payload["message"].as_str().and_then(title_of);
            }
            Some("response_item") if fallback_title.is_none() => {
                if let Some(t) = user_text(payload) {
                    fallback_title = title_of(&t);
                }
            }
            _ => {}
        }
    }

    let mut last_ts = None;
    let mut model = None;
    let mut in_turn = false;
    for v in complete_lines(&tail, tail_start > 0) {
        if let Some(ts) = v["timestamp"].as_str().and_then(parse_ts) {
            last_ts = Some(ts);
        }
        let payload = &v["payload"];
        match v["type"].as_str() {
            Some("turn_context") => {
                if let Some(m) = payload["model"].as_str() {
                    model = Some(m.to_string());
                }
            }
            Some("event_msg") => match payload["type"].as_str() {
                Some("task_started" | "turn_started") => in_turn = true,
                Some("task_complete" | "turn_complete" | "turn_aborted") => in_turn = false,
                _ => {}
            },
            _ => {}
        }
    }

    // Turn count needs the whole file; a byte scan for the marker is cheap enough and
    // only happens when the file changed (facts are memoized).
    let turns = count_user_turns(path);

    FileFacts {
        cwd,
        title: title.or(fallback_title),
        recency: last_ts.unwrap_or_else(|| mtime_of(path)),
        source,
        model,
        parent,
        turns,
        in_turn,
    }
}

fn count_user_turns(path: &Path) -> usize {
    let Ok(contents) = fs::read_to_string(path) else {
        return 0;
    };
    let mut n = 0;
    for line in contents.lines() {
        // Cheap prefilter before parsing.
        if !line.contains("user_message") {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v["type"] == "event_msg" && v["payload"]["type"] == "user_message" {
                n += 1;
            }
        }
    }
    n
}

/// `session_meta.source` is an externally-tagged enum: a bare string (`"exec"`) or a
/// one-key object (`{"subagent": …}`). Flatten to the variant name, lowercased.
fn flatten_source(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_lowercase(),
        Value::Object(m) => m
            .keys()
            .next()
            .map(|k| k.to_lowercase())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Is this source a non-interactive launch?
pub fn is_headless(source: &str) -> bool {
    matches!(source, "exec" | "mcp" | "subagent" | "sub_agent")
}

/// Build the rail row for one rollout, judging liveness against `locks`.
pub fn thread(codex_home: &Path, stub: &ThreadStub, locks: &WriterLocks) -> CodexThread {
    let f = facts(&stub.path);
    let writer_pid = locks.holder(codex_home, &stub.id);
    let state = match (writer_pid.is_some(), f.in_turn) {
        (false, _) => ThreadState::Recent,
        (true, true) => ThreadState::Working,
        (true, false) => ThreadState::Ready,
    };
    CodexThread {
        id: stub.id.clone(),
        path: stub.path.clone(),
        cwd: f.cwd,
        title: f.title,
        recency: f.recency,
        headless: is_headless(&f.source),
        source: f.source,
        model: f.model,
        parent: f.parent,
        turns: f.turns,
        writer_pid,
        state,
    }
}

/// Every Codex thread on disk, newest first. Empty if `codex_home` has no sessions.
pub fn threads(codex_home: &Path) -> Vec<CodexThread> {
    let locks = WriterLocks::probe();
    let mut by_id: HashMap<String, ThreadStub> = HashMap::new();
    for stub in enumerate(codex_home) {
        // A reverted thread can own several rollouts; keep the newest file.
        match by_id.get(&stub.id) {
            Some(cur) if cur.path.file_name() >= stub.path.file_name() => {}
            _ => {
                by_id.insert(stub.id.clone(), stub);
            }
        }
    }
    let mut rows: Vec<CodexThread> = by_id
        .values()
        .map(|s| thread(codex_home, s, &locks))
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.recency));
    rows
}

/// A snapshot of the kernel's flock table (`/proc/locks`), keyed by (dev, inode).
/// Reading it never takes or perturbs a lock.
#[derive(Debug, Default, Clone)]
pub struct WriterLocks {
    held: HashMap<(u64, u64), u32>,
}

impl WriterLocks {
    /// Read `/proc/locks`. Off Linux (or unreadable) the table is empty: every thread
    /// reads as having no live writer.
    pub fn probe() -> Self {
        fs::read_to_string("/proc/locks")
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    /// Parse `/proc/locks` text. Lines look like
    /// `3: FLOCK  ADVISORY  WRITE 4242 00:2a:1234567 0 EOF`; blocked waiters carry a
    /// `->` after the ordinal and are skipped (they don't hold the lock).
    pub fn parse(text: &str) -> Self {
        let mut held = HashMap::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 6 || fields[1] != "FLOCK" {
                continue;
            }
            let Ok(pid) = fields[4].parse::<u32>() else {
                continue;
            };
            let mut dev_ino = fields[5].split(':');
            let (Some(maj), Some(min), Some(ino)) =
                (dev_ino.next(), dev_ino.next(), dev_ino.next())
            else {
                continue;
            };
            let (Ok(maj), Ok(min), Ok(ino)) = (
                u64::from_str_radix(maj, 16),
                u64::from_str_radix(min, 16),
                ino.parse::<u64>(),
            ) else {
                continue;
            };
            held.insert((makedev(maj, min), ino), pid);
        }
        WriterLocks { held }
    }

    /// The pid holding `codex_home/thread-writer-locks/<id>.lock`, if any.
    pub fn holder(&self, codex_home: &Path, id: &str) -> Option<u32> {
        let path = codex_home
            .join("thread-writer-locks")
            .join(format!("{id}.lock"));
        let (dev, ino) = dev_ino(&path)?;
        self.held.get(&(dev, ino)).copied()
    }
}

/// glibc's `makedev` encoding, so a `/proc/locks` major:minor matches `st_dev`.
fn makedev(major: u64, minor: u64) -> u64 {
    ((major & 0xfff) << 8) | ((major & !0xfff) << 32) | (minor & 0xff) | ((minor & !0xff) << 12)
}

#[cfg(unix)]
fn dev_ino(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let md = fs::metadata(path).ok()?;
    Some((md.dev(), md.ino()))
}

#[cfg(not(unix))]
fn dev_ino(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// The pid currently writing thread `id`, if any (one-off probe).
pub fn writer_pid(codex_home: &Path, id: &str) -> Option<u32> {
    WriterLocks::probe().holder(codex_home, id)
}

// ---------------------------------------------------------------------------
// Transcript → the drawer's session JSON
// ---------------------------------------------------------------------------

/// Render a rollout as the same session JSON the render crate emits for Claude
/// transcripts (`{id,total,branches,windowStart,model,exchanges}`), so the drawer, reach
/// map and forest preview consume a Codex thread without a separate code path.
///
/// Codex tools are mapped onto Claude's vocabulary where the reach map keys on it:
/// shell calls become `Bash {command}`, `apply_patch` becomes one `Edit`/`Write` per file
/// touched (absolute `file_path`), web search becomes `WebSearch {query}`. The original
/// tool name rides along as `input.codexTool`. Everything else passes through by name.
pub fn session_json(thread_id: &str, contents: &str) -> String {
    let lines: Vec<Value> = contents
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    // Prefer `event_msg.user_message` (the user's own words) for user turns; fall back to
    // `response_item` user messages for rollouts that predate it — filtered of the
    // context Codex injects as user-role messages (AGENTS.md, environment context).
    let has_user_events = lines
        .iter()
        .any(|v| v["type"] == "event_msg" && v["payload"]["type"] == "user_message");

    let mut outputs: HashMap<String, String> = HashMap::new();
    for v in &lines {
        let p = &v["payload"];
        if v["type"] == "response_item"
            && matches!(
                p["type"].as_str(),
                Some("function_call_output" | "custom_tool_call_output")
            )
        {
            if let Some(id) = p["call_id"].as_str() {
                outputs.insert(id.to_string(), output_text(&p["output"]));
            }
        }
    }

    let mut cwd = PathBuf::new();
    let mut model: Option<String> = None;
    let mut exchanges: Vec<Value> = Vec::new();
    for (i, v) in lines.iter().enumerate() {
        let p = &v["payload"];
        match v["type"].as_str() {
            Some("session_meta") => {
                if cwd.as_os_str().is_empty() {
                    cwd = PathBuf::from(p["cwd"].as_str().unwrap_or_default());
                }
            }
            Some("turn_context") => {
                if let Some(c) = p["cwd"].as_str() {
                    cwd = PathBuf::from(c);
                }
                if let Some(m) = p["model"].as_str() {
                    model = Some(m.to_string());
                }
            }
            Some("event_msg") if has_user_events && p["type"] == "user_message" => {
                let text = p["message"].as_str().unwrap_or_default();
                exchanges.push(json!({ "user": text, "uuid": format!("codex-{i}") }));
            }
            Some("response_item") => match p["type"].as_str() {
                Some("message") if p["role"] == "user" && !has_user_events => {
                    if let Some(text) = user_text(p) {
                        exchanges.push(json!({ "user": text, "uuid": format!("codex-{i}") }));
                    }
                }
                Some("message") if p["role"] == "assistant" => {
                    let text = content_text(&p["content"]);
                    if text.is_empty() {
                        continue;
                    }
                    match exchanges.last_mut() {
                        Some(cur) => append_text(cur, "assistant", &text),
                        None => exchanges.push(json!({ "user": "", "assistant": text })),
                    }
                }
                Some(
                    "function_call" | "custom_tool_call" | "local_shell_call" | "web_search_call",
                ) => {
                    for tool in tools_of(p, &outputs, &cwd) {
                        let needs_new = exchanges
                            .last()
                            .map(|e| e.get("tool").is_some())
                            .unwrap_or(true);
                        if needs_new {
                            exchanges.push(json!({ "user": "", "tool": tool }));
                        } else {
                            exchanges.last_mut().expect("non-empty")["tool"] = tool;
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    let count = exchanges.len();
    let mut out: Vec<Value> = Vec::with_capacity(count + 1);
    for (i, mut e) in exchanges.into_iter().enumerate() {
        e["n"] = json!(i + 1);
        e["tok"] = json!(0);
        out.push(e);
    }
    let total = count + 1;
    out.push(json!({ "n": total, "tok": 0, "user": "", "leaf": true }));

    serde_json::to_string(&json!({
        "id": thread_id.chars().take(8).collect::<String>(),
        "total": total,
        "branches": 0,
        "windowStart": 1,
        "model": model,
        "engine": "codex",
        "exchanges": out,
    }))
    .expect("session json serializes")
}

/// The drawer `tool` objects for one Codex call item. Usually one; `apply_patch` yields
/// one per file it touches.
fn tools_of(p: &Value, outputs: &HashMap<String, String>, cwd: &Path) -> Vec<Value> {
    let call_id = p["call_id"].as_str().unwrap_or_default();
    let output = outputs.get(call_id).map(String::as_str);
    match p["type"].as_str() {
        Some("web_search_call") => {
            let query = p["action"]["query"].as_str().unwrap_or_default();
            vec![tool(
                "WebSearch",
                json!({ "query": query, "codexTool": "web_search" }),
                output,
            )]
        }
        Some("local_shell_call") => {
            let action = &p["action"];
            let command = command_string(&action["command"]);
            let mut input = json!({ "command": command, "codexTool": "local_shell" });
            if let Some(wd) = action["working_directory"].as_str() {
                input["cwd"] = json!(wd);
            }
            vec![tool("Bash", input, output)]
        }
        Some("custom_tool_call") => {
            let name = p["name"].as_str().unwrap_or("tool");
            let raw = p["input"].as_str().unwrap_or_default();
            if name == "apply_patch" {
                return patch_tools(raw, cwd, output);
            }
            vec![tool(name, json!({ "input": raw }), output)]
        }
        _ => {
            // function_call: arguments is a JSON *string*.
            let name = p["name"].as_str().unwrap_or("tool");
            let args: Value = p["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(Value::Null);
            match name {
                "shell" | "container.exec" | "shell_command" | "exec_command" | "local_shell" => {
                    let cmd = if args["cmd"].is_null() {
                        &args["command"]
                    } else {
                        &args["cmd"]
                    };
                    let mut input = json!({ "command": command_string(cmd), "codexTool": name });
                    if let Some(wd) = args["workdir"].as_str() {
                        input["cwd"] = json!(wd);
                    }
                    vec![tool("Bash", input, output)]
                }
                "apply_patch" => {
                    let raw = args["input"]
                        .as_str()
                        .or(args["patch"].as_str())
                        .unwrap_or_default();
                    patch_tools(raw, cwd, output)
                }
                "view_image" => {
                    let path = args["path"].as_str().unwrap_or_default();
                    vec![tool(
                        "Read",
                        json!({ "file_path": absolutize(path, cwd), "codexTool": name }),
                        output,
                    )]
                }
                _ => {
                    let input = if args.is_null() { json!({}) } else { args };
                    vec![tool(name, input, output)]
                }
            }
        }
    }
}

/// One `apply_patch` envelope → an `Edit` (update/delete) or `Write` (add) per file, each
/// carrying its own section of the patch.
fn patch_tools(patch: &str, cwd: &Path, output: Option<&str>) -> Vec<Value> {
    let mut tools = Vec::new();
    let mut current: Option<(&str, String, String)> = None; // (kind, path, section)
    let flush = |cur: Option<(&str, String, String)>, tools: &mut Vec<Value>| {
        if let Some((kind, path, section)) = cur {
            tools.push(tool(
                kind,
                json!({ "file_path": absolutize(&path, cwd), "patch": section, "codexTool": "apply_patch" }),
                output,
            ));
        }
    };
    for line in patch.lines() {
        let header = [
            ("*** Add File: ", "Write"),
            ("*** Update File: ", "Edit"),
            ("*** Delete File: ", "Edit"),
        ]
        .iter()
        .find_map(|(prefix, kind)| {
            line.strip_prefix(prefix)
                .map(|p| (*kind, p.trim().to_string()))
        });
        if let Some((kind, path)) = header {
            flush(current.take(), &mut tools);
            current = Some((kind, path, format!("{line}\n")));
        } else if line.starts_with("*** End Patch") || line.starts_with("*** Begin Patch") {
            continue;
        } else if let Some((_, _, section)) = current.as_mut() {
            section.push_str(line);
            section.push('\n');
        }
    }
    flush(current.take(), &mut tools);
    if tools.is_empty() {
        tools.push(tool("apply_patch", json!({ "input": patch }), output));
    }
    tools
}

fn tool(kind: &str, input: Value, output: Option<&str>) -> Value {
    let input_str = serde_json::to_string(&input).unwrap_or_default();
    let (input_val, input_truncated) = if input_str.len() <= TOOL_CONTENT_BYTES {
        (input, false)
    } else {
        (Value::String(truncate(&input_str).0.to_string()), true)
    };
    let arg = input_val
        .as_object()
        .and_then(|m| m.values().find_map(|v| v.as_str()))
        .unwrap_or(kind)
        .to_string();
    let mut t = json!({ "kind": kind, "arg": arg, "delta": "", "input": input_val });
    if input_truncated {
        t["inputTruncated"] = json!(true);
    }
    if let Some(out) = output {
        let (cut, was) = truncate(out);
        t["output"] = json!(cut);
        if was {
            t["truncated"] = json!(true);
        }
    }
    t
}

fn truncate(s: &str) -> (&str, bool) {
    if s.len() <= TOOL_CONTENT_BYTES {
        return (s, false);
    }
    let mut end = TOOL_CONTENT_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

/// A shell command as one display string: `["bash","-lc","ls -la"]` → `ls -la`; other
/// argv arrays are space-joined; a bare string passes through.
fn command_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let argv: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            match argv.as_slice() {
                [_, flag, script, ..] if matches!(*flag, "-lc" | "-c") => script.to_string(),
                _ => argv.join(" "),
            }
        }
        _ => String::new(),
    }
}

fn absolutize(path: &str, cwd: &Path) -> String {
    let p = Path::new(path);
    if p.is_absolute() || cwd.as_os_str().is_empty() {
        path.to_string()
    } else {
        cwd.join(p).display().to_string()
    }
}

/// A function-call output is a string or an array of content items.
fn output_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(_) => content_text(v),
        Value::Object(m) => m
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| v.to_string()),
        _ => String::new(),
    }
}

/// Joined text of a content-item array (`input_text` / `output_text` / `text`).
fn content_text(v: &Value) -> String {
    v.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|it| it["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// The user's own text from a `response_item` user message, or None when the message is
/// context Codex injected as a user-role turn.
fn user_text(p: &Value) -> Option<String> {
    if p["type"] != "message" || p["role"] != "user" {
        return None;
    }
    let text = content_text(&p["content"]);
    let t = text.trim_start();
    let injected = t.is_empty()
        || t.starts_with('<')
        || t.starts_with("# AGENTS.md")
        || t.starts_with("# Context from my IDE");
    (!injected).then_some(text)
}

fn title_of(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut t: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().count() > TITLE_CHARS {
        t.push('…');
    }
    Some(t)
}

fn append_text(obj: &mut Value, key: &str, text: &str) {
    match obj.get_mut(key) {
        Some(Value::String(s)) if !s.is_empty() => {
            s.push_str("\n\n");
            s.push_str(text);
        }
        _ => obj[key] = json!(text),
    }
}

// ---------------------------------------------------------------------------
// File helpers
// ---------------------------------------------------------------------------

fn read_range(path: &Path, start: u64, len: u64) -> String {
    let Ok(mut f) = fs::File::open(path) else {
        return String::new();
    };
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buf = Vec::with_capacity(len as usize);
    let _ = f.take(len).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// Parse the complete JSON lines of a window. A window that starts mid-file drops its
/// first (partial) line; any trailing partial line simply fails to parse.
fn complete_lines(window: &str, skip_first: bool) -> impl Iterator<Item = Value> + '_ {
    window
        .lines()
        .skip(usize::from(skip_first))
        .filter_map(|l| serde_json::from_str(l).ok())
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn mtime_of(path: &Path) -> DateTime<Utc> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_yields_thread_id() {
        assert_eq!(
            thread_id_from_file_name(
                "rollout-2026-10-02T09-14-03-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b.jsonl"
            ),
            Some("0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b")
        );
        // A reverted thread keeps its stable id before the `_<rollout_id>` suffix.
        assert_eq!(
            thread_id_from_file_name("rollout-2026-10-02T09-14-03-aaaa_bbbb.jsonl"),
            Some("aaaa")
        );
        assert_eq!(
            thread_id_from_file_name("rollout-2026-10-02T09-14-03-x.jsonl.zst"),
            None
        );
        assert_eq!(thread_id_from_file_name("notes.jsonl"), None);
    }

    #[test]
    fn proc_locks_parse_skips_waiters_and_posix_locks() {
        let text = "1: FLOCK  ADVISORY  WRITE 4242 00:2a:1234567 0 EOF\n\
                    2: -> FLOCK  ADVISORY  WRITE 4343 00:2a:1234567 0 EOF\n\
                    3: POSIX  ADVISORY  WRITE 99 08:01:42 0 EOF\n";
        let locks = WriterLocks::parse(text);
        assert_eq!(locks.held.len(), 1);
        assert_eq!(locks.held.get(&(makedev(0, 0x2a), 1234567)), Some(&4242));
    }

    #[test]
    fn makedev_matches_glibc_for_small_numbers() {
        // major 8, minor 1 (sda1) → 0x801
        assert_eq!(makedev(8, 1), 0x801);
        assert_eq!(makedev(0, 0x2a), 0x2a);
    }

    #[test]
    fn command_string_unwraps_bash_lc() {
        assert_eq!(
            command_string(&json!(["bash", "-lc", "cargo test"])),
            "cargo test"
        );
        assert_eq!(command_string(&json!(["rg", "foo"])), "rg foo");
        assert_eq!(command_string(&json!("ls")), "ls");
    }

    #[test]
    fn patch_splits_per_file_and_absolutizes() {
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: b.txt\n+hi\n*** End Patch";
        let tools = patch_tools(patch, Path::new("/w"), Some("Success"));
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["kind"], "Edit");
        assert_eq!(tools[0]["input"]["file_path"], "/w/src/a.rs");
        assert!(tools[0]["input"]["patch"].as_str().unwrap().contains("+y"));
        assert_eq!(tools[1]["kind"], "Write");
        assert_eq!(tools[1]["input"]["file_path"], "/w/b.txt");
        assert_eq!(tools[1]["output"], "Success");
    }

    #[test]
    fn injected_context_is_not_a_title() {
        let env = json!({"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n</environment_context>"}]});
        let agents = json!({"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /w"}]});
        let real = json!({"type":"message","role":"user","content":[{"type":"input_text","text":"fix the flaky test"}]});
        assert_eq!(user_text(&env), None);
        assert_eq!(user_text(&agents), None);
        assert_eq!(user_text(&real).as_deref(), Some("fix the flaky test"));
    }
}
