import { create } from "zustand";

import {
  type VoiceProfile,
  deleteAllVoiceProfiles,
  deleteVoiceProfile,
  errorMessage,
  listVoiceProfiles,
  renameVoiceProfile,
} from "@/lib/ipc";

interface VoicesState {
  profiles: VoiceProfile[];
  error: string | null;
  load: () => Promise<void>;
  rename: (id: string, name: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
  removeAll: () => Promise<void>;
}

/** Profili vocali salvati (Impostazioni). */
export const useVoicesStore = create<VoicesState>((set) => {
  const run = async (action: () => Promise<unknown>) => {
    try {
      await action();
      set({ profiles: await listVoiceProfiles(), error: null });
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  };
  return {
    profiles: [],
    error: null,
    load: () => run(async () => {}),
    rename: (id, name) => run(() => renameVoiceProfile(id, name)),
    remove: (id) => run(() => deleteVoiceProfile(id)),
    removeAll: () => run(deleteAllVoiceProfiles),
  };
});
