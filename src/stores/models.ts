import { create } from "zustand";

import {
  type DownloadProgress,
  type LocalModel,
  cancelLocalModelDownload,
  deleteLocalModel,
  downloadLocalModel,
  errorMessage,
  listLocalModels,
  verifyLocalModel,
} from "@/lib/ipc";

interface ModelsState {
  models: LocalModel[];
  /** Avanzamento dei download avviati da questa finestra. */
  progress: Record<string, DownloadProgress>;
  /** Ultimo errore per modello. */
  errors: Record<string, string>;
  /** Operazioni in corso (verifica/eliminazione) per modello. */
  busy: Record<string, boolean>;
  load: () => Promise<void>;
  download: (id: string) => Promise<void>;
  cancel: (id: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
  verify: (id: string) => Promise<void>;
}

const without = <T,>(rec: Record<string, T>, id: string) => {
  const next = { ...rec };
  delete next[id];
  return next;
};

export const useModelsStore = create<ModelsState>((set, get) => {
  const replace = (m: LocalModel) =>
    set((s) => ({ models: s.models.map((x) => (x.id === m.id ? m : x)) }));

  /** Esegue un'operazione sul modello, con stato "occupato" ed errore. */
  const run = async (id: string, op: () => Promise<LocalModel>) => {
    set((s) => ({ busy: { ...s.busy, [id]: true }, errors: without(s.errors, id) }));
    try {
      replace(await op());
    } catch (e) {
      set((s) => ({ errors: { ...s.errors, [id]: errorMessage(e) } }));
      await get().load();
    } finally {
      set((s) => ({ busy: without(s.busy, id) }));
    }
  };

  return {
    models: [],
    progress: {},
    errors: {},
    busy: {},

    load: async () => {
      try {
        set({ models: await listLocalModels() });
      } catch (e) {
        set((s) => ({ errors: { ...s.errors, _list: errorMessage(e) } }));
      }
    },

    download: async (id) => {
      set((s) => ({
        models: s.models.map((m) => (m.id === id ? { ...m, downloading: true } : m)),
        progress: { ...s.progress, [id]: { downloaded: 0, total: s.models.find((m) => m.id === id)?.size ?? 0 } },
      }));
      await run(id, () =>
        downloadLocalModel(id, (p) => set((s) => ({ progress: { ...s.progress, [id]: p } }))),
      );
      set((s) => ({ progress: without(s.progress, id) }));
    },

    cancel: async (id) => {
      await cancelLocalModelDownload(id);
    },

    remove: (id) => run(id, () => deleteLocalModel(id)),

    verify: (id) => run(id, () => verifyLocalModel(id)),
  };
});
