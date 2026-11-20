//! Session links: `GET /s/{ref}` opens eigenform on one session, named by its uuid, a
//! unique prefix of it, or its peer name (what other Claude sessions message it by).
//! `GET /api/resolve/{ref}` answers the same lookup as JSON, for the app's own clicks.
//!
//! - one match, an app window connected: a `focus` event goes to that window (as with
//!   `POST /api/focus`) and the response is a short page that closes its own tab, or
//!   failing that says where the session went, with a link to open it in this tab
//!   instead
//! - one match, no window: 303 to `/?session=<uuid>`, which boots the app on it
//! - several matches: 300 with a page listing them, each linking to its full uuid
//! - none: 404 with a page saying nothing matched
//!
//! The candidates are the forest snapshot's rows (`GET /api/forest`), so a peer name only
//! resolves while its session runs: the name lives in the session's live claim. A full
//! scan takes seconds on a busy machine, so the rows of the forest stream's last scan
//! answer first; a reference they don't resolve to one session gets a fresh scan.

use axum::extract::{Path as AxumPath, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use crate::focus::Target;
use crate::AppState;

/// One session a reference can resolve to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub uuid: String,
    pub peer: Option<String>,
    pub title: Option<String>,
    pub cwd: String,
    pub live: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Resolution<'a> {
    Found(&'a Candidate),
    Ambiguous(Vec<&'a Candidate>),
    Unknown,
}

/// Resolve `reference` against `candidates`. A full uuid wins outright; otherwise every
/// session whose peer name equals it (ignoring case) or whose uuid starts with it is a
/// match, and only a single match resolves.
pub fn resolve<'a>(candidates: &'a [Candidate], reference: &str) -> Resolution<'a> {
    let r = reference.trim().to_ascii_lowercase();
    if r.is_empty() {
        return Resolution::Unknown;
    }
    if let Some(c) = candidates.iter().find(|c| c.uuid == r) {
        return Resolution::Found(c);
    }
    let mut hits: Vec<&Candidate> = Vec::new();
    for c in candidates {
        let named = c
            .peer
            .as_deref()
            .is_some_and(|p| p.eq_ignore_ascii_case(&r));
        if (named || c.uuid.starts_with(&r)) && !hits.iter().any(|h| h.uuid == c.uuid) {
            hits.push(c);
        }
    }
    match hits.len() {
        0 => Resolution::Unknown,
        1 => Resolution::Found(hits[0]),
        _ => Resolution::Ambiguous(hits),
    }
}

