// Tests for status.ts — the status-dot / tag / ink presentation helpers.
import { test } from "node:test";
import assert from "node:assert/strict";
import { dotClasses, dotTitle, inkVar, livenessTag } from "./status.ts";

test("dotClasses: activity sets color, liveness sets fill", () => {
  assert.equal(dotClasses("working", "eigenform"), "dot dot--working dot--eigenform");
  assert.equal(dotClasses("waiting", "external"), "dot dot--waiting dot--external");
  assert.equal(dotClasses("idle", "none"), "dot dot--idle dot--dead");
});

test("dotClasses: an unknown activity falls back to idle", () => {
  assert.equal(dotClasses("compacting", "eigenform"), "dot dot--idle dot--eigenform");
});

test("livenessTag: dead rows get no tag; external rows are prefixed", () => {
  assert.equal(livenessTag("working", "none"), null);
  assert.equal(livenessTag("working", "eigenform"), "· running");
  assert.equal(livenessTag("waiting", "eigenform"), "· your turn");
  assert.equal(livenessTag("idle", "external"), "· ext · live");
});

test("dotTitle: explains both channels, or just provenance when dead", () => {
  assert.equal(dotTitle("working", "eigenform"), "assistant running — eigenform session");
  assert.equal(
    dotTitle("waiting", "external"),
    "waiting for your input — running outside eigenform — can't attach",
  );
  assert.equal(dotTitle("working", "none"), "no live process");
});

test("inkVar: same cwd → same hue; falls back to the label without a cwd", () => {
  assert.equal(inkVar("/home/me/proj", "x"), inkVar("/home/me/proj", "y"));
  assert.equal(inkVar(undefined, "proj"), inkVar("proj", "ignored"));
  assert.match(inkVar("/a", "a"), /^var\(--ink-\w+\)$/);
});
