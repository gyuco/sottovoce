import { create } from "zustand";

import { checkUpdate, type UpdateInfo } from "@/lib/ipc";

/** Sha dell'ultima build che l'utente ha scelto di ignorare. */
const DISMISSED_KEY = "sottovoce.dismissedUpdate";
/** Quando è stato fatto l'ultimo controllo (ms epoch). */
const CHECKED_AT_KEY = "sottovoce.updateCheckedAt";
/** Non si interroga GitHub più spesso di così. */
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

interface UpdateState {
  /** Build disponibile da proporre; `null` = niente da segnalare. */
  info: UpdateInfo | null;
  /** Controlla (con cache) l'ultima release pubblicata. */
  check: () => Promise<void>;
  /** Ignora questa build: non viene più proposta. */
  dismiss: () => void;
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  info: null,

  check: async () => {
    const checkedAt = Number(localStorage.getItem(CHECKED_AT_KEY) ?? 0);
    if (Number.isFinite(checkedAt) && Date.now() - checkedAt < CHECK_INTERVAL_MS) return;

    try {
      const info = await checkUpdate();
      localStorage.setItem(CHECKED_AT_KEY, String(Date.now()));
      // Build locale o niente di nuovo: niente da mostrare.
      if (!info) return;
      // L'utente ha già detto "ignora" per questa build.
      if (localStorage.getItem(DISMISSED_KEY) === info.build) return;
      set({ info });
    } catch {
      // Offline, rate limit o API giù: si riprova al prossimo avvio.
    }
  },

  dismiss: () => {
    const info = get().info;
    if (info) localStorage.setItem(DISMISSED_KEY, info.build);
    set({ info: null });
  },
}));
