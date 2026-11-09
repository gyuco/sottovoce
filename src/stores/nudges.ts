import { useEffect } from "react";
import { create } from "zustand";

import {
  type Nudge,
  type NudgeHistory,
  NUDGE_VISIBLE_MS,
  emptyHistory,
  evaluateNudge,
  recordNudge,
} from "@/lib/nudges";
import { useSettingsStore } from "@/stores/settings";
import { useTranscriptStore } from "@/stores/transcript";

interface NudgeState {
  /** Nudge a schermo; uno alla volta. */
  active: Nudge | null;
  history: NudgeHistory;
  /** Valuta le regole; `now` = ms dall'avvio della sessione. */
  tick: (now: number) => void;
  dismiss: () => void;
  reset: () => void;
}

export const useNudgeStore = create<NudgeState>((set, get) => ({
  active: null,
  history: emptyHistory(),

  tick: (now) => {
    const { active, history } = get();
    if (active && now - active.at >= NUDGE_VISIBLE_MS) set({ active: null });
    if (get().active || !useSettingsStore.getState().settings?.nudges) return;
    const nudge = evaluateNudge(useTranscriptStore.getState().lines, now, history);
    if (nudge) set({ active: nudge, history: recordNudge(history, nudge) });
  },

  // Il cooldown resta: chiuderlo non fa ricomparire subito un altro nudge.
  dismiss: () => set({ active: null }),

  reset: () => set({ active: null, history: emptyHistory() }),
}));

/** Ogni quanto si rivalutano le regole durante la sessione. */
const TICK_MS = 5_000;

/** Valuta i nudge a intervalli finché la sessione è attiva (finestra principale). */
export function useNudgeEngine(running: boolean, startedAt: number) {
  useEffect(() => {
    if (!running) return;
    const tick = () => useNudgeStore.getState().tick(performance.now() - startedAt);
    const id = setInterval(tick, TICK_MS);
    return () => clearInterval(id);
  }, [running, startedAt]);
}
