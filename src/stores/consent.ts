import { create } from "zustand";

import type { Language } from "@/lib/ipc";

interface ConsentRequest {
  id: number;
  language: Language;
  resolve: (ok: boolean) => void;
}

interface ConsentState {
  pending: ConsentRequest | null;
  /**
   * Mostra il dialogo di consenso e risolve `true` se l'utente conferma che i
   * partecipanti hanno accettato. Va chiesto prima di ogni nuova call (non al
   * riavvio per cambio microfono).
   */
  ask: (language: Language) => Promise<boolean>;
  answer: (ok: boolean) => void;
}

let nextId = 0;

export const useConsentStore = create<ConsentState>((set, get) => ({
  pending: null,

  ask: (language) =>
    new Promise<boolean>((resolve) => {
      // Una richiesta già aperta si chiude come rifiutata.
      get().pending?.resolve(false);
      set({ pending: { id: ++nextId, language, resolve } });
    }),

  answer: (ok) => {
    get().pending?.resolve(ok);
    set({ pending: null });
  },
}));
