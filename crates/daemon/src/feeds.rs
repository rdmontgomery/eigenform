//! Feeds: local providers that list what needs the operator (`docs/plans/2026-11-12-workers-and-feeds-design.md`).
//!
//! `GET /api/feeds` reads `feeds.json`, fetches every loopback feed concurrently and keeps
//! a snapshot of what each advertised. `POST /api/feeds/act` names an item and an action
//! index, never a URL: the daemon POSTs only what a provider put in its snapshot, so a
//! page that reaches the daemon can't use it as a proxy for arbitrary localhost requests.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::AppState;

const FETCH_TIMEOUT: Duration = Duration::from_secs(3);
const ACT_TIMEOUT: Duration = Duration::from_secs(30);
const ERROR_BYTES: usize = 300;

#[derive(Debug, Clone, PartialEq)]
pub struct FeedConfig {
    pub name: String,
    pub url: String,
    /// `http://host[:port]`, what an action's `post` path resolves against.
    pub origin: Result<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Action {
    pub label: String,
    pub post: String,
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub actions: Vec<Action>,
}

/// The last good fetch of each feed: its origin and the items it advertised.
#[derive(Default)]
pub struct FeedHub {
    snapshot: Mutex<HashMap<String, (String, Vec<Item>)>>,
}

impl FeedHub {
    fn store(&self, name: &str, origin: &str, items: Vec<Item>) {
        self.snapshot
            .lock()
            .unwrap()
            .insert(name.to_string(), (origin.to_string(), items));
    }

    /// The URL and body an advertised action posts, or None when it isn't in the snapshot.
    pub fn resolve(&self, feed: &str, item: &str, action: usize) -> Option<(String, Value)> {
        let snap = self.snapshot.lock().unwrap();
        let (origin, items) = snap.get(feed)?;
        let a = items.iter().find(|i| i.id == item)?.actions.get(action)?;
        Some((format!("{origin}{}", a.post), a.body.clone()))
    }
}

/// `http://` on a loopback host, nothing else. Answers the origin (`http://host[:port]`).
pub fn loopback_origin(url: &str) -> Result<String, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("url must start with http://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err("url must not carry credentials".into());
    }
    let host = crate::pty::host_of(authority);
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(format!("url host {host:?} is not loopback"));
    }
    let after_host = match authority.split_once(']') {
        Some((_, rest)) if authority.starts_with('[') => rest,
        _ => &authority[host.len()..],
    };
    if !after_host.is_empty() {
        let port = after_host.strip_prefix(':').unwrap_or("");
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err("url port is not a number".into());
        }
    }
    Ok(format!("http://{authority}"))
}

/// Read `feeds.json`. A missing file is no feeds; a malformed one is an error.
pub fn load_config(path: &Path) -> Result<Vec<FeedConfig>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("feeds.json: {e}")),
    };
    #[derive(Deserialize)]
    struct Entry {
        name: String,
        url: String,
    }
    let entries: Vec<Entry> =
        serde_json::from_str(&text).map_err(|e| format!("feeds.json: {e}"))?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for e in entries {
        let valid_name = !e.name.is_empty()
            && e.name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid_name || !seen.insert(e.name.clone()) {
            return Err(format!(
                "feeds.json: feed name {:?} must be unique and [a-z0-9-]+",
                e.name
            ));
        }
        let origin = loopback_origin(&e.url);
        out.push(FeedConfig {
            name: e.name,
            url: e.url,
            origin,
        });
    }
    Ok(out)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The items of a feed document. A bad item is dropped on its own; a bad optional field
