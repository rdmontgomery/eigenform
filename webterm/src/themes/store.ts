// Client for the daemon's custom-theme store (`/api/themes`).
import { readCustomTheme } from "./custom.ts";
import type { CustomTheme } from "./custom.ts";

function readList(raw: unknown): CustomTheme[] {
  return Array.isArray(raw) ? raw.map(readCustomTheme).filter((t): t is CustomTheme => t !== null) : [];
}

/** Every saved theme; null when the daemon can't be reached. */
export async function fetchThemes(): Promise<CustomTheme[] | null> {
  try {
    const res = await fetch("/api/themes");
    return res.ok ? readList(await res.json()) : null;
  } catch {
    return null;
  }
}

/** Create or replace `t`; the daemon's list afterwards. Throws with the daemon's reason. */
export async function saveTheme(t: CustomTheme): Promise<CustomTheme[]> {
  const res = await fetch(`/api/themes/${encodeURIComponent(t.id)}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(t),
  });
  if (!res.ok) throw new Error((await res.text()) || `save failed (${res.status})`);
  return readList(await res.json());
}

/** Delete theme `id`; the daemon's list afterwards (a 404 counts as done). */
export async function deleteTheme(id: string): Promise<CustomTheme[]> {
  const res = await fetch(`/api/themes/${encodeURIComponent(id)}`, { method: "DELETE" });
  if (!res.ok && res.status !== 404) throw new Error((await res.text()) || `delete failed (${res.status})`);
  return (await fetchThemes()) ?? [];
}
