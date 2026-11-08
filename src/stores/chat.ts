import { create } from "zustand";

import { type ChatTurn, askModel, errorMessage } from "@/lib/ipc";
import { useMeetingStore } from "@/stores/meeting";
import { useTranscriptStore } from "@/stores/transcript";

/** Frasi recenti di trascrizione passate come contesto alla chat. */
const CONTEXT_LINES = 60;
/** Turni precedenti inviati al modello. */
const HISTORY_TURNS = 8;

export interface ChatLine {
  id: number;
  role: "user" | "assistant";
  text: string;
}

interface ChatState {
  lines: ChatLine[];
  sending: boolean;
  error: string | null;
  /** Invia la domanda con contesto meeting + trascrizione recente. */
  ask: (text: string) => Promise<void>;
  reset: () => void;
}

let nextId = 1;

/** Trascrizione etichettata come nel resto dei prompt ("Io"/"Loro"/nomi). */
function callTranscript(): string {
  const lines = useTranscriptStore.getState().lines.slice(-CONTEXT_LINES);
  const names = useMeetingStore.getState().context.speakers;
  return lines
    .map((l) => {
      const who =
        l.channel === "me"
          ? "Io"
          : l.speaker === null
            ? "Loro"
            : (names[l.speaker] ?? `Speaker ${l.speaker + 1}`);
      return `${who}: ${l.text.trim()}`;
    })
    .join("\n");
}

export const useChatStore = create<ChatState>((set, get) => ({
  lines: [],
  sending: false,
  error: null,

  ask: async (text) => {
    const question = text.trim();
    if (question === "" || get().sending) return;
    nextId += 1;
    const user: ChatLine = { id: nextId, role: "user", text: question };
    set((s) => ({ lines: [...s.lines, user], sending: true, error: null }));
    const history: ChatTurn[] = get()
      .lines.filter((l) => l.id !== user.id)
      .slice(-HISTORY_TURNS)
      .map((l) => ({ role: l.role, content: l.text }));
    try {
      const reply = await askModel({
        question,
        history,
        transcript: callTranscript(),
        context: useMeetingStore.getState().context,
      });
      nextId += 1;
      set((s) => ({
        lines: [...s.lines, { id: nextId, role: "assistant", text: reply }],
        sending: false,
      }));
    } catch (e) {
      set({ sending: false, error: errorMessage(e) });
    }
  },

  reset: () => set({ lines: [], sending: false, error: null }),
}));
