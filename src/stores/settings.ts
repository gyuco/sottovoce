import { create } from "zustand";

import {
  type KeyProvider,
  type Settings,
  deleteApiKey,
  errorMessage,
  getSettings,
  hasApiKey,
  llmKeyAccount,
  setApiKey,
  updateSettings,
} from "@/lib/ipc";

interface SettingsState {
  settings: Settings | null;
  /** La chiave del provider STT corrente è nel Keychain. */
  hasKey: boolean;
  /** Presenza delle chiavi nel Keychain, per account. */
  keys: Partial<Record<KeyProvider, boolean>>;
  error: string | null;
  load: () => Promise<void>;
  update: (patch: Partial<Settings>) => Promise<void>;
  refreshKey: (provider: KeyProvider) => Promise<void>;
  saveKey: (provider: KeyProvider, key: string) => Promise<boolean>;
  deleteKey: (provider: KeyProvider) => Promise<void>;
}

export const useSettingsStore = create<SettingsState>((set, get) => {
  const setKey = (provider: KeyProvider, present: boolean) =>
    set((s) => ({
      keys: { ...s.keys, [provider]: present },
      hasKey: provider === s.settings?.sttProvider ? present : s.hasKey,
    }));

  return {
    settings: null,
    hasKey: false,
    keys: {},
    error: null,

    load: async () => {
      try {
        const settings = await getSettings();
        set({ settings, error: null });
        await get().refreshKey(settings.sttProvider);
        const llm = llmKeyAccount(settings.llmProvider);
        if (llm) await get().refreshKey(llm);
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

    refreshKey: async (provider) => {
      try {
        setKey(provider, await hasApiKey(provider));
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    saveKey: async (provider, key) => {
      try {
        await setApiKey(provider, key);
        setKey(provider, true);
        set({ error: null });
        return true;
      } catch (e) {
        set({ error: errorMessage(e) });
        return false;
      }
    },

    deleteKey: async (provider) => {
      try {
        await deleteApiKey(provider);
        setKey(provider, false);
        set({ error: null });
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },
  };
});