/// or a bad action is dropped from its item, which keeps the rest.
pub fn parse_items(doc: &Value) -> Vec<Item> {
    let Some(items) = doc.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| {
            let link =
                text(v, "link").filter(|l| l.starts_with("https://") || l.starts_with("http://"));
            let at = text(v, "at").filter(|a| chrono::DateTime::parse_from_rfc3339(a).is_ok());
            let actions = v
                .get("actions")
                .and_then(Value::as_array)
                .map(|acts| {
                    acts.iter()
                        .filter_map(|a| {
                            let post = text(a, "post")
                                .filter(|p| p.starts_with('/') && !p.starts_with("//"))?;
                            Some(Action {
                                label: text(a, "label")?,
                                post,
                                body: a.get("body").cloned().unwrap_or_else(|| json!({})),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Item {
                id: text(v, "id")?,
                kind: text(v, "kind")?,
                title: text(v, "title")?,
                detail: text(v, "detail"),
                link,
                at,
                actions,
            })
        })
        .collect()
}

/// No proxy, whatever the environment says: a loopback request must stay on the machine.
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into()
}

fn fetch(url: &str) -> Result<Vec<Item>, String> {
    let mut resp = agent(FETCH_TIMEOUT)
        .get(url)
        .call()
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    let doc: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("not a feed document: {e}"))?;
    Ok(parse_items(&doc))
}

/// `GET /api/feeds`.
pub(crate) async fn feeds_route(State(state): State<AppState>) -> Response {
    let Some(path) = state.config.feeds_file.clone() else {
        return Json(json!({ "feeds": [] })).into_response();
    };
    let configs = match load_config(&path) {
        Ok(c) => c,
        Err(e) => return Json(json!({ "feeds": [], "error": e })).into_response(),
    };
    let fetches = configs.into_iter().map(|cfg| {
        let hub = state.feeds.clone();
        async move {
            let origin = match &cfg.origin {
                Ok(o) => o.clone(),
                Err(e) => return json!({ "name": cfg.name, "ok": false, "error": e, "items": [] }),
            };
            let url = cfg.url.clone();
            let result = tokio::task::spawn_blocking(move || fetch(&url))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            match result {
                Ok(items) => {
                    hub.store(&cfg.name, &origin, items.clone());
                    json!({ "name": cfg.name, "ok": true, "items": items })
                }
                Err(e) => json!({ "name": cfg.name, "ok": false, "error": e, "items": [] }),
            }
        }
    });
    let feeds = futures_util::future::join_all(fetches).await;
    Json(json!({ "feeds": feeds })).into_response()
}

#[derive(Deserialize)]
struct ActBody {
    feed: String,
    item: String,
    action: usize,
}

/// `POST /api/feeds/act` — local origin and a JSON content type, like the plan gate's
/// decision route: the content type forces a CORS preflight this daemon never grants.
pub(crate) async fn feeds_act_route(
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
    let Ok(b) = serde_json::from_slice::<ActBody>(&body) else {
        return (StatusCode::BAD_REQUEST, "bad act body").into_response();
    };
    let Some((url, payload)) = state.feeds.resolve(&b.feed, &b.item, b.action) else {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "refresh and try again" })),
        )
            .into_response();
    };
    let result = tokio::task::spawn_blocking(move || {
        let mut resp = agent(ACT_TIMEOUT)
            .post(&url)
            .header("content-type", "application/json")
            .send(payload.to_string())
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        Ok::<_, String>((status, text))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok((status, _)) if (200..300).contains(&status) => {
            Json(json!({ "ok": true, "status": status })).into_response()
        }
        Ok((status, text)) => {
            let mut end = text.len().min(ERROR_BYTES);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "ok": false, "status": status, "error": &text[..end] })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "ok": false, "error": e })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_http_is_a_feed_origin() {
        assert_eq!(
            loopback_origin("http://127.0.0.1:8090/local/feed").unwrap(),
            "http://127.0.0.1:8090"
        );
        assert_eq!(
            loopback_origin("http://localhost/feed").unwrap(),
            "http://localhost"
        );
        assert_eq!(
            loopback_origin("http://[::1]:9/x").unwrap(),
            "http://[::1]:9"
        );
        for bad in [
            "https://127.0.0.1/feed",
            "http://example.com/feed",
            "http://127.0.0.1.evil.com/feed",
            "http://user@127.0.0.1/feed",
            "http://127.0.0.1:x/feed",
            "http://localhostx/feed",
            "ftp://127.0.0.1/",
        ] {
            assert!(loopback_origin(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn a_missing_file_is_no_feeds_and_names_must_be_unique() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("feeds.json");
        assert_eq!(load_config(&path).unwrap(), Vec::new());
        std::fs::write(
            &path,
            r#"[{"name":"a","url":"http://127.0.0.1:1/f"},{"name":"a","url":"http://127.0.0.1:2/f"}]"#,
        )
        .unwrap();
        assert!(load_config(&path).is_err());
        std::fs::write(&path, r#"[{"name":"b","url":"http://example.com/f"}]"#).unwrap();
        let cfg = load_config(&path).unwrap();
        assert!(cfg[0].origin.is_err());
    }

    #[test]
    fn bad_items_and_fields_drop_on_their_own() {
        let doc = json!({ "items": [
            { "id": "1", "kind": "review", "title": "ok",
              "link": "javascript:alert(1)", "at": "yesterday",
              "actions": [
                { "label": "Go", "post": "/api/x", "body": { "k": 1 } },
                { "label": "Escape", "post": "//evil.com/x" },
                { "label": "Abs", "post": "http://127.0.0.1:1/x" },
                { "post": "/no-label" },
                { "label": "Bare", "post": "/api/y" }
              ] },
            { "id": "2", "kind": "review" },
            { "kind": "review", "title": "no id" },
            "junk"
        ]});
        let items = parse_items(&doc);
        assert_eq!(items.len(), 1);
        let i = &items[0];
        assert_eq!(i.link, None);
        assert_eq!(i.at, None);
        assert_eq!(i.actions.len(), 2);
        assert_eq!(i.actions[0].body, json!({ "k": 1 }));
        assert_eq!(i.actions[1].body, json!({}));
    }
}
