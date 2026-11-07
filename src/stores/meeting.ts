import { create } from "zustand";

import { type MeetingContext, errorMessage, updateMeetingContext } from "@/lib/ipc";

/** Limiti dei campi (allineati a `src-tauri/src/meeting.rs`). */
export const MEETING_LIMITS = { name: 200, goal: 500, notes: 2000 } as const;

/** Attesa dopo l'ultima modifica prima di inviare il contesto alla sessione. */
const SYNC_DELAY_MS = 600;

interface MeetingState {
  context: MeetingContext;
  syncError: string | null;
  /** Modifica locale; durante la sessione viene inviata con un debounce. */
  set: (patch: Partial<MeetingContext>) => void;
}

let timer: ReturnType<typeof setTimeout> | undefined;

export const useMeetingStore = create<MeetingState>((set, get) => ({
  context: { name: "", goal: "", notes: "" },
  syncError: null,

  set: (patch) => {
    set((s) => ({ context: { ...s.context, ...patch } }));
    clearTimeout(timer);
    // Senza sessione il backend ignora l'aggiornamento: il contesto parte
    // comunque con `start_session`.
    timer = setTimeout(() => {
      updateMeetingContext(get().context).then(
        () => set({ syncError: null }),
        (e: unknown) => set({ syncError: errorMessage(e) }),
      );
    }, SYNC_DELAY_MS);
  },
}));
