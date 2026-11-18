// Session references in a terminal's output become links: a peer name or session id
// (see session-refs.ts) underlines on hover, and a click brings that session forward
// in this window. xterm asks for one buffer line at a time as the pointer moves over
// it, so the cost is one line's scan per hover, whatever the scrollback holds.

import type { IBufferCell, IDisposable, ILink, Terminal } from "@xterm/xterm";
import { findSessionRefs, type SessionRefIndex } from "./session-refs.ts";

/** One buffer line as a string, with the 1-based column each UTF-16 unit sits in
 *  (wide glyphs take two columns; their spacer cell contributes nothing). */
function lineText(term: Terminal, y: number): { text: string; cols: number[] } | null {
  const line = term.buffer.active.getLine(y - 1);
  if (!line) return null;
  let text = "";
  const cols: number[] = [];
  let cell: IBufferCell | undefined;
  for (let x = 0; x < line.length; x++) {
    cell = line.getCell(x, cell);
    if (!cell || cell.getWidth() === 0) continue;
    const chars = cell.getChars() || " ";
    text += chars;
    for (let i = 0; i < chars.length; i++) cols.push(x + 1);
  }
  return { text, cols };
}

export function registerSessionLinks(
  term: Terminal,
  index: () => SessionRefIndex,
  open: (uuid: string) => void,
): IDisposable {
  return term.registerLinkProvider({
    provideLinks(y, callback) {
      const line = lineText(term, y);
      const matches = line ? findSessionRefs(line.text, index()) : [];
      if (!line || matches.length === 0) return callback(undefined);
      const links: ILink[] = matches.map((m) => ({
        range: {
          start: { x: line.cols[m.start]!, y },
          end: { x: line.cols[m.end - 1]!, y },
        },
        text: m.text,
        decorations: { underline: true, pointerCursor: true },
        activate: (e) => {
          if (e.button === 0) open(m.uuid);
        },
      }));
      callback(links);
    },
  });
}
