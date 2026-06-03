# 05 — Bundled skills (in the claude binary?)

**Claim:** Skills shown in the harness's available-skills list that have no disk footprint (e.g. `update-config`, `simplify`, `loop`, `schedule`, `claude-api`, `keybindings-help`, `fewer-permission-prompts`, `init`, `review`, `security-review`) are bundled inside the `claude` CLI executable and not resolvable from `~/.claude/`.
**Status:** PENDING
**claude version:** 2.1.161
**Date:** —

## Why this matters

`eigen skills tree` discovers 166+ skill contributions across the disk-resident layouts but does NOT see the bundled set. For the audit goal — "did a local skill accidentally become global or vice-versa" — that's fine: bundled skills aren't move-able anyway. But for "list all skills available right now" (which the user expects from `tree`), it's a known gap. Document it; surface it in the rendered output.

## Procedure

1. `strings $(which claude) | grep -iE 'update-config|simplify|keybindings' | head -20`
2. Read the binary's `--help` for any flag like `--list-skills` or `--print-skill`.
3. Check whether `~/.claude/plugins/` has a JSON manifest that lists bundled-but-not-on-disk skills.
4. Examine `~/.local/share/claude/` or other non-`~/.claude` locations.

## Result

(paste real output here)

## Implication

If CONFIRMED: `eigen skills tree` adds a `Layer::Bundled` entry whose paths point inside the claude binary (or an empty path) and whose source is documented as "shipped with claude $version". `eigen skills audit` will not flag mismatches between bundled skills and disk locations.

If REFUTED: there's another disk source we're not scanning yet. Add to `canonical_roots` and re-test.
