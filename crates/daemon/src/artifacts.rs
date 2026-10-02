//! The artifact pane's backend: which renderable files a session produced, and a
//! sandboxed route that serves them for the pane's iframe.
//!
//! **Isolation is the point.** An artifact is HTML an agent wrote. Served from the
//! daemon's own origin it could open the `/pty` websocket (whose guard,
//! `origin_is_local`, admits any localhost origin) and get a shell. So every artifact
//! response carries `Content-Security-Policy: sandbox …` *without* `allow-same-origin`:
//! the browser gives the document an opaque (`null`) origin even on a direct top-level
//! navigation, a `null` Origin fails the pty guard, and with no CORS headers on `/api`
//! the page can't read the daemon's responses either.
//!
//! **Scope.** Only files the session itself wrote or edited (Write / Edit / MultiEdit /
//! NotebookEdit / Artifact, including inside nested subagent and Codex-worker
//! transcripts). Beside an HTML/SVG artifact, non-hidden files under its directory so
//! relative assets (`./style.css`, `img/x.png`) resolve; beside a markdown file, its
//! images only. Nothing else on disk is reachable.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::AppState;

/// File extensions the pane renders, and how.
pub fn kind_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" | "htm" => "html",
        "svg" => "svg",
        "md" | "markdown" => "markdown",
        "png" | "jpg" | "jpeg" | "gif" | "webp" => "image",
        _ => return None,
    })
}

/// Tools whose `input.file_path` is a file the session produced.
const WRITING_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Artifact"];

/// One renderable file a session produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub path: PathBuf,
    pub kind: &'static str,
    /// Exchange number (`n`) of the last write — the top-level exchange, for writes
    /// made inside a nested subagent / worker transcript.
    pub turn: u64,
    /// Which agent wrote it last: `"session"`, or the nested agent's type (`codex`, …).
    pub by: String,
}

/// Every renderable file written in a rendered session JSON (render-crate / Codex
/// shape), newest write first. Relative paths are skipped (no cwd to resolve against).
pub fn from_session_json(json: &str) -> Vec<Artifact> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut seen: HashMap<PathBuf, Artifact> = HashMap::new();
    let mut order = 0u64;
    let mut last_order: HashMap<PathBuf, u64> = HashMap::new();
    if let Some(exchanges) = v["exchanges"].as_array() {
        for ex in exchanges {
            let turn = ex["n"].as_u64().unwrap_or(0);
            collect(ex, turn, "session", &mut seen, &mut last_order, &mut order);
        }
    }
    let mut out: Vec<Artifact> = seen.into_values().collect();
    out.sort_by_key(|a| std::cmp::Reverse(last_order.get(&a.path).copied().unwrap_or(0)));
    out
}

fn collect(
    ex: &Value,
    turn: u64,
    by: &str,
    seen: &mut HashMap<PathBuf, Artifact>,
    last_order: &mut HashMap<PathBuf, u64>,
    order: &mut u64,
) {
    let tool = &ex["tool"];
    let kind = tool["kind"].as_str().unwrap_or_default();
    if WRITING_TOOLS.contains(&kind) {
        if let Some(fp) = tool["input"]["file_path"].as_str() {
            let path = PathBuf::from(fp);
            if path.is_absolute() {
                if let Some(k) = kind_of(&path) {
                    *order += 1;
                    last_order.insert(path.clone(), *order);
                    seen.insert(
                        path.clone(),
                        Artifact {
                            path,
                            kind: k,
                            turn,
                            by: by.to_string(),
                        },
                    );
                }
            }
        }
    }
    if let Some(sub) = tool["subagent"]["exchanges"].as_array() {
        let sub_by = tool["subagent"]["agentType"].as_str().unwrap_or("subagent");
        for sx in sub {
            collect(sx, turn, sub_by, seen, last_order, order);
        }
    }
}

/// Why a requested path can't be served.
#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    /// Not a file this session produced, nor a non-dot file beside one.
    OutOfScope,
    /// Doesn't exist (or can't be canonicalized).
    Missing,
}

