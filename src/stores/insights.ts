import { create } from "zustand";

import type { CompressionEvent, Summary } from "@/lib/ipc";

interface InsightsState {
  blocks: CompressionEvent[];
  /** Riepilogo finale della sessione appena conclusa (o nullo). */
  summary: Summary | null;
  addBlock: (block: CompressionEvent) => void;
  setSummary: (summary: Summary | null) => void;
  reset: () => void;
}

export const useInsightsStore = create<InsightsState>((set) => ({
  blocks: [],
  summary: null,
  addBlock: (block) =>
    set((s) =>
      s.blocks.some((b) => b.index === block.index)
        ? s
        : { blocks: [...s.blocks, block].sort((a, b) => a.index - b.index) },
    ),
  setSummary: (summary) => set({ summary }),
  reset: () => set({ blocks: [], summary: null }),
}));
