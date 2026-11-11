/** Parte di una risposta: testo semplice o una citazione `[n]` / `[n, m]`. */
export type AnswerPart = { text: string } | { cites: number[] };

/** Stesso formato letto dal backend (`sv_search::citations`). */
const CITATION = /\[(\d+(?:\s*,\s*\d+)*)\]/g;

/** Divide una risposta in testo e citazioni, nell'ordine. */
export function splitCitations(answer: string): AnswerPart[] {
  const parts: AnswerPart[] = [];
  let last = 0;
  for (const match of answer.matchAll(CITATION)) {
    const cites = (match[1] ?? "")
      .split(",")
      .map((n) => Number(n.trim()))
      .filter((n) => n > 0);
    if (cites.length === 0) continue;
    if (match.index > last) parts.push({ text: answer.slice(last, match.index) });
    parts.push({ cites });
    last = match.index + match[0].length;
  }
  if (last < answer.length) parts.push({ text: answer.slice(last) });
  return parts;
}
