//! The session graph: which session launched which (lineage) and which sessions have
//! messaged each other (messages), over the live sessions and their immediate kin.
//!
//! - **Lineage** comes from `POST /api/pty`'s optional `parent` (the caller's session
//!   uuid). The daemon holds it against the new pty until the pty's uuid is known, then
//!   records `child → parent` in `<state_dir>/lineage.json`. Nothing is inferred: a
//!   session launched without `parent` has no recorded parent.
//! - **Messages** come from each receiver's transcript (the digest's peer inbox), which
//!   names the sender only by peer name and pid. Claude Code deletes a session's claim
//!   when it exits, so the daemon keeps a ledger of every (name, pid) → uuid it has seen
//!   in `<state_dir>/peer-names.json`. A sender the ledger never saw stays a name-only
//!   "ghost" node.
//!
//! Scope: every live session, plus any session one edge away from a live one; edges are
//! drawn between the sessions in scope. Message edges count deliveries within
//! [`MESSAGE_WINDOW_HOURS`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Messages older than this neither draw an edge nor pull a dead session into scope.
pub const MESSAGE_WINDOW_HOURS: i64 = 72;
/// Bounds on the persisted files, so a long-lived state dir can't grow without limit.
const MAX_LINKS: usize = 5000;
const MAX_LEDGER: usize = 5000;

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// One session as the graph sees it (a forest row, reduced).
#[derive(Debug, Clone, Default)]
pub struct GraphSession {
    pub uuid: String,
    pub title: Option<String>,
    pub cwd: String,
    pub live: bool,
    pub state: String,
    pub peer: Option<String>,
    pub pid: Option<u32>,
    /// Messages other sessions sent this one: (sender name, sender pid, at).
    pub inbox: Vec<(String, Option<u32>, String)>,
}

/// `child` was launched by `parent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub child: String,
    pub parent: String,
    /// Epoch milliseconds.
    pub at: i64,
}

/// A peer name a session was seen holding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    pub name: String,
    pub pid: Option<u32>,
    pub uuid: String,
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    /// The session uuid, or `peer:<name>` for a ghost.
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub live: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// A sender known only by its peer name: no session to open.
    pub ghost: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineageEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageEdge {
    pub from: String,
    pub to: String,
    pub count: u32,
    /// The latest delivery (RFC 3339).
    pub last: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub lineage: Vec<LineageEdge>,
    pub messages: Vec<MessageEdge>,
}

// ---------------------------------------------------------------------------
// Derivation
// ---------------------------------------------------------------------------

fn ghost_id(name: &str) -> String {
    format!("peer:{name}")
}

fn short(uuid: &str) -> String {
    uuid.chars().take(8).collect()
}

/// Who sent a message: the uuid that held (`name`, `pid`), else the one session that ever
/// held `name`, else a ghost named for it.
fn resolve_sender(
    name: &str,
    pid: Option<u32>,
    sessions: &[GraphSession],
    ledger: &[Alias],
) -> String {
    let exact = sessions
        .iter()
        .find(|s| s.peer.as_deref() == Some(name) && pid.is_some() && s.pid == pid)
        .map(|s| s.uuid.clone())
        .or_else(|| {
            ledger
                .iter()
                .find(|a| a.name == name && pid.is_some() && a.pid == pid)
                .map(|a| a.uuid.clone())
        });
    if let Some(u) = exact {
        return u;
    }
    let holders: BTreeSet<&str> = sessions
        .iter()
        .filter(|s| s.peer.as_deref() == Some(name))
        .map(|s| s.uuid.as_str())
        .chain(
            ledger
                .iter()
                .filter(|a| a.name == name)
                .map(|a| a.uuid.as_str()),
        )
        .collect();
    match holders.len() {
        1 => holders.into_iter().next().unwrap().to_string(),
        _ => ghost_id(name),
    }
}

