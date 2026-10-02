//! Path helpers shared by the pty spawner and the launcher's path probe.

use std::path::{Path, PathBuf};

/// Claude Code's project dir name for a cwd: `/` → `-` (e.g. `/home/me/p` → `-home-me-p`).
pub(crate) fn escaped_cwd(cwd: &str) -> String {
    cwd.replace('/', "-")
}

/// Expand a leading `~` or `~/…` to `$HOME` — the launcher input comes from a browser,
/// so there's no shell to do it. Non-tilde paths (and a missing `$HOME`) pass through
/// unchanged. Only a leading `~` is special; `~user` is not expanded.
pub(crate) fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Normalize a path by resolving `.` and `..` components without touching the filesystem
/// (i.e. without canonicalizing — the path may not exist yet). This is used to check
/// whether a `new=<cwd>&create=1` request is under `workspace_root` even when the
/// requested directory hasn't been created yet.
///
/// Rules:
/// - `.` components are skipped.
/// - `..` pops the last accumulated component (if any; at the root it is a no-op).
/// - All other components are pushed.
///
/// The input is treated as an absolute path. If it is relative, it is used as-is
/// (the containment check will likely fail since the root is always absolute).
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_resolves_dotdot() {
        // Normal path — unchanged.
        assert_eq!(normalize_path(Path::new("/a/b/c")), PathBuf::from("/a/b/c"));
        // Single `..` — pops one component.
        assert_eq!(
            normalize_path(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        // Double `..` — escapes the workspace.
        assert_eq!(
            normalize_path(Path::new("/workspace/child/../../outside")),
            PathBuf::from("/outside")
        );
        // `.` is skipped.
        assert_eq!(normalize_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
        // `..` at root is a no-op (no component to pop).
        assert_eq!(normalize_path(Path::new("/../x")), PathBuf::from("/x"));
    }

    #[test]
    fn expand_tilde_uses_home_for_leading_tilde_only() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        // Absolute and relative non-tilde paths pass through untouched.
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_tilde("relative/x"), PathBuf::from("relative/x"));
        // `~user` is NOT expanded (only a bare ~ or ~/).
        assert_eq!(expand_tilde("~bob/x"), PathBuf::from("~bob/x"));
        if let Some(home) = home {
            assert_eq!(expand_tilde("~"), home);
            assert_eq!(expand_tilde("~/src/proj"), home.join("src/proj"));
        }
    }
}
