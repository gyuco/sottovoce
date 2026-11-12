import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";
import { isTauri } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { create } from "zustand";

import {
  type Participant,
  type ScheduledMeeting,
  type ScheduledMeetingInput,
  deleteScheduledMeeting,
  errorMessage,
  listScheduledMeetings,
  saveScheduledMeeting,
} from "@/lib/ipc";
import { useAudioStore } from "@/stores/audio";
import { findContact, useContactsStore } from "@/stores/contacts";
import { useMeetingStore } from "@/stores/meeting";

/** Da quanto prima dell'inizio la riunione si propone in Live. */
export const DUE_BEFORE_MS = 10 * 60_000;
/** Riunioni non avviate ancora mostrate dopo la fine prevista. */
const MISSED_WINDOW_MS = 7 * 24 * 60 * 60_000;
/** Ogni quanto si ricontrolla l'ora. */
const TICK_MS = 20_000;

export const SCHEDULED_DURATIONS = [15, 30, 45, 60, 90, 120] as const;

export function endsAt(m: ScheduledMeeting): number {
  return m.startsAtMs + m.durationMin * 60_000;
}

/** È ora (o quasi): da 10 minuti prima dell'inizio fino alla fine prevista. */
export function isDue(m: ScheduledMeeting, now: number): boolean {
  return now >= m.startsAtMs - DUE_BEFORE_MS && now < endsAt(m);
}

interface ScheduledState {
  meetings: ScheduledMeeting[];
  loaded: boolean;
  error: string | null;
  /** `Date.now()` dell'ultimo controllo (aggiornato dal ticker). */
  now: number;
  /** Riunioni nascoste dall'avviso in Live per questa esecuzione. */
  dismissed: number[];
  load: () => Promise<void>;
  /** Crea o aggiorna; lancia l'errore (lo mostra il form). */
  save: (input: ScheduledMeetingInput) => Promise<ScheduledMeeting>;
  remove: (id: number) => Promise<void>;
  dismiss: (id: number) => void;
  tick: () => void;
  /**
   * Avvia la call con il contesto della riunione: nome, obiettivo, note,
   * cartella del codice, partecipanti (ruoli riletti dalla rubrica) e lingua.
   * Avviata la sessione, la riunione esce dall'agenda.
   */
  start: (m: ScheduledMeeting) => Promise<void>;
}

/** Ruolo e descrizione aggiornati dalla rubrica, se il contatto c'è ancora. */
export function freshParticipants(participants: Participant[]): Participant[] {
  const contacts = useContactsStore.getState().contacts;
  return participants.map((p) => {
    const c = findContact(contacts, p.name);
    return c ? { name: c.name, role: c.role, description: c.description } : p;
  });
}

/** Contesto del meeting preso dalla riunione programmata. */
export function applyScheduledContext(m: ScheduledMeeting) {
  useMeetingStore.getState().set({
    name: m.name,
    goal: m.goal,
    notes: m.notes,
    codePath: m.codePath,
    participants: freshParticipants(m.participants),
  });
}

export const useScheduledStore = create<ScheduledState>((set, get) => ({
  meetings: [],
  loaded: false,
  error: null,
  now: Date.now(),
  dismissed: [],

  load: async () => {
    try {
      const meetings = await listScheduledMeetings(Date.now() - MISSED_WINDOW_MS);
      set({ meetings, loaded: true, error: null, now: Date.now() });
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },

  save: async (input) => {
    const meeting = await saveScheduledMeeting(input);
    // Spostata a un'altra ora: l'avviso torna a valere.
    set((s) => ({ dismissed: s.dismissed.filter((id) => id !== meeting.id) }));
    await get().load();
    return meeting;
  },

  remove: async (id) => {
    try {
      await deleteScheduledMeeting(id);
      await get().load();
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },

  dismiss: (id) => set((s) => ({ dismissed: [...s.dismissed, id] })),

  tick: () => set({ now: Date.now() }),

  start: async (m) => {
    const audio = useAudioStore.getState();
    if (audio.status !== "idle") return;
    applyScheduledContext(m);
    await audio.start(m.language);
    if (useAudioStore.getState().status === "running") await get().remove(m.id);
  },
}));

/** Riunione da proporre adesso in Live (la più vicina), se c'è. */
export function useDueMeeting(): ScheduledMeeting | null {
  const meetings = useScheduledStore((s) => s.meetings);
  const now = useScheduledStore((s) => s.now);
  const dismissed = useScheduledStore((s) => s.dismissed);
  return meetings.find((m) => isDue(m, now) && !dismissed.includes(m.id)) ?? null;
}

/** Riunioni per cui la finestra ha già chiesto attenzione. */
const attended = new Set<number>();

/**
 * Tiene aggiornata l'ora e, quando una riunione inizia, chiede attenzione
 * (icona del Dock che rimbalza) se la finestra non è in primo piano.
 */
export function useScheduledTicker() {
  useEffect(() => {
    const store = useScheduledStore.getState();
    void store.load();
    const id = setInterval(() => {
      useScheduledStore.getState().tick();
      const { meetings, now } = useScheduledStore.getState();
      const starting = meetings.filter((m) => now >= m.startsAtMs && now < endsAt(m) && !attended.has(m.id));
      if (starting.length === 0) return;
      for (const m of starting) attended.add(m.id);
      if (isTauri() && useAudioStore.getState().status === "idle") {
        void getCurrentWindow()
          .requestUserAttention(UserAttentionType.Informational)
          .catch(() => undefined);
      }
    }, TICK_MS);
    return () => clearInterval(id);
  }, []);
}
