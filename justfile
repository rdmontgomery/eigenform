# eigen — canonical command invocations.
# Engine-touching targets are human-triggered. CI never runs them.

# Default target lists available recipes.
default:
    @just --list

# --- spikes --------------------------------------------------------------
# These spawn `claude` interactively against your real plan. Run by hand.

# spike 02 — tail fork via --fork-session.
# Sets up before/after snapshots so you can diff the projects dir.
spike-02 src_uuid:
    @echo "spike-02: see notes/spikes/02-tail-fork.md"
    @echo "snapshotting projects dir BEFORE fork ..."
    ls ~/.claude/projects/-home-rdmontgomery-projects-eigen/*.jsonl | sort > /tmp/eigen-spike-02-before.txt
    @echo "now launching: claude --resume {{src_uuid}} --fork-session"
    @echo "(send one trivial turn, then /exit)"
    claude --resume {{src_uuid}} --fork-session
    @echo "snapshotting projects dir AFTER fork ..."
    ls ~/.claude/projects/-home-rdmontgomery-projects-eigen/*.jsonl | sort > /tmp/eigen-spike-02-after.txt
    @echo "--- new files ---"
    diff /tmp/eigen-spike-02-before.txt /tmp/eigen-spike-02-after.txt || true
    @echo "record outcome in notes/spikes/02-tail-fork.md"

# spike 03 — mid-tree cold-load.
# This one needs editing notes/spikes/03 by hand. Justfile only enforces the
# safety check that you really mean it.
spike-03:
    @echo "spike-03 is hand-driven. see notes/spikes/03-mid-tree-cold-load.md"
    @echo "you'll need: a source session with a memorable mid-conversation fact,"
    @echo "the truncation turn uuid, and willingness to ask the resumed model"
    @echo "the same probe questions twice (once on source, once on fork)."

# spike 04 — billing flip.
# Pure observation; no command needed beyond your own dashboard checks.
spike-04:
    @echo "spike-04 is pure observation. see notes/spikes/04-billing-flip.md"
    @echo "track /cost before and after one --print invocation."

# --- testing -------------------------------------------------------------

# Safe default: unit tests only.
test:
    cargo test --workspace

# Live integration test (spawns real claude). Triple-gated.
# Requires: cargo feature 'live-claude' AND env EIGEN_ALLOW_LIVE_CLAUDE=1.
test-live:
    EIGEN_ALLOW_LIVE_CLAUDE=1 cargo test --workspace --features live-claude -- --ignored
