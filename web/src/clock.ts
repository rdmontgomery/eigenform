// clock.ts — the Furnace cooling clock: the session's vital sign. It owns the only
// per-second update in the app. build() returns the node plus an update(cache) that
// touches a handful of text nodes + one arc attribute; the bar width and ember opacity
// ride the --temp custom property in CSS, so the rest is free.
import { el, svg } from "./dom.ts";
import { type CacheReading, fmtClock, fmtK, TTL_EXTENDED } from "./cooling.ts";

export interface ClockHandle {
  node: HTMLElement;
  update(cache: CacheReading): void;
}

const ARC = 53.4; // 2πr for r≈8.5

export function buildClock(opts: { onMind: () => void; onExtend: () => void; extended: () => boolean }): ClockHandle {
  const arc = svg("circle", { cx: 10, cy: 10, r: 8.5, fill: "none", stroke: "var(--temp-color)", "stroke-width": 1.5, "stroke-dasharray": `${ARC} ${ARC}`, "stroke-linecap": "round", transform: "rotate(-90 10 10)" });
  const core = svg("circle", { cx: 10, cy: 10, r: 3.4, fill: "var(--temp-color)" });
  core.classList.add("ember-core");
  const ember = svg("svg", { width: 20, height: 20, viewBox: "0 0 20 20" },
    svg("circle", { cx: 10, cy: 10, r: 8.5, fill: "none", stroke: "var(--line)", "stroke-width": 1.5 }), arc, core);

  const emberLabel = el("div", { class: "ember-label", text: "cache hot" });
  const emberSub = el("div", { class: "ember-sub", text: "the furnace is lit" });

  const bar = el("i");
  const lastWrite = el("span", { text: "last write 0:00 ago" });
  const ttlLabel = el("span", { text: "TTL 5:00" });

  // warm readout (cold in M:SS) and cold readout — toggled via the .cold class
  const rewarmWarm = el("span", { class: "rewarm", title: "open the Mind — what re-warms", onclick: opts.onMind, text: "~118k" });
  const rewarmCold = el("span", { class: "rewarm", title: "open the Mind — what re-warms", onclick: opts.onMind, text: "~118k" });
  const countdown = el("span", { class: "mono", style: "font-size:14px", text: "0:00" });
  const warmRead = el("div", { class: "warm-read", style: "line-height:1.2" },
    el("div", { class: "big" }, "cold in ", countdown),
    el("div", { class: "sub" }, "then re-warm ", rewarmWarm, " · feed the leaf to stay hot"));
  const coldRead = el("div", { class: "big cold", style: "display:none" }, "cold — next message re-warms ", rewarmCold, " at full price");

  const ttlBtn = el("button", { class: "ghost", title: "toggle cache TTL", onclick: opts.onExtend },
    el("span", { class: "ttl-text", text: "5-min cache" }));

  const node = el("div", { class: "clock" },
    el("div", { style: "display:flex;align-items:center;gap:10px" }, ember, el("div", {}, emberLabel, emberSub)),
    el("div", { class: "bar-wrap" },
      el("div", { class: "bar" }, bar),
      el("div", { class: "bar-meta" }, lastWrite, ttlLabel)),
    el("div", { class: "readout" }, warmRead, coldRead),
    ttlBtn);

  function update(cache: CacheReading): void {
    const urgent = !cache.cold && cache.remaining <= 45;
    node.classList.toggle("cold", cache.cold);
    node.classList.toggle("urgent", urgent);
    arc.setAttribute("stroke-dasharray", `${(cache.temp * ARC).toFixed(1)} ${ARC}`);
    core.classList.toggle("ember-pulse", !cache.cold);
    emberLabel.textContent = `cache ${cache.label}`;
    emberSub.textContent = cache.cold ? "the furnace has gone out" : cache.temp > 0.66 ? "the furnace is lit" : "the furnace is cooling";
    lastWrite.textContent = `last write ${fmtClock(cache.idle)} ago`;
    ttlLabel.textContent = `TTL ${fmtClock(cache.ttl)}`;
    countdown.textContent = fmtClock(cache.remaining);
    countdown.classList.toggle("urgent-num", urgent);
    const k = `~${fmtK(cache.reWarmFull)}`;
    rewarmWarm.textContent = k;
    rewarmCold.textContent = k;
    warmRead.style.display = cache.cold ? "none" : "block";
    coldRead.style.display = cache.cold ? "block" : "none";
    (ttlBtn.querySelector(".ttl-text") as HTMLElement).textContent = opts.extended() ? "1-hr cache" : "5-min cache";
    ttlBtn.style.color = opts.extended() ? "var(--amber)" : "var(--dim)";
    void TTL_EXTENDED;
  }

  return { node, update };
}
