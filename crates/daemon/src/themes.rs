//! Custom color themes: schemes made in the app's theme editor, or sent by a tool or a
//! Claude session over `PUT /api/themes/{id}`.
//!
//! Persisted to `<state_dir>/themes.json`; with no `state_dir` they live in memory only.
//! A theme is a name plus a map of xterm `ITheme` keys (`background`, `foreground`,
//! `cursor`, `selectionBackground`, the 16 ANSI names) to `#rrggbb`. Only `background`
//! and `foreground` are required; the app fills the rest. Every change records a
//! `themes-changed` event, so open windows pick it up without a reload.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Upper bound on stored themes, so a runaway client can't grow the file without limit.
const MAX_THEMES: usize = 100;
const MAX_ID: usize = 48;
const MAX_NAME: usize = 64;

/// The `ITheme` keys a custom theme may set.
pub const THEME_KEYS: [&str; 21] = [
    "background",
    "foreground",
    "cursor",
    "cursorAccent",
    "selectionBackground",
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightMagenta",
    "brightCyan",
    "brightWhite",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTheme {
    /// `[a-z0-9-]`, 1–48 chars; the app persists it as the active scheme id.
    pub id: String,
    pub name: String,
    /// `ITheme` key → lowercase `#rrggbb`.
    pub theme: BTreeMap<String, String>,
    /// Epoch milliseconds of the last save.
    pub updated_at: i64,
}

/// A `PUT` body: the id comes from the path.
#[derive(Deserialize)]
struct ThemeBody {
    name: String,
    theme: BTreeMap<String, String>,
}

pub struct ThemeStore {
    path: Option<PathBuf>,
    items: Mutex<Vec<CustomTheme>>,
}

/// `#rgb`, `#rrggbb` (any case, `#` optional) → lowercase `#rrggbb`.
fn normalize_hex(v: &str) -> Option<String> {
    let h = v.trim().trim_start_matches('#');
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let h = h.to_ascii_lowercase();
    match h.len() {
        6 => Some(format!("#{h}")),
        3 => Some(h.chars().fold(String::from("#"), |mut s, c| {
            s.push(c);
            s.push(c);
            s
        })),
        _ => None,
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Check and normalize a theme; the error says what's wrong, for the 400 body.
fn validate(
    id: &str,
    name: &str,
    theme: BTreeMap<String, String>,
) -> Result<(String, BTreeMap<String, String>), String> {
    if !valid_id(id) {
        return Err(format!("id must be 1–{MAX_ID} of [a-z0-9-]"));
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(format!("name must be 1–{MAX_NAME} characters"));
    }
    let mut out = BTreeMap::new();
    for (k, v) in theme {
        if !THEME_KEYS.contains(&k.as_str()) {
            return Err(format!(
                "unknown key {k:?}; expected one of {}",
                THEME_KEYS.join(", ")
            ));
        }
        let Some(hex) = normalize_hex(&v) else {
            return Err(format!("{k}: {v:?} is not a #rrggbb color"));
        };
        out.insert(k, hex);
    }
    for k in ["background", "foreground"] {
        if !out.contains_key(k) {
            return Err(format!(
                "theme needs at least background and foreground; {k} is missing"
            ));
        }
    }
    Ok((name.to_string(), out))
}

impl ThemeStore {
    /// Load from `<state_dir>/themes.json` (missing or unreadable → none).
    pub fn open(state_dir: Option<&Path>) -> Self {
        let path = state_dir.map(|d| d.join("themes.json"));
        let items = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Vec<CustomTheme>>(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            items: Mutex::new(items),
        }
    }

    /// Every custom theme, in the order first saved.
    pub fn list(&self) -> Vec<CustomTheme> {
        self.lock().clone()
    }

    /// Create or replace theme `id` (replacing keeps its place in the list).
    pub fn put(
        &self,
        id: &str,
        name: &str,
        theme: BTreeMap<String, String>,
        now_ms: i64,
    ) -> Result<(), String> {
        let (name, theme) = validate(id, name, theme)?;
        let mut items = self.lock();
        let entry = CustomTheme {
            id: id.to_string(),
            name,
            theme,
            updated_at: now_ms,
        };
        if let Some(slot) = items.iter_mut().find(|t| t.id == id) {
            *slot = entry;
        } else {
            if items.len() >= MAX_THEMES {
                return Err("too many themes".into());
            }
            items.push(entry);
        }
        self.persist(&items);
        Ok(())
    }

    /// Delete theme `id`; false when there was none.
    pub fn delete(&self, id: &str) -> bool {
        let mut items = self.lock();
        let before = items.len();
        items.retain(|t| t.id != id);
        let changed = items.len() != before;
        if changed {
            self.persist(&items);
        }
        changed
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<CustomTheme>> {
        self.items.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Best-effort atomic write (tmp + rename); a failed write keeps the in-memory list.
    fn persist(&self, items: &[CustomTheme]) {
        let Some(path) = &self.path else { return };
        let Ok(bytes) = serde_json::to_vec_pretty(items) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// `GET /api/themes` — every custom theme.
pub(crate) async fn themes_route(State(state): State<AppState>) -> Response {
    Json(state.themes.list()).into_response()
}

/// `PUT /api/themes/{id}` — body `{"name": "...", "theme": {"background": "#…", …}}`.
/// Local origin and a JSON content type are required (the content type forces a CORS
/// preflight this daemon never grants). Returns the full list.
pub(crate) async fn theme_put_route(
    AxumPath(id): AxumPath<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let json_ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if !crate::pty::origin_is_local(&headers) || !json_ct {
        return (StatusCode::FORBIDDEN, "local JSON requests only").into_response();
    }
    let b =
        match serde_json::from_slice::<ThemeBody>(&body) {
            Ok(b) => b,
            Err(e) => return (
                StatusCode::BAD_REQUEST,
                format!(
                    "need {{\"name\": \"…\", \"theme\": {{\"background\": \"#rrggbb\", …}}}}: {e}"
                ),
            )
                .into_response(),
        };
    match state
        .themes
        .put(&id, &b.name, b.theme, chrono::Utc::now().timestamp_millis())
    {
        Ok(()) => {
            state.events.record(
                "themes-changed",
                serde_json::json!({ "id": id, "op": "put" }),
            );
            Json(state.themes.list()).into_response()
        }
        Err(e) if e == "too many themes" => (StatusCode::INSUFFICIENT_STORAGE, e).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// `DELETE /api/themes/{id}` — 404 when there was none. Returns the full list.
pub(crate) async fn theme_delete_route(
    AxumPath(id): AxumPath<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if !crate::pty::origin_is_local(&headers) {
        return (StatusCode::FORBIDDEN, "local requests only").into_response();
    }
    if state.themes.delete(&id) {
        state.events.record(
            "themes-changed",
            serde_json::json!({ "id": id, "op": "delete" }),
        );
        Json(state.themes.list()).into_response()
    } else {
        (StatusCode::NOT_FOUND, "no such theme").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
    fn base() -> BTreeMap<String, String> {
        colors(&[("background", "#101010"), ("foreground", "#EEE")])
    }

    #[test]
    fn themes_survive_a_reopen_and_normalize_colors() {
        let dir = tempfile::tempdir().unwrap();
        let s = ThemeStore::open(Some(dir.path()));
        s.put("dusk", "Dusk", base(), 1).unwrap();
        drop(s);

        let s = ThemeStore::open(Some(dir.path()));
        let t = &s.list()[0];
        assert_eq!(
            (t.id.as_str(), t.name.as_str(), t.updated_at),
            ("dusk", "Dusk", 1)
        );
        assert_eq!(t.theme["foreground"], "#eeeeee");
        assert_eq!(t.theme["background"], "#101010");
    }

    #[test]
    fn replacing_keeps_the_place() {
        let s = ThemeStore::open(None);
        s.put("a", "A", base(), 1).unwrap();
        s.put("b", "B", base(), 2).unwrap();
        s.put("a", "A2", base(), 3).unwrap();
        let l = s.list();
        assert_eq!(
            l.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["A2", "B"]
        );
    }

    #[test]
    fn bad_input_is_refused_with_a_reason() {
        let s = ThemeStore::open(None);
        assert!(s.put("Bad Id", "x", base(), 0).unwrap_err().contains("id"));
        assert!(s.put("ok", "  ", base(), 0).unwrap_err().contains("name"));
        let mut t = base();
        t.insert("purple".into(), "#ff00ff".into());
        assert!(s.put("ok", "x", t, 0).unwrap_err().contains("unknown key"));
        let mut t = base();
        t.insert("red".into(), "tomato".into());
        assert!(s.put("ok", "x", t, 0).unwrap_err().contains("red"));
        let t = colors(&[("background", "#000000")]);
        assert!(s.put("ok", "x", t, 0).unwrap_err().contains("foreground"));
        assert!(s.list().is_empty());
    }

    #[test]
    fn delete_says_whether_it_existed() {
        let s = ThemeStore::open(None);
        s.put("a", "A", base(), 0).unwrap();
        assert!(s.delete("a"));
        assert!(!s.delete("a"));
    }

    #[test]
    fn a_corrupt_file_reads_as_no_themes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("themes.json"), "not json").unwrap();
        assert!(ThemeStore::open(Some(dir.path())).list().is_empty());
    }

    #[test]
    fn the_store_is_bounded() {
        let s = ThemeStore::open(None);
        for i in 0..MAX_THEMES {
            s.put(&format!("t{i}"), "t", base(), 0).unwrap();
        }
        assert!(s.put("one-more", "t", base(), 0).is_err());
        s.put("t0", "replace still works", base(), 0).unwrap();
    }
}
