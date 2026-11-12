import { create } from "zustand";

import {
  type CalendarStatus,
  cancelGoogleCalendarConnect,
  connectGoogleCalendar,
  disconnectGoogleCalendar,
  errorMessage,
  googleCalendarStatus,
  syncGoogleCalendar,
} from "@/lib/ipc";
import { useSettingsStore } from "@/stores/settings";

interface CalendarState {
  status: CalendarStatus | null;
  /** Errore dell'ultima azione dell'utente (collega, scollega, sincronizza). */
  error: string | null;
  busy: boolean;
  load: () => Promise<void>;
  connect: (uiLanguage: string) => Promise<void>;
  cancel: () => Promise<void>;
  disconnect: () => Promise<void>;
  sync: () => Promise<void>;
}

/** Sincronizzazione con Google Calendar (stato dal backend). */
export const useCalendarStore = create<CalendarState>((set, get) => {
  const run = async (action: () => Promise<CalendarStatus>) => {
    set({ busy: true, error: null });
    try {
      set({ status: await action() });
    } catch (e) {
      set({ error: errorMessage(e) });
      await get().load();
    } finally {
      set({ busy: false });
      // Email dell'account collegato: sta nei settings.
      void useSettingsStore.getState().load();
    }
  };

  return {
    status: null,
    error: null,
    busy: false,

    load: async () => {
      try {
        set({ status: await googleCalendarStatus() });
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    connect: (uiLanguage) => {
      // Lo stato "collegamento in corso" arriva subito, il resto al ritorno.
      setTimeout(() => void get().load(), 300);
      return run(() => connectGoogleCalendar(uiLanguage));
    },
    cancel: async () => {
      await cancelGoogleCalendarConnect();
    },
    disconnect: () => run(disconnectGoogleCalendar),
    sync: () => run(syncGoogleCalendar),
  };
});
