import { create } from "zustand";

import {
  type ChatTurn,
  type HistoryFilter,
  type HistorySource,
  askHistory,
  errorMessage,
} from "@/lib/ipc";

/** Turni precedenti inviati al modello. */
const HISTORY_TURNS = 6;

export interface HistoryChatLine {
  id: number;
  role: "user" | "assistant";
  /** Domanda, o risposta con citazioni `[n]` (vuota se non trovata). */
  text: string;
  /** Solo risposte: `false` = non trovato nei meeting. */
  found?: boolean;
  /** Solo risposte: `false` = solo ricerca per parole. */
  semantic?: boolean;
  sources?: HistorySource[];
}

/** Filtri della UI: date `AAAA-MM-GG` (vuote = nessun limite). */
export interface HistoryChatFilters {
  from: string;
  to: string;
  onlySelected: boolean;
}

interface HistoryChatState {
  lines: HistoryChatLine[];
  sending: boolean;
  error: string | null;
  filters: HistoryChatFilters;
  setFilters: (patch: Partial<HistoryChatFilters>) => void;
  /** `selectedMeeting` vale solo con il filtro "solo il meeting aperto". */
  ask: (text: string, selectedMeeting: number | null) => Promise<void>;
  reset: () => void;
}

let nextId = 1;

/** Mezzanotte locale del giorno `AAAA-MM-GG` (+`days`), in ms. */
function localDay(value: string, days = 0): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + days).getTime();
}

export function toFilter(f: HistoryChatFilters, selectedMeeting: number | null): HistoryFilter | null {
  const filter: HistoryFilter = {
    fromMs: localDay(f.from),
    // La data finale è inclusa: fino alla mezzanotte del giorno dopo.
    toMs: localDay(f.to, 1),
    meetingId: f.onlySelected ? selectedMeeting : null,
  };
  return filter.fromMs === null && filter.toMs === null && filter.meetingId === null
    ? null
    : filter;
}

export const useHistoryChatStore = create<HistoryChatState>((set, get) => ({
  lines: [],
  sending: false,
  error: null,
  filters: { from: "", to: "", onlySelected: false },

  setFilters: (patch) => set((s) => ({ filters: { ...s.filters, ...patch } })),

  ask: async (text, selectedMeeting) => {
    const question = text.trim();
    if (question === "" || get().sending) return;
    nextId += 1;
    const user: HistoryChatLine = { id: nextId, role: "user", text: question };
    const history: ChatTurn[] = get()
      .lines.slice(-HISTORY_TURNS)
      // "Non trovato" torna al modello nella sua forma convenzionale.
      .map((l) => ({ role: l.role, content: l.found === false ? "NOT_FOUND" : l.text }));
    set((s) => ({ lines: [...s.lines, user], sending: true, error: null }));
    try {
      const reply = await askHistory({
        question,
        history,
        filter: toFilter(get().filters, selectedMeeting),
      });
      nextId += 1;
      set((s) => ({
        lines: [
          ...s.lines,
          {
            id: nextId,
            role: "assistant",
            text: reply.answer,
            found: reply.found,
            semantic: reply.semantic,
            sources: reply.sources,
          },
        ],
        sending: false,
      }));
    } catch (e) {
      set({ sending: false, error: errorMessage(e) });
    }
  },

  reset: () => set({ lines: [], sending: false, error: null }),
}));