/// Decide whether `requested` (an absolute path) may be served for a session whose
/// artifacts are `artifacts`. Returns the canonical path to read. Symlinks are resolved
/// before the scope check, so a link can't smuggle a path out of scope.
pub fn authorize(requested: &Path, artifacts: &[Artifact]) -> Result<PathBuf, Denied> {
    if !requested.is_absolute()
        || requested
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Denied::OutOfScope);
    }
    let canon = requested.canonicalize().map_err(|_| Denied::Missing)?;
    if !canon.is_file() {
        return Err(Denied::Missing);
    }
    for a in artifacts {
        let Ok(file) = a.path.canonicalize() else {
            continue;
        };
        if canon == file {
            return Ok(canon);
        }
        // What may ride along beside an artifact: an HTML/SVG document's relative assets
        // (anything non-hidden under its directory); a markdown file's images only, so
        // a plan.md at a repo root doesn't put the whole repo in scope; nothing for an
        // image.
        let siblings_ok = match a.kind {
            "html" | "svg" => true,
            "markdown" => kind_of(&canon) == Some("image"),
            _ => false,
        };
        if !siblings_ok {
            continue;
        }
        let Some(root) = file.parent() else { continue };
        if let Ok(rel) = canon.strip_prefix(root) {
            let hidden = rel
                .components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
            if !hidden {
                return Ok(canon);
            }
        }
    }
    Err(Denied::OutOfScope)
}

/// Content type for a served file. Unknown types go out as `application/octet-stream`
/// (with `nosniff`), so the browser downloads rather than guesses.
pub fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" | "csv" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The CSP for an artifact response. Always `sandbox` without `allow-same-origin`
/// (opaque origin). Scripted documents (HTML) may run scripts in that sandbox; rendered
/// markdown and everything else may not.
pub fn csp(scripts: bool) -> &'static str {
    if scripts {
        "sandbox allow-scripts allow-forms allow-popups allow-modals allow-downloads"
    } else {
        "sandbox; script-src 'none'"
    }
}

