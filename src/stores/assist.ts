import { create } from "zustand";

import type { AssistEvent } from "@/lib/ipc";

/** Suggerimenti visibili al massimo (i più recenti). */
export const MAX_VISIBLE = 6;

interface AssistState {
  /** Più recenti per primi; solo quelli non chiusi. */
  items: AssistEvent[];
  add: (e: AssistEvent) => void;
  dismiss: (id: number) => void;
  reset: () => void;
}

export const useAssistStore = create<AssistState>((set) => ({
  items: [],
  add: (e) => set((s) => ({ items: [e, ...s.items].slice(0, MAX_VISIBLE) })),
  dismiss: (id) => set((s) => ({ items: s.items.filter((i) => i.id !== id) })),
  reset: () => set({ items: [] }),
}));
