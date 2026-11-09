import type { ChannelId } from "@/lib/ipc";
import { type MetricLine, countQuestions, countWords } from "@/lib/metrics";

/**
 * Nudge: avvisi brevi durante la call, calcolati dalle frasi finali senza LLM.
 * Pochi e distanziati: al massimo uno ogni 2 minuti, lo stesso tipo non prima
 * di 5 minuti, "nessuna domanda" una volta sola per meeting.
 */
export type NudgeKind = "monologue" | "talkTooMuch" | "tooFast" | "listenLong" | "noQuestions";

export type NudgeLevel = "low" | "medium";

export interface Nudge {
  kind: NudgeKind;
  level: NudgeLevel;
  /** Valore da mostrare nel testo: percentuale, secondi o parole/min. */
  value: number;
  /** Ms dall'avvio della sessione. */
  at: number;
}

/** Quando sono scattati gli ultimi nudge (ms di sessione). */
export interface NudgeHistory {
  lastAt: number | null;
  byKind: Partial<Record<NudgeKind, number>>;
}

export const emptyHistory = (): NudgeHistory => ({ lastAt: null, byKind: {} });

/** Nessun nudge nel primo minuto: le metriche sono ancora rumorose. */
export const WARMUP_MS = 60_000;
export const GLOBAL_COOLDOWN_MS = 120_000;
export const KIND_COOLDOWN_MS = 300_000;
/** Durata a schermo di un nudge prima che sparisca da solo. */
export const NUDGE_VISIBLE_MS = 30_000;

const SHARE_WINDOW_MS = 5 * 60_000;
const SHARE_MIN_TALK_MS = 45_000;
const SHARE_LOW = 65;
const SHARE_MEDIUM = 75;

const MONOLOGUE_MS = 90_000;
const MONOLOGUE_MEDIUM_MS = 150_000;
const LISTEN_LONG_MS = 180_000;
/** Un turno è "in corso" se l'ultima frase è finita da poco. */
const TURN_GAP_MS = 10_000;

const PACE_WINDOW_MS = 2 * 60_000;
const PACE_MIN_TALK_MS = 20_000;
const PACE_LOW_WPM = 175;
const PACE_MEDIUM_WPM = 200;

const NO_QUESTIONS_AFTER_MS = 5 * 60_000;
const NO_QUESTIONS_MIN_TALK_MS = 30_000;

/** Ordine di priorità: il primo che scatta vince. */
const RULES: readonly ((lines: readonly MetricLine[], now: number) => Omit<Nudge, "at"> | null)[] = [
  monologue,
  talkTooMuch,
  tooFast,
  listenLong,
  noQuestions,
];

/**
 * Nudge da mostrare adesso, o `null`. `now` e i tempi delle frasi sono ms
 * dall'avvio della sessione.
 */
export function evaluateNudge(lines: readonly MetricLine[], now: number, history: NudgeHistory): Nudge | null {
  if (now < WARMUP_MS) return null;
  if (history.lastAt !== null && now - history.lastAt < GLOBAL_COOLDOWN_MS) return null;

  for (const rule of RULES) {
    const hit = rule(lines, now);
    if (!hit) continue;
    const last = history.byKind[hit.kind];
    if (last !== undefined && (hit.kind === "noQuestions" || now - last < KIND_COOLDOWN_MS)) continue;
    return { ...hit, at: now };
  }
  return null;
}

/** Registra un nudge mostrato. */
export function recordNudge(history: NudgeHistory, nudge: Nudge): NudgeHistory {
  return { lastAt: nudge.at, byKind: { ...history.byKind, [nudge.kind]: nudge.at } };
}

/** "Io" parla da troppo senza che "Loro" intervenga. */
function monologue(lines: readonly MetricLine[], now: number) {
  const ms = currentTurnMs(lines, "me", now);
  if (ms < MONOLOGUE_MS) return null;
  return {
    kind: "monologue" as const,
    level: ms >= MONOLOGUE_MEDIUM_MS ? ("medium" as const) : ("low" as const),
    value: Math.round(ms / 1000),
  };
}

/** Quota "Io" del tempo di parola negli ultimi 5 minuti. */
function talkTooMuch(lines: readonly MetricLine[], now: number) {
  const recent = since(lines, now - SHARE_WINDOW_MS);
  const me = talkMs(recent, "me");
  const total = me + talkMs(recent, "them");
  if (total < SHARE_MIN_TALK_MS) return null;
  const pct = Math.round((me / total) * 100);
  if (pct <= SHARE_LOW) return null;
  return { kind: "talkTooMuch" as const, level: pct > SHARE_MEDIUM ? ("medium" as const) : ("low" as const), value: pct };
}

/** Parole al minuto di "Io" negli ultimi 2 minuti. */
function tooFast(lines: readonly MetricLine[], now: number) {
  const mine = since(lines, now - PACE_WINDOW_MS).filter((l) => l.channel === "me");
  const ms = talkMs(mine, "me");
  if (ms < PACE_MIN_TALK_MS) return null;
  const words = mine.reduce((n, l) => n + countWords(l.text), 0);
  const wpm = Math.round(words / (ms / 60_000));
  if (wpm < PACE_LOW_WPM) return null;
  return { kind: "tooFast" as const, level: wpm >= PACE_MEDIUM_WPM ? ("medium" as const) : ("low" as const), value: wpm };
}

/** "Loro" parla da 3 minuti di fila: momento per riassumere o chiedere. */
function listenLong(lines: readonly MetricLine[], now: number) {
  const ms = currentTurnMs(lines, "them", now);
  if (ms < LISTEN_LONG_MS) return null;
  return { kind: "listenLong" as const, level: "low" as const, value: Math.round(ms / 1000) };
}

/** Dopo 5 minuti "Io" non ha ancora fatto domande. */
function noQuestions(lines: readonly MetricLine[], now: number) {
  if (now < NO_QUESTIONS_AFTER_MS) return null;
  const mine = lines.filter((l) => l.channel === "me");
  if (talkMs(mine, "me") < NO_QUESTIONS_MIN_TALK_MS) return null;
  if (mine.some((l) => countQuestions(l.text) > 0)) return null;
  return { kind: "noQuestions" as const, level: "low" as const, value: Math.round(now / 60_000) };
}

/**
 * Durata del turno in corso di `channel`: frasi consecutive dello stesso
 * canale fino all'ultima, purché finita da poco. 0 se il turno è dell'altro
 * canale o è chiuso.
 */
export function currentTurnMs(lines: readonly MetricLine[], channel: ChannelId, now: number): number {
  const sorted = [...lines].sort((a, b) => a.start - b.start);
  const last = sorted.at(-1);
  if (!last || last.channel !== channel || now - last.end > TURN_GAP_MS) return 0;
  let start = last.start;
  let end = last.end;
  for (const line of [...sorted].reverse()) {
    if (line.channel !== channel) break;
    start = Math.min(start, line.start);
    end = Math.max(end, line.end);
  }
  return end - start;
}

function since(lines: readonly MetricLine[], from: number) {
  return lines.filter((l) => l.end > from);
}

function talkMs(lines: readonly MetricLine[], channel: ChannelId) {
  return lines.reduce((ms, l) => (l.channel === channel ? ms + Math.max(0, l.end - l.start) : ms), 0);
}