/// Render markdown as a standalone, readable HTML page (no scripts; served under
/// [`csp`]`(false)`). Light/dark follow the viewer's preference.
pub fn markdown_page(title: &str, md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let mut body = String::new();
    html::push_html(&mut body, Parser::new_ext(md, opts));
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title><style>{MD_CSS}</style></head><body><main>{body}</main></body></html>"#,
        title = escape(title),
    )
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const MD_CSS: &str = r#"
:root{color-scheme:light dark;--bg:#fbfaf7;--fg:#24211c;--muted:#6f6a61;--line:#e4e0d8;--code:#f1eee8;--accent:#9a5b2e}
@media (prefers-color-scheme:dark){:root{--bg:#1c1b19;--fg:#e8e4dc;--muted:#a39d92;--line:#35322d;--code:#26241f;--accent:#d9955f}}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.6 ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif}
main{max-width:46rem;margin:0 auto;padding:1.5rem 1.25rem 4rem}
h1,h2,h3,h4{line-height:1.25;margin:1.6em 0 .5em}h1{font-size:1.6rem}h2{font-size:1.25rem;border-bottom:1px solid var(--line);padding-bottom:.2em}h3{font-size:1.05rem}
a{color:var(--accent)}p,ul,ol,table,pre,blockquote{margin:0 0 1em}
code{font:13px/1.5 ui-monospace,"SF Mono",Menlo,monospace;background:var(--code);padding:.1em .3em;border-radius:3px}
pre{background:var(--code);padding:.8em 1em;border-radius:6px;overflow-x:auto}pre code{background:none;padding:0}
blockquote{border-left:3px solid var(--line);margin-left:0;padding-left:1em;color:var(--muted)}
table{border-collapse:collapse;display:block;overflow-x:auto}th,td{border:1px solid var(--line);padding:.35em .6em;text-align:left}
img{max-width:100%}hr{border:0;border-top:1px solid var(--line)}
"#;

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// `GET /api/session/:uuid/artifacts` — the renderable files (HTML, SVG, markdown,
/// images) the session wrote or edited, newest write first, each with the sandboxed
/// `/artifact/…` URL the pane loads and the file's current mtime (null if it's gone).
pub(crate) async fn artifacts_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let Some(json) = crate::session::session_json_cached(&state.config, &uuid) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let rows: Vec<serde_json::Value> = from_session_json(&json)
        .into_iter()
        .map(|a| {
            let mtime = std::fs::metadata(&a.path)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            serde_json::json!({
                "path": a.path.display().to_string(),
                "name": a.path.file_name().map(|n| n.to_string_lossy().into_owned()),
                "kind": a.kind,
                "turn": a.turn,
                "by": a.by,
                "mtime": mtime,
                "url": artifact_url(&uuid, &a.path),
            })
        })
        .collect();
    axum::Json(rows).into_response()
}

/// `/artifact/<uuid>/<absolute path, leading slash dropped, each segment
/// percent-encoded>` — relative references inside the document resolve beside it.
fn artifact_url(uuid: &str, path: &std::path::Path) -> String {
    let mut url = format!("/artifact/{}", percent_encode(uuid));
    for part in path.components() {
        if let std::path::Component::Normal(seg) = part {
            url.push('/');
            url.push_str(&percent_encode(&seg.to_string_lossy()));
        }
    }
    url
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct ArtifactQuery {
    /// `raw=1` serves a markdown file as text instead of the rendered page.
    #[serde(default)]
    raw: u8,
}

/// `GET /artifact/:uuid/*path` — one file in an artifact's scope (see
/// [`authorize`]), always under a `sandbox` CSP so the document gets an
/// opaque origin and can't reach the daemon's API or pty socket. Markdown is rendered
/// to a script-free page unless `?raw=1`.
pub(crate) async fn artifact_file_route(
    AxumPath((uuid, path)): AxumPath<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<ArtifactQuery>,
    State(state): State<AppState>,
) -> Response {
    use axum::http::header;
    let Some(json) = crate::session::session_json_cached(&state.config, &uuid) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let requested = PathBuf::from("/").join(path.trim_start_matches('/'));
    let arts = from_session_json(&json);
    let canon = match authorize(&requested, &arts) {
        Ok(p) => p,
        Err(Denied::Missing) => return (StatusCode::NOT_FOUND, "no such file").into_response(),
        Err(Denied::OutOfScope) => {
            return (StatusCode::FORBIDDEN, "not an artifact of this session").into_response()
        }
    };
    let Ok(bytes) = std::fs::read(&canon) else {
        return (StatusCode::NOT_FOUND, "no such file").into_response();
    };
    let common = |ct: &'static str, csp: &'static str| {
        [
            (header::CONTENT_TYPE, ct),
            (header::CONTENT_SECURITY_POLICY, csp),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ]
    };
    if kind_of(&canon) == Some("markdown") {
        if q.raw != 0 {
            return (common("text/plain; charset=utf-8", csp(false)), bytes).into_response();
        }
        let title = canon
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let page = markdown_page(&title, &String::from_utf8_lossy(&bytes));
        return (common("text/html; charset=utf-8", csp(false)), page).into_response();
    }
    let ct = content_type(&canon);
    let scripts = ct.starts_with("text/html") || ct == "image/svg+xml";
    (common(ct, csp(scripts)), bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(exchanges: Value) -> String {
        json!({ "exchanges": exchanges }).to_string()
    }

    #[test]
    fn collects_renderable_writes_newest_first_including_nested_workers() {
        let json = session(json!([
            {"n":1,"tool":{"kind":"Write","input":{"file_path":"/p/report.html"}}},
            {"n":2,"tool":{"kind":"Edit","input":{"file_path":"/p/src/lib.rs"}}},
            {"n":3,"tool":{"kind":"Read","input":{"file_path":"/p/old.html"}}},
            {"n":4,"tool":{"kind":"Write","input":{"file_path":"notes/rel.md"}}},
            {"n":5,"tool":{"kind":"Bash","input":{"command":"codex-worker spawn x"},
              "subagent":{"agentType":"codex","exchanges":[
                {"n":1,"tool":{"kind":"Write","input":{"file_path":"/w/chart.svg"}}}
              ]}}},
            {"n":6,"tool":{"kind":"Edit","input":{"file_path":"/p/report.html"}}}
        ]));
        let got = from_session_json(&json);
        let paths: Vec<&str> = got.iter().map(|a| a.path.to_str().unwrap()).collect();
        // report.html re-edited at n=6 → newest; chart.svg from the codex worker at n=5.
        assert_eq!(paths, vec!["/p/report.html", "/w/chart.svg"]);
        assert_eq!(got[0].turn, 6);
        assert_eq!(got[1].by, "codex");
        assert_eq!(got[1].kind, "svg");
    }

    #[test]
    fn authorize_admits_the_artifact_and_its_visible_siblings_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let out = root.join("out");
        std::fs::create_dir_all(out.join("img")).unwrap();
        std::fs::create_dir_all(out.join(".git")).unwrap();
        for f in [
            "index.html",
            "style.css",
            "img/a.png",
            ".env",
            ".git/config",
        ] {
            std::fs::write(out.join(f), "x").unwrap();
        }
        std::fs::write(root.join("secret.txt"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("secret.txt"), out.join("link.txt")).unwrap();

        let arts = vec![Artifact {
            path: out.join("index.html"),
            kind: "html",
            turn: 1,
            by: "session".into(),
        }];
        assert!(authorize(&out.join("index.html"), &arts).is_ok());
        assert!(authorize(&out.join("style.css"), &arts).is_ok());
        assert!(authorize(&out.join("img/a.png"), &arts).is_ok());
        assert_eq!(authorize(&out.join(".env"), &arts), Err(Denied::OutOfScope));
        assert_eq!(
            authorize(&out.join(".git/config"), &arts),
            Err(Denied::OutOfScope)
        );
        assert_eq!(
            authorize(&root.join("secret.txt"), &arts),
            Err(Denied::OutOfScope)
        );
        assert_eq!(
            authorize(&out.join("../secret.txt"), &arts),
            Err(Denied::OutOfScope)
        );
        #[cfg(unix)]
        assert_eq!(
            authorize(&out.join("link.txt"), &arts),
            Err(Denied::OutOfScope),
            "a symlink resolves before the scope check"
        );
        assert_eq!(
            authorize(&out.join("nope.html"), &arts),
            Err(Denied::Missing)
        );
    }

    #[test]
    fn a_markdown_artifact_admits_sibling_images_not_sibling_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for f in ["plan.md", "diagram.png", "Cargo.toml"] {
            std::fs::write(root.join(f), "x").unwrap();
        }
        let arts = vec![Artifact {
            path: root.join("plan.md"),
            kind: "markdown",
            turn: 1,
            by: "session".into(),
        }];
        assert!(authorize(&root.join("plan.md"), &arts).is_ok());
        assert!(authorize(&root.join("diagram.png"), &arts).is_ok());
        assert_eq!(
            authorize(&root.join("Cargo.toml"), &arts),
            Err(Denied::OutOfScope)
        );
    }

    #[test]
    fn markdown_renders_without_scripts_and_escapes_the_title() {
        let page = markdown_page("<plan>", "# Plan\n\n- [x] done\n\n|a|b|\n|-|-|\n|1|2|\n");
        assert!(page.contains("<h1>Plan</h1>"));
        assert!(page.contains("<table>"));
        assert!(page.contains("<title>&lt;plan&gt;</title>"));
        assert!(!csp(false).contains("allow-scripts"));
        assert!(!csp(true).contains("allow-same-origin"));
    }
}
