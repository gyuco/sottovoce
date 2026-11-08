import { create } from "zustand";

import { type MeetingContext, errorMessage, updateMeetingContext } from "@/lib/ipc";

/** Limiti dei campi (allineati a `src-tauri/src/meeting.rs`). */
export const MEETING_LIMITS = {
  name: 200,
  goal: 500,
  notes: 2000,
  codePath: 1024,
  speakerName: 60,
} as const;

/** Attesa dopo l'ultima modifica prima di inviare il contesto alla sessione. */
const SYNC_DELAY_MS = 600;

interface MeetingState {
  context: MeetingContext;
  syncError: string | null;
  /** Modifica locale; durante la sessione viene inviata con un debounce. */
  set: (patch: Partial<MeetingContext>) => void;
  /** Nome di un parlante di "Loro"; vuoto = torna "Speaker N". */
  renameSpeaker: (speaker: number, name: string) => void;
}

let timer: ReturnType<typeof setTimeout> | undefined;

export const useMeetingStore = create<MeetingState>((set, get) => ({
  context: { name: "", goal: "", notes: "", codePath: "", speakers: {} },
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

  renameSpeaker: (speaker, name) => {
    const speakers = { ...get().context.speakers };
    const trimmed = name.trim().slice(0, MEETING_LIMITS.speakerName);
    if (trimmed) speakers[speaker] = trimmed;
    else delete speakers[speaker];
    get().set({ speakers });
  },
}));