/// Build the graph over the live sessions and their immediate kin. Pure: the caller
/// supplies the forest rows, the recorded lineage and the peer-name ledger.
pub fn derive(
    sessions: &[GraphSession],
    lineage: &[Link],
    ledger: &[Alias],
    now: DateTime<Utc>,
) -> Graph {
    let by_uuid: HashMap<&str, &GraphSession> =
        sessions.iter().map(|s| (s.uuid.as_str(), s)).collect();
    let live = |id: &str| by_uuid.get(id).is_some_and(|s| s.live);
    let cutoff = now - chrono::Duration::hours(MESSAGE_WINDOW_HOURS);

    // Every candidate edge, before scoping.
    let mut lin: BTreeSet<(String, String)> = BTreeSet::new();
    for l in lineage {
        if l.child != l.parent {
            lin.insert((l.parent.clone(), l.child.clone()));
        }
    }
    let mut msgs: BTreeMap<(String, String), (u32, String)> = BTreeMap::new();
    for s in sessions {
        for (name, pid, at) in &s.inbox {
            let recent = DateTime::parse_from_rfc3339(at)
                .map(|t| t.with_timezone(&Utc) >= cutoff)
                .unwrap_or(false);
            if !recent {
                continue;
            }
            let from = resolve_sender(name, *pid, sessions, ledger);
            if from == s.uuid {
                continue;
            }
            let e = msgs
                .entry((from, s.uuid.clone()))
                .or_insert((0, String::new()));
            e.0 += 1;
            if at.as_str() > e.1.as_str() {
                e.1 = at.clone();
            }
        }
    }

    // Scope: live sessions, plus anything one edge from one.
    let mut scope: BTreeSet<String> = sessions
        .iter()
        .filter(|s| s.live)
        .map(|s| s.uuid.clone())
        .collect();
    let pairs = lin.iter().chain(msgs.keys());
    for (a, b) in pairs {
        if live(a) {
            scope.insert(b.clone());
        }
        if live(b) {
            scope.insert(a.clone());
        }
    }

    let nodes = scope
        .iter()
        .map(
            |id| match (by_uuid.get(id.as_str()), id.strip_prefix("peer:")) {
                (Some(s), _) => Node {
                    id: id.clone(),
                    label: s
                        .peer
                        .clone()
                        .or_else(|| s.title.clone())
                        .unwrap_or_else(|| short(id)),
                    title: s.title.clone(),
                    cwd: Some(s.cwd.clone()),
                    live: s.live,
                    state: Some(s.state.clone()),
                    ghost: false,
                },
                (None, Some(name)) => Node {
                    id: id.clone(),
                    label: name.to_string(),
                    title: None,
                    cwd: None,
                    live: false,
                    state: None,
                    ghost: true,
                },
                // A recorded parent or sender the forest no longer lists: still a session.
                (None, None) => Node {
                    id: id.clone(),
                    label: ledger
                        .iter()
                        .rev()
                        .find(|a| &a.uuid == id)
                        .map(|a| a.name.clone())
                        .unwrap_or_else(|| short(id)),
                    title: None,
                    cwd: None,
                    live: false,
                    state: None,
                    ghost: false,
                },
            },
        )
        .collect();

    let inside = |a: &str, b: &str| scope.contains(a) && scope.contains(b);
    Graph {
        nodes,
        lineage: lin
            .into_iter()
            .filter(|(a, b)| inside(a, b))
            .map(|(from, to)| LineageEdge { from, to })
            .collect(),
        messages: msgs
            .into_iter()
            .filter(|((a, b), _)| inside(a, b))
            .map(|((from, to), (count, last))| MessageEdge {
                from,
                to,
                count,
                last,
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: Option<&Path>) -> T {
    path.and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: Option<&Path>, value: &T) {
    let Some(path) = path else { return };
    let Ok(bytes) = serde_json::to_vec(value) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Pty id → (parent uuid, epoch ms) for launches whose pty has no uuid yet.
type Pending = HashMap<String, (String, i64)>;

/// Recorded lineage: links persisted under `state_dir`, plus parents named at launch whose
/// pty hasn't learned its session uuid yet.
pub struct LineageStore {
    path: Option<PathBuf>,
    inner: Mutex<(Vec<Link>, Pending)>,
}

impl LineageStore {
    pub fn open(state_dir: Option<&Path>) -> Self {
        let path = state_dir.map(|d| d.join("lineage.json"));
        let links: Vec<Link> = read_json(path.as_deref());
        Self {
            path,
            inner: Mutex::new((links, HashMap::new())),
        }
    }

    /// Pty `pty_id` was launched by session `parent`; recorded once its uuid is known.
    pub fn launched(&self, pty_id: &str, parent: &str, now_ms: i64) {
        self.lock()
            .1
            .insert(pty_id.to_string(), (parent.to_string(), now_ms));
    }

    /// Record every pending launch whose pty now has a uuid. `uuid_of` maps a pty id to
    /// its session uuid; a pty gone from the host drops its pending entry.
    pub fn settle(&self, uuid_of: &HashMap<String, Option<String>>) {
        let mut g = self.lock();
        let (links, pending) = &mut *g;
        let mut changed = false;
        pending.retain(|pty, (parent, at)| match uuid_of.get(pty) {
            Some(Some(child)) => {
                if !links.iter().any(|l| &l.child == child) {
                    links.push(Link {
                        child: child.clone(),
                        parent: parent.clone(),
                        at: *at,
                    });
                    changed = true;
                }
                false
            }
            Some(None) => true,
            None => false,
        });
        if changed {
            if links.len() > MAX_LINKS {
                let excess = links.len() - MAX_LINKS;
                links.drain(..excess);
            }
            write_json(self.path.as_deref(), links);
        }
    }

    pub fn links(&self) -> Vec<Link> {
        self.lock().0.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Vec<Link>, Pending)> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Add the (name, pid) → uuid pairs `sessions` hold now to the ledger at
/// `<state_dir>/peer-names.json`, writing only when one is new. Returns the ledger.
pub fn observe_aliases(state_dir: Option<&Path>, sessions: &[GraphSession]) -> Vec<Alias> {
    let path = state_dir.map(|d| d.join("peer-names.json"));
    let mut ledger: Vec<Alias> = read_json(path.as_deref());
    let mut changed = false;
    for s in sessions {
        let Some(name) = &s.peer else { continue };
        let alias = Alias {
            name: name.clone(),
            pid: s.pid,
            uuid: s.uuid.clone(),
        };
        if !ledger.contains(&alias) {
            ledger.push(alias);
            changed = true;
        }
    }
    if changed {
        if ledger.len() > MAX_LEDGER {
            let excess = ledger.len() - MAX_LEDGER;
            ledger.drain(..excess);
        }
        write_json(path.as_deref(), &ledger);
    }
    ledger
}

/// A forest row as the graph's input.
pub fn graph_session(s: &eigenform_forest::LiveSession) -> GraphSession {
    GraphSession {
        uuid: s.uuid.clone(),
        title: s.title.clone(),
        cwd: s.cwd.display().to_string(),
        live: s.live,
        state: s.state.as_str().to_string(),
        peer: s.peer.clone(),
        pid: s.pid,
        inbox: s
            .peers
            .iter()
            .map(|m| (m.from.clone(), m.pid, m.at.clone()))
            .collect(),
    }
}

/// Settle pending launches against the host's ptys.
pub(crate) fn settle_lineage(state: &AppState) {
    let uuid_of: HashMap<String, Option<String>> = state
        .host
        .list()
        .iter()
        .map(|l| (l.id.to_string(), l.uuid()))
        .collect();
    state.lineage.settle(&uuid_of);
}

/// The graph from the last forest scan (see [`store`]).
#[derive(Default)]
pub struct GraphSlot(Mutex<Option<Graph>>);

/// Derive the graph from one forest scan and keep it for `GET /api/graph`.
pub(crate) fn store(state: &AppState, scan: &[eigenform_forest::LiveSession]) -> Graph {
    let sessions: Vec<GraphSession> = scan.iter().map(graph_session).collect();
    let ledger = observe_aliases(state.config.state_dir.as_deref(), &sessions);
    let graph = derive(&sessions, &state.lineage.links(), &ledger, Utc::now());
    *state.graph.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(graph.clone());
    graph
}

/// `GET /api/graph` — `{nodes, lineage, messages}` over the live sessions and their kin.
/// See the module docs for where each edge comes from. Served from the last forest
/// push's scan while the app has the forest stream open; scans only when nothing has.
pub(crate) async fn graph_route(State(state): State<AppState>) -> Response {
    settle_lineage(&state);
    let cached = state
        .graph
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let graph = match cached {
        Some(g) => g,
        None => {
            let st = state.clone();
            tokio::task::spawn_blocking(move || store(&st, &crate::forest::scan(&st.config)))
                .await
                .unwrap_or_default()
        }
    };
    axum::Json(graph).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0000aaaa-0000-0000-0000-000000000001";
    const B: &str = "0000bbbb-0000-0000-0000-000000000002";
    const C: &str = "0000cccc-0000-0000-0000-000000000003";
    const D: &str = "0000dddd-0000-0000-0000-000000000004";
    const OLD: &str = "0000eeee-0000-0000-0000-000000000005";

    fn now() -> DateTime<Utc> {
        "2026-01-02T00:00:00Z".parse().unwrap()
    }

    fn s(uuid: &str, live: bool, peer: Option<&str>, pid: Option<u32>) -> GraphSession {
        GraphSession {
            uuid: uuid.into(),
            title: Some(format!("title {}", &uuid[..8])),
            cwd: "/work/app".into(),
            live,
            state: if live { "ready" } else { "recent" }.into(),
            peer: peer.map(Into::into),
            pid,
            inbox: Vec::new(),
        }
    }

    fn link(child: &str, parent: &str) -> Link {
        Link {
            child: child.into(),
            parent: parent.into(),
            at: 0,
        }
    }

    fn ids(g: &Graph) -> Vec<&str> {
        g.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn lineage_keeps_live_sessions_and_their_parents_and_children() {
        // A (live) launched B (dead) which launched C (dead): C is two hops from a live
        // session, so it and the B→C edge stay out. D (dead) launched A: in.
        let sessions = [
            s(A, true, Some("orch-1"), Some(1)),
            s(B, false, None, None),
            s(C, false, None, None),
            s(D, false, None, None),
        ];
        let links = [link(B, A), link(C, B), link(A, D)];
        let g = derive(&sessions, &links, &[], now());
        assert_eq!(ids(&g), vec![A, B, D]);
        assert_eq!(
            g.lineage,
            vec![
                LineageEdge {
                    from: A.into(),
                    to: B.into()
                },
                LineageEdge {
                    from: D.into(),
                    to: A.into()
                },
            ]
        );
        let a = &g.nodes[0];
        assert_eq!((a.label.as_str(), a.live, a.ghost), ("orch-1", true, false));
        assert_eq!(g.nodes[1].label, format!("title {}", &B[..8]));
    }

    #[test]
    fn a_recorded_parent_the_forest_no_longer_lists_is_still_a_node() {
        let sessions = [s(A, true, None, None)];
        let g = derive(&sessions, &[link(A, OLD)], &[], now());
        let old = g.nodes.iter().find(|n| n.id == OLD).unwrap();
        assert_eq!(
            (old.label.as_str(), old.live, old.ghost),
            ("0000eeee", false, false)
        );
    }

    #[test]
    fn messages_resolve_senders_by_name_and_pid_then_count_per_direction() {
        let mut b = s(B, true, Some("worker-2"), Some(22));
        b.inbox = vec![
            ("orch-1".into(), Some(11), "2026-01-01T10:00:00Z".into()),
            ("orch-1".into(), Some(11), "2026-01-01T12:00:00Z".into()),
        ];
        let mut a = s(A, true, Some("orch-1"), Some(11));
        a.inbox = vec![("worker-2".into(), Some(22), "2026-01-01T11:00:00Z".into())];
        let g = derive(&[a, b], &[], &[], now());
        assert_eq!(
            g.messages,
            vec![
                MessageEdge {
                    from: A.into(),
                    to: B.into(),
                    count: 2,
                    last: "2026-01-01T12:00:00Z".into()
                },
                MessageEdge {
                    from: B.into(),
                    to: A.into(),
                    count: 1,
                    last: "2026-01-01T11:00:00Z".into()
                },
            ]
        );
    }

    #[test]
    fn an_exited_sender_resolves_through_the_ledger() {
        // C has exited: its claim (and so its name) is gone from the forest, but the
        // ledger saw it holding "old-3" under pid 33.
        let mut a = s(A, true, Some("orch-1"), Some(11));
        a.inbox = vec![("old-3".into(), Some(33), "2026-01-01T09:00:00Z".into())];
        let ledger = [Alias {
            name: "old-3".into(),
            pid: Some(33),
            uuid: C.into(),
        }];
        let g = derive(&[a, s(C, false, None, None)], &[], &ledger, now());
        assert_eq!(ids(&g), vec![A, C]);
        assert_eq!(g.messages[0].from, C);
    }

    #[test]
    fn a_reused_name_with_another_pid_matches_the_pid_not_the_newest_holder() {
        let mut a = s(A, true, Some("orch-1"), Some(11));
        a.inbox = vec![("dup".into(), Some(40), "2026-01-01T09:00:00Z".into())];
        let ledger = [
            Alias {
                name: "dup".into(),
                pid: Some(40),
                uuid: C.into(),
            },
            Alias {
                name: "dup".into(),
                pid: Some(41),
                uuid: D.into(),
            },
        ];
        let g = derive(&[a], &[], &ledger, now());
        assert_eq!(g.messages[0].from, C);
    }

    #[test]
    fn an_unknown_sender_is_a_ghost_named_for_its_peer_name() {
        let mut a = s(A, true, None, None);
        a.inbox = vec![
            ("stranger".into(), Some(99), "2026-01-01T09:00:00Z".into()),
            // Ambiguous: two sessions held this name, and the pid matches neither.
            ("dup".into(), Some(7), "2026-01-01T09:30:00Z".into()),
        ];
        let ledger = [
            Alias {
                name: "dup".into(),
                pid: Some(40),
                uuid: C.into(),
            },
            Alias {
                name: "dup".into(),
                pid: Some(41),
                uuid: D.into(),
            },
        ];
        let g = derive(&[a], &[], &ledger, now());
        let ghost = g.nodes.iter().find(|n| n.id == "peer:stranger").unwrap();
        assert!(ghost.ghost && !ghost.live);
        assert_eq!(ghost.label, "stranger");
        assert!(g.nodes.iter().any(|n| n.id == "peer:dup"));
    }

    #[test]
    fn old_messages_and_dead_pairs_draw_nothing() {
        let mut a = s(A, true, None, None);
        a.inbox = vec![("old-3".into(), Some(33), "2025-12-01T00:00:00Z".into())];
        // Two dead sessions messaging each other are out of scope.
        let mut b = s(B, false, Some("b"), None);
        b.inbox = vec![("c".into(), None, "2026-01-01T09:00:00Z".into())];
        let c = s(C, false, Some("c"), None);
        let g = derive(&[a, b, c], &[], &[], now());
        assert_eq!(ids(&g), vec![A]);
        assert!(g.messages.is_empty());
    }

    #[test]
    fn edges_between_two_in_scope_dead_sessions_are_kept() {
        // Live A launched dead B and C: both are one hop from A, so the B→C message
        // between them is drawn too.
        let mut c = s(C, false, Some("c"), None);
        c.inbox = vec![("b".into(), None, "2026-01-01T09:00:00Z".into())];
        let sessions = [s(A, true, None, None), s(B, false, Some("b"), None), c];
        let g = derive(&sessions, &[link(B, A), link(C, A)], &[], now());
        assert_eq!(g.messages.len(), 1);
        assert_eq!(
            (g.messages[0].from.as_str(), g.messages[0].to.as_str()),
            (B, C)
        );
    }

    #[test]
    fn lineage_settles_once_the_pty_has_a_uuid_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let store = LineageStore::open(Some(dir.path()));
        store.launched("7", A, 5);
        store.launched("8", A, 6);
        store.settle(&HashMap::from([
            ("7".to_string(), None),
            ("8".to_string(), None),
        ]));
        assert!(store.links().is_empty());
        // Pty 7 learns its uuid; pty 8 is gone from the host.
        store.settle(&HashMap::from([("7".to_string(), Some(B.to_string()))]));
        assert_eq!(
            store.links(),
            vec![Link {
                child: B.into(),
                parent: A.into(),
                at: 5
            }]
        );
        let reopened = LineageStore::open(Some(dir.path()));
        assert_eq!(reopened.links(), store.links());
        // Pty 8's pending entry was dropped, not kept waiting forever.
        store.settle(&HashMap::from([("8".to_string(), Some(C.to_string()))]));
        assert_eq!(store.links().len(), 1);
    }

    #[test]
    fn the_ledger_adds_new_aliases_only() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = [s(A, true, Some("orch-1"), Some(11)), s(B, true, None, None)];
        let first = observe_aliases(Some(dir.path()), &sessions);
        assert_eq!(
            first,
            vec![Alias {
                name: "orch-1".into(),
                pid: Some(11),
                uuid: A.into()
            }]
        );
        let again = observe_aliases(Some(dir.path()), &sessions);
        assert_eq!(again, first);
    }
}
