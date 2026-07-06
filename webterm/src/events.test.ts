// Tests for events.ts pure helpers — summary + time formatting. Run: `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { eventSummary, fmtEventTime, shortId } from "./events.ts";
import type { EventRecord } from "./types.ts";

function ev(kind: string, data: Record<string, unknown>): EventRecord {
  return { seq: 1, at: "2026-07-03T12:34:56.000Z", kind, data };
}

test("shortId takes the first 8 chars of a string, else empty", () => {
  assert.equal(shortId("abcdef00-0000-4000-8000-000000000000"), "abcdef00");
  assert.equal(shortId("short"), "short");
  assert.equal(shortId(undefined), "");
  assert.equal(shortId(42), "");
});

test("fmtEventTime formats HH:MM:SS and rejects garbage", () => {
  // Format against a fixed UTC instant via its local rendering — assert the shape,
  // not a timezone-specific value.
  assert.match(fmtEventTime("2026-07-03T12:34:56.000Z"), /^\d{2}:\d{2}:\d{2}$/);
  assert.equal(fmtEventTime("not a date"), "");
  assert.equal(fmtEventTime(""), "");
});

test("pty-spawned summarizes id, program, and cwd", () => {
  assert.equal(
    eventSummary(ev("pty-spawned", { id: "7", program: "claude", cwd: "/home/me/proj" })),
    "pty 7 · claude · /home/me/proj",
  );
  // Missing optional fields drop out cleanly.
  assert.equal(eventSummary(ev("pty-spawned", { id: "7" })), "pty 7");
});

test("pty-exited summarizes the id", () => {
  assert.equal(eventSummary(ev("pty-exited", { id: "7" })), "pty 7");
});

test("session-uuid-adopted shows pty, short uuid, and source", () => {
  assert.equal(
    eventSummary(
      ev("session-uuid-adopted", {
        ptyId: "3",
        uuid: "abcdef00-1111-4000-8000-000000000000",
        source: "watcher",
      }),
    ),
    "pty 3 → abcdef00 (watcher)",
  );
});

test("fork-created shows short src → branch @ turn", () => {
  assert.equal(
    eventSummary(
      ev("fork-created", {
        srcUuid: "11111111-aaaa-4000-8000-000000000000",
        branchUuid: "22222222-bbbb-4000-8000-000000000000",
        turn: "u2abcdef-cccc",
      }),
    ),
    "11111111 → 22222222 @ u2abcdef",
  );
});

test("refusals lead with the reason and add context", () => {
  assert.equal(
    eventSummary(ev("spawn-refused", { reason: "no such directory", cwd: "/gone" })),
    "no such directory · /gone",
  );
  assert.equal(
    eventSummary(
      ev("resume-refused", {
        reason: "session's project directory no longer exists",
        session: "abcdef00-0000",
      }),
    ),
    "session's project directory no longer exists · abcdef00",
  );
});

test("downgrade-recovered shows short src → branch and the restatement mode", () => {
  assert.equal(
    eventSummary(
      ev("downgrade-recovered", {
        srcUuid: "dddd4444-0000-4000-8000-000000000004",
        branchUuid: "eeee5555-0000-4000-8000-000000000005",
        offendingTurn: "u2",
        rephrased: true,
      }),
    ),
    "dddd4444 → eeee5555 · rephrased",
  );
  // The rephraser fell back to the verbatim prompt.
  assert.equal(
    eventSummary(
      ev("downgrade-recovered", {
        srcUuid: "dddd4444-0000-4000-8000-000000000004",
        branchUuid: "eeee5555-0000-4000-8000-000000000005",
        rephrased: false,
      }),
    ),
    "dddd4444 → eeee5555 · verbatim",
  );
});

test("downgrade-recovery-failed leads with src and the reason", () => {
  assert.equal(
    eventSummary(
      ev("downgrade-recovery-failed", {
        srcUuid: "dddd4444-0000-4000-8000-000000000004",
        reason: "no downgrade detected",
      }),
    ),
    "dddd4444 · no downgrade detected",
  );
  // A missing reason still renders the fallback verb, not an empty string.
  assert.equal(
    eventSummary(ev("downgrade-recovery-failed", { srcUuid: "dddd4444-0000" })),
    "dddd4444 · failed",
  );
});

test("an unknown (future) kind still renders a generic summary", () => {
  // The key extensibility property: a kind events.ts has never seen must still
  // summarize legibly, so a later branch can emit new kinds with no change here.
  assert.equal(
    eventSummary(ev("downgrade-detected", { from: "fable", to: "opus", ptyId: "9" })),
    "from: fable · to: opus · ptyId: 9",
  );
  // No payload → empty summary, not a crash.
  assert.equal(eventSummary(ev("mystery", {})), "");
});