fn candidates(rows: &[serde_json::Value]) -> Vec<Candidate> {
    rows.iter()
        .filter_map(|r| {
            let s = |k: &str| r[k].as_str().map(str::to_string);
            Some(Candidate {
                uuid: s("uuid")?.to_ascii_lowercase(),
                peer: s("peer"),
                title: s("title"),
                cwd: s("cwd").unwrap_or_default(),
                live: r["live"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

async fn scan(state: &AppState) -> Vec<Candidate> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || candidates(&crate::forest::forest_rows(&state.config)))
        .await
        .unwrap_or_default()
}

/// Candidates for `reference`: the stream's last rows when they name exactly one session,
/// else a fresh scan.
async fn load(state: &AppState, reference: &str) -> Vec<Candidate> {
    if let Some(rows) = state.forest.fresh() {
        let cached = candidates(&rows);
        if matches!(resolve(&cached, reference), Resolution::Found(_)) {
            return cached;
        }
    }
    scan(state).await
}

fn candidate_json(c: &Candidate) -> serde_json::Value {
    serde_json::json!({
        "uuid": c.uuid,
        "peer": c.peer,
        "title": c.title,
        "cwd": c.cwd,
        "live": c.live,
    })
}

/// `GET /api/resolve/{ref}` — `{"session": {…}}` for one match, `{"candidates": […]}`
/// (300) for several, 404 for none.
pub(crate) async fn resolve_route(
    AxumPath(reference): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let all = load(&state, &reference).await;
    match resolve(&all, &reference) {
        Resolution::Found(c) => {
            Json(serde_json::json!({ "session": candidate_json(c) })).into_response()
        }
        Resolution::Ambiguous(hits) => (
            StatusCode::MULTIPLE_CHOICES,
            Json(serde_json::json!({
                "candidates": hits.into_iter().map(candidate_json).collect::<Vec<_>>(),
            })),
        )
            .into_response(),
        Resolution::Unknown => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no session matches", "ref": reference })),
        )
            .into_response(),
    }
}

/// `GET /s/{ref}` — see the module docs.
pub(crate) async fn session_link_route(
    AxumPath(reference): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let all = load(&state, &reference).await;
    match resolve(&all, &reference) {
        Resolution::Found(c) => {
            let open_here = format!("/?session={}", c.uuid);
            let target = Target::Session(c.uuid.clone());
            match state.focus.request_if_connected(&state.events, &target) {
                // The tab this link opened is only a relay: it closes itself at once.
                // A browser lets a script close a tab with one history entry (a link
                // opened from another app); otherwise the text stays, with a way in.
                Some(_) => page(
                    StatusCode::OK,
                    "Opened in eigenform",
                    &format!(
                        "<script>window.close()</script>\
                         <p>Sent <b>{}</b> to your eigenform window. You can close this tab.</p>\
                         <p><a href=\"{}\">Open it in this tab instead</a></p>",
                        esc(&label(c)),
                        esc(&open_here),
                    ),
                ),
                None => (StatusCode::SEE_OTHER, [(header::LOCATION, open_here)]).into_response(),
            }
        }
        Resolution::Ambiguous(hits) => {
            let items: String = hits
                .iter()
                .map(|c| {
                    format!(
                        "<li><a href=\"/s/{uuid}\"><code>{uuid}</code></a> {label}\
                         <span class=\"dim\">{cwd}{live}</span></li>",
                        uuid = esc(&c.uuid),
                        label = esc(&label(c)),
                        cwd = esc(&c.cwd),
                        live = if c.live { " · live" } else { "" },
                    )
                })
                .collect();
            page(
                StatusCode::MULTIPLE_CHOICES,
                "Which session?",
                &format!(
                    "<p><code>{}</code> matches {} sessions:</p><ul>{items}</ul>",
                    esc(&reference),
                    hits.len()
                ),
            )
        }
        Resolution::Unknown => page(
            StatusCode::NOT_FOUND,
            "No such session",
            &format!(
                "<p>No session has the id, id prefix or peer name <code>{}</code>.</p>\
                 <p>Peer names only resolve while their session runs. \
                 <a href=\"/\">Open eigenform</a></p>",
                esc(&reference)
            ),
        ),
    }
}

fn label(c: &Candidate) -> String {
    match (&c.peer, &c.title) {
        (Some(p), Some(t)) => format!("{p} — {t}"),
        (Some(p), None) => p.clone(),
        (None, Some(t)) => t.clone(),
        (None, None) => c.uuid[..8.min(c.uuid.len())].to_string(),
    }
}

fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>{t} · eigenform</title>\
         <style>body{{font:14px system-ui,sans-serif;max-width:46rem;margin:3rem auto;padding:0 1rem;\
         color:#ddd;background:#16161a}}a{{color:#8ab4f8}}li{{margin:.4rem 0}}\
         .dim{{color:#888;margin-left:.5rem}}code{{font-size:13px}}</style>\
         <h1>{t}</h1>{body}",
        t = esc(title),
    );
    (status, Html(html)).into_response()
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(uuid: &str, peer: Option<&str>) -> Candidate {
        Candidate {
            uuid: uuid.into(),
            peer: peer.map(Into::into),
            title: None,
            cwd: "/srv/app".into(),
            live: peer.is_some(),
        }
    }

    fn sessions() -> Vec<Candidate> {
        vec![
            c("aaaa1111-0000-4000-8000-000000000001", Some("app-dd")),
            c("aaaa2222-0000-4000-8000-000000000002", None),
            c("bbbb3333-0000-4000-8000-000000000003", Some("commander")),
            c("cccc4444-0000-4000-8000-000000000004", Some("aaaa")),
        ]
    }

    fn found(all: &[Candidate], r: &str) -> Option<String> {
        match resolve(all, r) {
            Resolution::Found(c) => Some(c.uuid.clone()),
            _ => None,
        }
    }

    #[test]
    fn a_full_uuid_resolves_in_any_case() {
        let all = sessions();
        let u = "bbbb3333-0000-4000-8000-000000000003";
        assert_eq!(found(&all, u).as_deref(), Some(u));
        assert_eq!(found(&all, &u.to_uppercase()).as_deref(), Some(u));
    }

    #[test]
    fn a_unique_prefix_resolves() {
        let all = sessions();
        assert_eq!(
            found(&all, "aaaa1").as_deref(),
            Some("aaaa1111-0000-4000-8000-000000000001")
        );
        assert_eq!(
            found(&all, "  BBBB ").as_deref(),
            Some("bbbb3333-0000-4000-8000-000000000003")
        );
    }

    #[test]
    fn a_peer_name_resolves_ignoring_case() {
        let all = sessions();
        assert_eq!(
            found(&all, "Commander").as_deref(),
            Some("bbbb3333-0000-4000-8000-000000000003")
        );
        assert_eq!(
            found(&all, "app-d"),
            None,
            "names match whole, not by prefix"
        );
    }

    #[test]
    fn a_shared_prefix_is_ambiguous_not_a_guess() {
        let all = sessions();
        let Resolution::Ambiguous(hits) = resolve(&all, "aaaa") else {
            panic!("expected ambiguous");
        };
        let uuids: Vec<&str> = hits.iter().map(|c| c.uuid.as_str()).collect();
        assert_eq!(
            uuids,
            [
                "aaaa1111-0000-4000-8000-000000000001",
                "aaaa2222-0000-4000-8000-000000000002",
                "cccc4444-0000-4000-8000-000000000004",
            ],
            "two id prefixes plus the session named `aaaa`"
        );
    }

    #[test]
    fn two_sessions_with_one_name_are_ambiguous() {
        let mut all = sessions();
        all.push(c("dddd5555-0000-4000-8000-000000000005", Some("commander")));
        assert!(matches!(resolve(&all, "commander"), Resolution::Ambiguous(h) if h.len() == 2));
    }

    #[test]
    fn nothing_matching_is_unknown() {
        let all = sessions();
        assert_eq!(resolve(&all, "zzz"), Resolution::Unknown);
        assert_eq!(resolve(&all, ""), Resolution::Unknown);
        assert_eq!(resolve(&all, "   "), Resolution::Unknown);
        assert_eq!(resolve(&[], "aaaa"), Resolution::Unknown);
    }

    #[test]
    fn a_duplicate_row_counts_once() {
        let mut all = sessions();
        all.push(c("bbbb3333-0000-4000-8000-000000000003", Some("commander")));
        assert!(matches!(resolve(&all, "bbbb"), Resolution::Found(_)));
    }

    #[test]
    fn page_text_is_escaped() {
        assert_eq!(
            esc("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }
}
