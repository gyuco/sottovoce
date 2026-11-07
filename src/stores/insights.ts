import { create } from "zustand";

import type { CompressionEvent, Summary, SummaryStatus } from "@/lib/ipc";

/** Avviso sull'esito del riepilogo generato in background. */
export type SummaryNotice =
  | { kind: "ready"; meetingId: number }
  | { kind: "failed"; meetingId: number; message: string };

interface InsightsState {
  blocks: CompressionEvent[];
  /** Riepilogo finale della sessione appena conclusa (o nullo). */
  summary: Summary | null;
  /** Meeting appena chiuso il cui riepilogo è in preparazione. */
  pendingMeetingId: number | null;
  /** Errore del riepilogo della sessione appena conclusa. */
  summaryError: string | null;
  /** Ultimo riepilogo arrivato in background, per qualunque meeting. */
  lastReady: Summary | null;
  notice: SummaryNotice | null;
  addBlock: (block: CompressionEvent) => void;
  setSummary: (summary: Summary | null) => void;
  setPending: (meetingId: number | null) => void;
  applyStatus: (status: SummaryStatus) => void;
  dismissNotice: () => void;
  reset: () => void;
}

export const useInsightsStore = create<InsightsState>((set) => ({
  blocks: [],
  summary: null,
  pendingMeetingId: null,
  summaryError: null,
  lastReady: null,
  notice: null,
  addBlock: (block) =>
    set((s) =>
      s.blocks.some((b) => b.index === block.index)
        ? s
        : { blocks: [...s.blocks, block].sort((a, b) => a.index - b.index) },
    ),
  setSummary: (summary) => set({ summary }),
  setPending: (pendingMeetingId) => set({ pendingMeetingId, summaryError: null }),
  applyStatus: (status) =>
    set((s) => {
      // Riepilogo della sessione mostrata in Live: lo sostituisce al pannello.
      const current = s.pendingMeetingId === status.meetingId;
      if (status.state === "ready") {
        return {
          lastReady: status.summary,
          notice: { kind: "ready", meetingId: status.meetingId },
          ...(current ? { summary: status.summary, pendingMeetingId: null } : {}),
        };
      }
      return {
        notice: { kind: "failed", meetingId: status.meetingId, message: status.message },
        ...(current ? { summaryError: status.message, pendingMeetingId: null } : {}),
      };
    }),
  dismissNotice: () => set({ notice: null }),
  // Nuova sessione: il riepilogo della precedente, se arriva, va solo nell'avviso.
  reset: () => set({ blocks: [], summary: null, pendingMeetingId: null, summaryError: null }),
}));
