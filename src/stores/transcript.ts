import { create } from "zustand";

import type { ChannelId, Segment, SttEvent, TranslationEvent } from "@/lib/ipc";

export type ConnState = "idle" | "connected" | "reconnecting" | "error" | "closed";

export interface Line extends Segment {
  id: number;
  channel: ChannelId;
}

interface TranscriptState {
  lines: Line[];
  /** Ipotesi provvisoria corrente per canale (sostituita a ogni parziale). */
  partial: Record<ChannelId, Line | null>;
  conn: Record<ChannelId, ConnState>;
  connError: Record<ChannelId, string | null>;
  /** Traduzioni per id di frase (arrivano anche fuori ordine). */
  translations: Record<number, TranslationEvent>;
  showTranslations: boolean;
  apply: (e: SttEvent) => void;
  applyTranslation: (e: TranslationEvent) => void;
  setShowTranslations: (show: boolean) => void;
  reset: () => void;
}

const initial = () => ({
  lines: [] as Line[],
  partial: { me: null, them: null },
  conn: { me: "idle", them: "idle" } as Record<ChannelId, ConnState>,
  connError: { me: null, them: null },
  translations: {} as Record<number, TranslationEvent>,
});

export const useTranscriptStore = create<TranscriptState>((set) => ({
  ...initial(),
  showTranslations: true,

  applyTranslation: (e) =>
    set((s) => ({ translations: { ...s.translations, [e.segmentId]: e } })),

  setShowTranslations: (showTranslations) => set({ showTranslations }),

  apply: (e) =>
    set((s) => {
      const ch = e.channel;
      switch (e.type) {
        case "partial":
          // Parziale vuoto = nessuna ipotesi in corso (es. rumore scartato dal motore locale).
          return {
            partial: { ...s.partial, [ch]: e.text.trim() === "" ? null : { ...seg(e), id: 0, channel: ch } },
          };
        case "final":
          return {
            lines: [...s.lines, { ...seg(e), channel: ch }],
            partial: { ...s.partial, [ch]: null },
          };
        case "connected":
          return { conn: { ...s.conn, [ch]: "connected" }, connError: { ...s.connError, [ch]: null } };
        case "reconnecting":
          return { conn: { ...s.conn, [ch]: "reconnecting" } };
        case "error":
          return {
            conn: { ...s.conn, [ch]: "error" },
            connError: { ...s.connError, [ch]: e.message },
            partial: { ...s.partial, [ch]: null },
          };
        case "disconnected":
          return { conn: { ...s.conn, [ch]: "closed" }, partial: { ...s.partial, [ch]: null } };
      }
    }),

  reset: () => set(initial()),
}));

function seg(e: Segment): Segment {
  return {
    id: e.id,
    text: e.text,
    start: e.start,
    end: e.end,
    confidence: e.confidence,
    language: e.language,
  };
}
