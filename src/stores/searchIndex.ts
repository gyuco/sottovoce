import { create } from "zustand";

import {
  type IndexStatus,
  errorMessage,
  onIndexStatus,
  reindexMeetings,
  semanticIndexStatus,
} from "@/lib/ipc";

interface SearchIndexState {
  status: IndexStatus | null;
  error: string | null;
  /** Legge lo stato e, la prima volta, si iscrive agli aggiornamenti. */
  load: () => Promise<void>;
  reindex: () => Promise<void>;
}

let subscribed = false;

/** Stato dell'indice della ricerca per significato (Storico e Impostazioni). */
export const useSearchIndexStore = create<SearchIndexState>((set) => ({
  status: null,
  error: null,
  load: async () => {
    if (!subscribed) {
      subscribed = true;
      void onIndexStatus((status) => set({ status, error: null })).catch(() => {
        subscribed = false;
      });
    }
    try {
      set({ status: await semanticIndexStatus(), error: null });
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },
  reindex: async () => {
    try {
      await reindexMeetings();
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },
}));

/** La ricerca per significato è usabile adesso. */
export const semanticReady = (s: IndexStatus | null) =>
  !!s && s.modelReady && s.enabled && s.indexed > 0;
