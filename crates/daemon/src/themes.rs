//! Custom color themes: the user's edited copies of a built-in scheme.
//!
//! Persisted to `<state_dir>/themes.json` so a theme outlives a reload, a daemon
//! restart and a change of browser profile; with no `state_dir` they live in memory
//! only. The daemon validates shape (id, name, `#rrggbb` colors under known keys) and
//! otherwise treats a theme as the client's data.

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
const MAX_NAME_CHARS: usize = 80;

/// The palette slots a theme must carry (xterm `ITheme` keys).
const PALETTE_KEYS: [&str; 20] = [
    "background",
    "foreground",
    "cursor",
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

/// Chrome tokens a theme may pin instead of deriving them.
const OVERRIDE_KEYS: [&str; 5] = ["accent", "running", "idle", "error", "success"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTheme {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub theme: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, String>,
    /// Epoch milliseconds of the last save.
    #[serde(default)]
    pub updated_at: i64,
}

fn is_hex6(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

impl CustomTheme {
    /// Check the shape and normalize colors to lowercase.
    fn validate(mut self) -> Result<Self, String> {
        if !is_id(&self.id) {
            return Err("id must be lowercase letters, digits and dashes".into());
        }
        self.name = self.name.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("name must be 1-{MAX_NAME_CHARS} characters"));
        }
        if self.base.as_deref().is_some_and(|b| !is_id(b)) {
            return Err("base must be a scheme id".into());
        }
        if let Some(k) = self
            .theme
            .keys()
            .find(|k| !PALETTE_KEYS.contains(&k.as_str()))
        {
            return Err(format!("unknown palette key {k:?}"));
        }
        if let Some(k) = PALETTE_KEYS.iter().find(|k| !self.theme.contains_key(**k)) {
            return Err(format!("missing palette key {k:?}"));
        }
        if let Some(k) = self
            .overrides
            .keys()
            .find(|k| !OVERRIDE_KEYS.contains(&k.as_str()))
        {
            return Err(format!("unknown override {k:?}"));
        }
        for (k, v) in self.theme.iter_mut().chain(self.overrides.iter_mut()) {
            if !is_hex6(v) {
                return Err(format!("{k} must be #rrggbb"));
            }
            v.make_ascii_lowercase();
        }
        Ok(self)
    }
}

pub struct ThemeStore {
    path: Option<PathBuf>,
    items: Mutex<Vec<CustomTheme>>,
}

impl ThemeStore {
    /// Load from `<state_dir>/themes.json` (missing or unreadable → none; invalid entries dropped).
    pub fn open(state_dir: Option<&Path>) -> Self {
        let path = state_dir.map(|d| d.join("themes.json"));
        let items = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Vec<CustomTheme>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| t.validate().ok())
            .collect();
        Self {
            path,
            items: Mutex::new(items),
        }
    }

    /// Every theme, in the order first saved.
    pub fn list(&self) -> Vec<CustomTheme> {
        self.lock().clone()
    }

    /// Insert or replace the theme with `theme.id`, keeping its place when it exists.
    pub fn save(&self, theme: CustomTheme, now_ms: i64) -> Result<(), String> {
        let mut theme = theme.validate()?;
        theme.updated_at = now_ms;
        let mut items = self.lock();
        if let Some(slot) = items.iter_mut().find(|t| t.id == theme.id) {
            *slot = theme;
        } else if items.len() >= MAX_THEMES {
            return Err("too many themes".into());
        } else {
            items.push(theme);
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

/// `PUT /api/themes/{id}` — create or replace a theme; the body's `id` must match the
/// path. Local origin and a JSON content type are required (the content type forces a
/// CORS preflight this daemon never grants). Returns the full list.
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
    let theme = match serde_json::from_slice::<CustomTheme>(&body) {
        Ok(t) if t.id == id => t,
        Ok(_) => return (StatusCode::BAD_REQUEST, "id does not match the path").into_response(),
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    match state
        .themes
        .save(theme, chrono::Utc::now().timestamp_millis())
    {
        Ok(()) => Json(state.themes.list()).into_response(),
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
        Json(state.themes.list()).into_response()
    } else {
        (StatusCode::NOT_FOUND, "no such theme").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(id: &str, name: &str) -> CustomTheme {
        CustomTheme {
            id: id.into(),
            name: name.into(),
            base: Some("dracula".into()),
            theme: PALETTE_KEYS
                .iter()
                .map(|k| (k.to_string(), "#AABBCC".to_string()))
                .collect(),
            overrides: BTreeMap::new(),
            updated_at: 0,
        }
    }

    #[test]
    fn themes_survive_a_reopen_with_colors_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let s = ThemeStore::open(Some(dir.path()));
        s.save(theme("custom-a", " A "), 5).unwrap();
        s.save(theme("custom-b", "B"), 6).unwrap();
        drop(s);

        let s = ThemeStore::open(Some(dir.path()));
        let list = s.list();
        assert_eq!(
            list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["custom-a", "custom-b"]
        );
        assert_eq!(list[0].name, "A");
        assert_eq!(list[0].theme["red"], "#aabbcc");
        assert_eq!(list[0].updated_at, 5);

        assert!(s.delete("custom-a"));
        assert!(!s.delete("custom-a"));
        drop(s);
        assert_eq!(ThemeStore::open(Some(dir.path())).list().len(), 1);
    }

    #[test]
    fn saving_an_existing_id_replaces_it_in_place() {
        let s = ThemeStore::open(None);
        s.save(theme("custom-a", "A"), 1).unwrap();
        s.save(theme("custom-b", "B"), 2).unwrap();
        s.save(theme("custom-a", "A2"), 3).unwrap();
        let list = s.list();
        assert_eq!(list[0].name, "A2");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn malformed_themes_are_refused() {
        let s = ThemeStore::open(None);
        assert!(s.save(theme("../etc", "x"), 0).is_err());
        assert!(s.save(theme("custom-a", "  "), 0).is_err());

        let mut t = theme("custom-a", "x");
        t.theme.remove("brightWhite");
        assert!(s.save(t, 0).unwrap_err().contains("brightWhite"));

        let mut t = theme("custom-a", "x");
        t.theme.insert("red".into(), "red".into());
        assert!(s.save(t, 0).is_err());

        let mut t = theme("custom-a", "x");
        t.overrides.insert("surface".into(), "#000000".into());
        assert!(s.save(t, 0).unwrap_err().contains("override"));

        assert!(s.list().is_empty());
    }

    #[test]
    fn a_corrupt_file_reads_as_no_themes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("themes.json"), "not json").unwrap();
        let s = ThemeStore::open(Some(dir.path()));
        assert!(s.list().is_empty());
        s.save(theme("custom-a", "A"), 1).unwrap();
        assert_eq!(ThemeStore::open(Some(dir.path())).list().len(), 1);
    }

    #[test]
    fn the_store_is_bounded() {
        let s = ThemeStore::open(None);
        for i in 0..MAX_THEMES {
            s.save(theme(&format!("custom-{i}"), "t"), 0).unwrap();
        }
        assert!(s.save(theme("custom-x", "t"), 0).is_err());
        assert!(
            s.save(theme("custom-0", "t"), 0).is_ok(),
            "replacing still works when full"
        );
    }
}
