// epigraphs.ts — the empty Manuscript's invitation to play. A fresh session has no
// transcript to render, so the page opens on a single epigraph instead: a line from
// Woland's literary/cybernetic lineage, each one doubling as a quiet gloss on what the
// page does. One is chosen at random per open (see Manuscript.setEmpty); the leaf below
// is the real invitation — the first line lights the cold furnace.
export interface Epigraph {
  text: string;
  attribution: string;
}

// Each maps to a Woland mechanic: the Furnace, the leaf's first mark, the fork, the
// Forest's branching, the act of beginning. Keep this spread when the list grows.
export const EPIGRAPHS: readonly Epigraph[] = [
  { text: "Manuscripts don’t burn.", attribution: "Woland, in Bulgakov’s The Master and Margarita" },
  { text: "Draw a distinction.", attribution: "G. Spencer-Brown, Laws of Form" },
  { text: "I leave to several futures (not to all) my garden of forking paths.", attribution: "Borges, The Garden of Forking Paths" },
  { text: "Act always so as to increase the number of choices.", attribution: "Heinz von Foerster, the ethical imperative" },
  { text: "You are about to begin reading…", attribution: "Italo Calvino, If on a winter’s night a traveler" },
];

// Pick one at random. The rng is injectable so the choice is testable; in the browser it
// defaults to Math.random, giving a fresh epigraph on each new-session open.
export function pickEpigraph(rng: () => number = Math.random): Epigraph {
  const i = Math.min(EPIGRAPHS.length - 1, Math.max(0, Math.floor(rng() * EPIGRAPHS.length)));
  return EPIGRAPHS[i]!;
}
