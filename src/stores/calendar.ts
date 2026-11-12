import { create } from "zustand";

import {
  CALENDAR_PROVIDERS,
  type CalendarProvider,
  type CalendarStatus,
  calendarStatus,
  cancelCalendarConnect,
  connectCalendar,
  disconnectCalendar,
  errorMessage,
  syncCalendar,
} from "@/lib/ipc";
import { useSettingsStore } from "@/stores/settings";

type PerProvider<T> = Record<CalendarProvider, T>;

interface CalendarState {
  status: PerProvider<CalendarStatus | null>;
  /** Errore dell'ultima azione dell'utente (collega, scollega, sincronizza). */
  error: PerProvider<string | null>;
  busy: PerProvider<boolean>;
  load: (provider?: CalendarProvider) => Promise<void>;
  connect: (provider: CalendarProvider, uiLanguage: string) => Promise<void>;
  cancel: (provider: CalendarProvider) => Promise<void>;
  disconnect: (provider: CalendarProvider) => Promise<void>;
  sync: (provider: CalendarProvider) => Promise<void>;
  /** Sincronizza tutti i calendari collegati. */
  syncAll: () => Promise<void>;
}

const each = <T,>(value: T): PerProvider<T> => ({ google: value, microsoft: value });

/** Sincronizzazione con i calendari (stato dal backend). */
export const useCalendarStore = create<CalendarState>((set, get) => {
  const patch = <K extends "status" | "error" | "busy">(
    key: K,
    provider: CalendarProvider,
    value: CalendarState[K][CalendarProvider],
  ) => set((s) => ({ [key]: { ...s[key], [provider]: value } }) as Pick<CalendarState, K>);

  const run = async (provider: CalendarProvider, action: () => Promise<CalendarStatus>) => {
    patch("busy", provider, true);
    patch("error", provider, null);
    try {
      patch("status", provider, await action());
    } catch (e) {
      patch("error", provider, errorMessage(e));
      await get().load(provider);
    } finally {
      patch("busy", provider, false);
      // Email dell'account collegato: sta nei settings.
      void useSettingsStore.getState().load();
    }
  };

  return {
    status: each(null),
    error: each(null),
    busy: each(false),

    load: async (provider) => {
      for (const p of provider ? [provider] : CALENDAR_PROVIDERS) {
        try {
          patch("status", p, await calendarStatus(p));
        } catch (e) {
          patch("error", p, errorMessage(e));
        }
      }
    },

    connect: (provider, uiLanguage) => {
      // Lo stato "collegamento in corso" arriva subito, il resto al ritorno.
      setTimeout(() => void get().load(provider), 300);
      return run(provider, () => connectCalendar(provider, uiLanguage));
    },
    cancel: async (provider) => {
      await cancelCalendarConnect(provider);
    },
    disconnect: (provider) => run(provider, () => disconnectCalendar(provider)),
    sync: (provider) => run(provider, () => syncCalendar(provider)),
    syncAll: async () => {
      const connected = CALENDAR_PROVIDERS.filter((p) => get().status[p]?.connected);
      await Promise.all(connected.map((p) => get().sync(p)));
    },
  };
});
