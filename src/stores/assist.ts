import { create } from "zustand";

import type { AssistEvent, AssistKind } from "@/lib/ipc";

/** Suggerimenti visibili al massimo per tipo (i più recenti). */
export const MAX_PER_KIND = 5;

interface AssistState {
  /** Più recenti per primi; solo quelli non chiusi. */
  items: AssistEvent[];
  /** Id dei suggerimenti segnati come "usati". */
  used: number[];
  add: (e: AssistEvent) => void;
  dismiss: (id: number) => void;
  toggleUsed: (id: number) => void;
  reset: () => void;
}

function trim(items: AssistEvent[]) {
  const seen: Record<AssistKind, number> = { say: 0, ask: 0 };
  return items.filter((i) => ++seen[i.kind] <= MAX_PER_KIND);
}

export const useAssistStore = create<AssistState>((set) => ({
  items: [],
  used: [],
  add: (e) =>
    set((s) => {
      const items = trim([e, ...s.items]);
      return { items, used: s.used.filter((id) => items.some((i) => i.id === id)) };
    }),
  dismiss: (id) =>
    set((s) => ({ items: s.items.filter((i) => i.id !== id), used: s.used.filter((u) => u !== id) })),
  toggleUsed: (id) =>
    set((s) => ({ used: s.used.includes(id) ? s.used.filter((u) => u !== id) : [...s.used, id] })),
  reset: () => set({ items: [], used: [] }),
}));
