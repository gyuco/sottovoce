import { create } from "zustand";

import {
  type Settings,
  deleteApiKey,
  errorMessage,
  getSettings,
  hasApiKey,
  setApiKey,
  updateSettings,
} from "@/lib/ipc";

interface SettingsState {
  settings: Settings | null;
  /** La chiave del provider STT corrente è nel Keychain. */
  hasKey: boolean;
  error: string | null;
  load: () => Promise<void>;
  update: (patch: Partial<Settings>) => Promise<void>;
  saveKey: (key: string) => Promise<boolean>;
  deleteKey: () => Promise<void>;
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  settings: null,
  hasKey: false,
  error: null,

  load: async () => {
    try {
      const settings = await getSettings();
      const hasKey = await hasApiKey(settings.sttProvider);
      set({ settings, hasKey, error: null });
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },

  update: async (patch) => {
    try {
      set({ settings: await updateSettings(patch), error: null });
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },

  saveKey: async (key) => {
    const provider = get().settings?.sttProvider;
    if (!provider) return false;
    try {
      await setApiKey(provider, key);
      set({ hasKey: true, error: null });
      return true;
    } catch (e) {
      set({ error: errorMessage(e) });
      return false;
    }
  },

  deleteKey: async () => {
    const provider = get().settings?.sttProvider;
    if (!provider) return;
    try {
      await deleteApiKey(provider);
      set({ hasKey: false, error: null });
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },
}));
