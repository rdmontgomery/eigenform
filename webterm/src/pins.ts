/**
 * pins.ts — sessions pinned to the top of the rail.
 *
 * The daemon stores the pins (`/api/pins`, persisted under its state dir), so they
 * outlive a reload and a daemon restart and every window shows the same ones. A pin
 * is a session uuid; a row with no uuid yet (a fresh pty) can't be pinned.
 *
 * `splitPinned` is pure and tested in pins.test.ts.
 */

export interface Pin {
  uuid: string;
  pinnedAt: number;
}

/** Split `rows` into the pinned ones (in pin order, oldest pin first) and the rest
 *  (in their original order). */
export function splitPinned<R extends { uuid?: string | undefined }>(
  rows: readonly R[],
  pins: readonly string[],
): { pinned: R[]; rest: R[] } {
  const order = new Map(pins.map((u, i) => [u, i]));
  const pinned: R[] = [];
  const rest: R[] = [];
  for (const r of rows) (r.uuid !== undefined && order.has(r.uuid) ? pinned : rest).push(r);
  pinned.sort((a, b) => order.get(a.uuid!)! - order.get(b.uuid!)!);
  return { pinned, rest };
}

/** The pinned uuids, oldest pin first; null when the daemon can't be reached. */
export async function fetchPins(): Promise<string[] | null> {
  try {
    const res = await fetch("/api/pins");
    if (!res.ok) return null;
    return ((await res.json()) as Pin[]).map((p) => p.uuid);
  } catch {
    return null;
  }
}

/** Pin or unpin `uuid`; the daemon's list afterwards, or null if the request failed. */
export async function setPinned(uuid: string, pinned: boolean): Promise<string[] | null> {
  try {
    const res = pinned
      ? await fetch("/api/pins", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ session: uuid }),
        })
      : await fetch(`/api/pins/${encodeURIComponent(uuid)}`, { method: "DELETE" });
    // Unpinning what another window already unpinned is a 404, not a failure.
    if (!res.ok && !(res.status === 404 && !pinned)) return null;
    return res.ok ? ((await res.json()) as Pin[]).map((p) => p.uuid) : await fetchPins();
  } catch {
    return null;
  }
}
