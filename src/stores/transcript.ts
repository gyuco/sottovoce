import { create } from "zustand";

import type { ChannelId, Segment, SttEvent } from "@/lib/ipc";

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
  apply: (e: SttEvent) => void;
  reset: () => void;
}

let nextId = 1;

const initial = () => ({
  lines: [] as Line[],
  partial: { me: null, them: null },
  conn: { me: "idle", them: "idle" } as Record<ChannelId, ConnState>,
  connError: { me: null, them: null },
});

export const useTranscriptStore = create<TranscriptState>((set) => ({
  ...initial(),

  apply: (e) =>
    set((s) => {
      const ch = e.channel;
      switch (e.type) {
        case "partial":
          return { partial: { ...s.partial, [ch]: { ...seg(e), id: 0, channel: ch } } };
        case "final":
          return {
            lines: [...s.lines, { ...seg(e), id: nextId++, channel: ch }],
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
  return { text: e.text, start: e.start, end: e.end, confidence: e.confidence };
}
