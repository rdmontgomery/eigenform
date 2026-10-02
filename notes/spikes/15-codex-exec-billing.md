# 15 — codex exec billing

**Claim:** `codex exec` signed in with a ChatGPT plan (`codex login`, not an API key) draws on the plan's usage limits, the same pool as the interactive TUI. It doesn't bill the OpenAI API per token.
**Status:** PENDING
**codex version:** _fill in_
**Date:** 2026-10-02

## Why it matters

Spike 04 exists because Claude's `--print` flips to usage billing. Fanning out several `codex exec` workers is only cheap if the same flip doesn't happen on the Codex side.

## Procedure

```sh
codex login status                       # expect: logged in using ChatGPT (not an API key)
env | grep -i OPENAI_API_KEY             # must be empty, or exec may prefer the key
# Note the plan's remaining usage: the interactive TUI's /status, or the ChatGPT usage page.
codex exec "print the numbers 1 to 5"    # one tiny run
# Re-check /status (TUI) and the API billing page (platform.openai.com/usage).
```

## Result

_Paste real output._

## Implication

- **CONFIRMED:** parallel workers cost plan quota only. The skill's default cap of 3 concurrent workers is about rate limits, not money.
- **REFUTED:** say so prominently in the skill, and make `codex-worker spawn` refuse when `OPENAI_API_KEY` is set unless `--allow-api-billing` is passed.
