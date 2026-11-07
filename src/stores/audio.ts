import { create } from "zustand";

import {
  type ChannelId,
  type InputDevice,
  type LevelEvent,
  errorMessage,
  listInputDevices,
  startSession,
  stopSession,
  systemAudioSupported,
} from "@/lib/ipc";
import { useAssistStore } from "@/stores/assist";
import { useMeetingStore } from "@/stores/meeting";
import { useSettingsStore } from "@/stores/settings";
import { useTranscriptStore } from "@/stores/transcript";

export const FLOOR_DB = -100;
/** Durata del picco mantenuto prima che inizi a scendere. */
const PEAK_HOLD_MS = 1200;
/** Velocità di discesa del picco dopo l'hold (dB/s). */
const PEAK_DECAY_DB_PER_S = 20;

export interface ChannelLevel {
  rmsDb: number;
  peakDb: number;
  holdDb: number;
  holdAt: number;
  /** Timestamp (ms) dell'ultimo evento ricevuto; 0 = mai. */
  lastAt: number;
}

type Status = "idle" | "starting" | "running" | "stopping";

interface AudioState {
  devices: InputDevice[];
  systemSupported: boolean;
  status: Status;
  startedAt: number;
  /** Ora di inizio (epoch ms) per mostrarla all'utente. */
  startedWall: number;
  error: { key: string; message: string } | null;
  levels: Record<ChannelId, ChannelLevel>;
  init: () => Promise<void>;
  setMic: (id: string) => Promise<void>;
  start: () => Promise<void>;
  stop: () => Promise<void>;
}

const silent = (): ChannelLevel => ({
  rmsDb: FLOOR_DB,
  peakDb: FLOOR_DB,
  holdDb: FLOOR_DB,
  holdAt: 0,
  lastAt: 0,
});

function nextLevel(prev: ChannelLevel, e: LevelEvent, now: number): ChannelLevel {
  const elapsed = now - prev.holdAt;
  const decayed =
    elapsed <= PEAK_HOLD_MS
      ? prev.holdDb
      : prev.holdDb - ((elapsed - PEAK_HOLD_MS) / 1000) * PEAK_DECAY_DB_PER_S;
  const fresh = e.peakDb >= decayed;
  return {
    rmsDb: e.rmsDb,
    peakDb: e.peakDb,
    holdDb: fresh ? e.peakDb : Math.max(decayed, FLOOR_DB),
    holdAt: fresh ? now : prev.holdAt,
    lastAt: now,
  };
}

export const useAudioStore = create<AudioState>((set, get) => ({
  devices: [],
  systemSupported: false,
  status: "idle",
  startedAt: 0,
  startedWall: 0,
  error: null,
  levels: { me: silent(), them: silent() },

  init: async () => {
    try {
      const [devices, systemSupported] = await Promise.all([
        listInputDevices(),
        systemAudioSupported(),
      ]);
      set({ devices, systemSupported });
    } catch (e) {
      set({ error: { key: "errors.devices", message: errorMessage(e) } });
    }
  },

  setMic: async (micDeviceId) => {
    await useSettingsStore.getState().update({ micDeviceId });
    // Cambio microfono a sessione attiva: riavvia sul nuovo device.
    if (get().status === "running") await get().start();
  },

  start: async () => {
    set({ status: "starting", error: null, levels: { me: silent(), them: silent() } });
    const transcript = useTranscriptStore.getState();
    transcript.reset();
    const assist = useAssistStore.getState();
    assist.reset();
    try {
      await startSession(
        (e) => {
          set((s) => ({
            levels: {
              ...s.levels,
              [e.channel]: nextLevel(s.levels[e.channel], e, performance.now()),
            },
          }));
        },
        transcript.apply,
        transcript.applyTranslation,
        assist.add,
        useMeetingStore.getState().context,
      );
      set({ status: "running", startedAt: performance.now(), startedWall: Date.now() });
    } catch (e) {
      set({ status: "idle", error: { key: "errors.start", message: errorMessage(e) } });
    }
  },

  stop: async () => {
    set({ status: "stopping" });
    try {
      await stopSession();
    } finally {
      set({ status: "idle", levels: { me: silent(), them: silent() } });
    }
  },
}));

/** Microfono scelto: quello salvato se presente, altrimenti il predefinito. */
export function useSelectedMicId() {
  const devices = useAudioStore((s) => s.devices);
  const saved = useSettingsStore((s) => s.settings?.micDeviceId);
  return (
    devices.find((d) => d.id === saved)?.id ??
    devices.find((d) => d.isDefault)?.id ??
    devices[0]?.id
  );
}
