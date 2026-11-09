import { create } from "zustand";

import { type MeetingContext, type VoiceEvent, errorMessage, updateMeetingContext } from "@/lib/ipc";

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
  /** Parlanti che somigliano a un profilo vocale, in attesa di conferma. */
  voiceSuggestions: Record<number, string>;
  /** Parlanti a cui il profilo vocale ha dato il nome (per "non è lui"). */
  voiceRecognized: Record<number, string>;
  /** Esito del riconoscimento vocale dal backend. */
  applyVoice: (e: VoiceEvent) => void;
  acceptVoice: (speaker: number) => void;
  /** Rifiuta un suggerimento o toglie un nome riconosciuto. */
  rejectVoice: (speaker: number) => void;
  /** Chiude l'avviso di un nome riconosciuto, tenendolo. */
  keepVoice: (speaker: number) => void;
  /** Nuova sessione: parlanti rinumerati, nomi e suggerimenti azzerati. */
  resetSpeakers: () => void;
  /** Modifica locale; durante la sessione viene inviata con un debounce. */
  set: (patch: Partial<MeetingContext>) => void;
  /** Nome di un parlante di "Loro"; vuoto = torna "Speaker N". */
  renameSpeaker: (speaker: number, name: string) => void;
}

let timer: ReturnType<typeof setTimeout> | undefined;

function without<T>(map: Record<number, T>, key: number): Record<number, T> {
  const next = { ...map };
  delete next[key];
  return next;
}

export const useMeetingStore = create<MeetingState>((set, get) => ({
  context: { name: "", goal: "", notes: "", codePath: "", speakers: {} },
  syncError: null,
  voiceSuggestions: {},
  voiceRecognized: {},

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
    // Un nome dato a mano prende il posto di quello riconosciuto.
    set((s) => ({
      voiceRecognized: without(s.voiceRecognized, speaker),
      voiceSuggestions: without(s.voiceSuggestions, speaker),
    }));
    get().set({ speakers });
  },

  applyVoice: (e) => {
    const { context } = get();
    // Mai sopra un nome dato dall'utente, mai lo stesso nome a due parlanti.
    if (context.speakers[e.speaker] !== undefined) return;
    if (Object.values(context.speakers).includes(e.name)) return;
    if (e.verdict === "suggest") {
      set((s) => ({ voiceSuggestions: { ...s.voiceSuggestions, [e.speaker]: e.name } }));
      return;
    }
    get().renameSpeaker(e.speaker, e.name);
    set((s) => ({ voiceRecognized: { ...s.voiceRecognized, [e.speaker]: e.name } }));
  },

  acceptVoice: (speaker) => {
    const name = get().voiceSuggestions[speaker];
    if (name) get().renameSpeaker(speaker, name);
  },

  rejectVoice: (speaker) => {
    if (get().voiceRecognized[speaker] !== undefined) {
      get().renameSpeaker(speaker, "");
    } else {
      set((s) => ({ voiceSuggestions: without(s.voiceSuggestions, speaker) }));
    }
  },

  keepVoice: (speaker) => set((s) => ({ voiceRecognized: without(s.voiceRecognized, speaker) })),

  resetSpeakers: () =>
    set((s) => ({
      context: { ...s.context, speakers: {} },
      voiceSuggestions: {},
      voiceRecognized: {},
    })),
}));
