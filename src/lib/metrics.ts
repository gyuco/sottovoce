import type { ChannelId } from "@/lib/ipc";

/** Frase finale ridotta a ciò che serve alle metriche (tempi in ms). */
export interface MetricLine {
  channel: ChannelId;
  text: string;
  start: number;
  end: number;
}

export interface ChannelMetrics {
  /** Somma delle durate delle frasi. */
  talkMs: number;
  words: number;
  /** Parole al minuto di parlato; `null` finché il parlato è troppo poco. */
  wpm: number | null;
  questions: number;
  /** Turno più lungo senza interruzioni dell'altro canale. */
  longestMonologueMs: number;
}

export interface MeetingMetrics {
  me: ChannelMetrics;
  them: ChannelMetrics;
  /** Quota "Io" del tempo di parola, 0–100; `null` senza parlato. */
  mySharePct: number | null;
}

/** Sotto questa soglia di parlato le parole al minuto oscillano troppo. */
const MIN_TALK_FOR_WPM_MS = 15_000;

const empty = (): ChannelMetrics => ({
  talkMs: 0,
  words: 0,
  wpm: null,
  questions: 0,
  longestMonologueMs: 0,
});

/** Metriche della conversazione, senza LLM: dipendono solo dalle frasi finali. */
export function computeMetrics(lines: readonly MetricLine[]): MeetingMetrics {
  const acc: Record<ChannelId, ChannelMetrics> = { me: empty(), them: empty() };
  // Le frasi arrivano per canale: per i turni serve l'ordine temporale.
  const sorted = [...lines].sort((a, b) => a.start - b.start);

  let turn: { channel: ChannelId; start: number; end: number } | null = null;
  const closeTurn = () => {
    if (!turn) return;
    const m = acc[turn.channel];
    m.longestMonologueMs = Math.max(m.longestMonologueMs, turn.end - turn.start);
  };

  for (const line of sorted) {
    const m = acc[line.channel];
    m.talkMs += Math.max(0, line.end - line.start);
    m.words += countWords(line.text);
    m.questions += countQuestions(line.text);

    if (turn && turn.channel === line.channel) {
      turn.end = Math.max(turn.end, line.end);
    } else {
      closeTurn();
      turn = { channel: line.channel, start: line.start, end: line.end };
    }
  }
  closeTurn();

  for (const m of [acc.me, acc.them]) {
    m.wpm = m.talkMs >= MIN_TALK_FOR_WPM_MS ? Math.round(m.words / (m.talkMs / 60_000)) : null;
  }

  const total = acc.me.talkMs + acc.them.talkMs;
  return {
    me: acc.me,
    them: acc.them,
    mySharePct: total > 0 ? Math.round((acc.me.talkMs / total) * 100) : null,
  };
}

/** Parole = gruppi che contengono almeno una lettera o una cifra. */
export function countWords(text: string): number {
  return text.split(/\s+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;
}

/** Domande = gruppi di "?" (anche "??" o "?!" contano una volta). */
export function countQuestions(text: string): number {
  return text.match(/\?+/g)?.length ?? 0;
}
