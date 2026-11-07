import { Channel, invoke } from "@tauri-apps/api/core";

export type ChannelId = "me" | "them";

export interface InputDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface LevelEvent {
  channel: ChannelId;
  rmsDb: number;
  peakDb: number;
}

export type Language = "multi" | "it" | "en";
export type SttProvider = "deepgram";

export interface Settings {
  sttProvider: SttProvider;
  sttModel: string;
  myLanguage: Language;
  theirLanguage: Language;
  micDeviceId: string | null;
  systemAudio: boolean;
}

/** Tempi in ms dall'avvio della cattura. */
export interface Segment {
  text: string;
  start: number;
  end: number;
  confidence: number | null;
}

export type SttEvent = { channel: ChannelId } & (
  | { type: "connected" }
  | ({ type: "partial" } & Segment)
  | ({ type: "final" } & Segment)
  | { type: "reconnecting"; attempt: number }
  | { type: "error"; message: string }
  | { type: "disconnected" }
);

export const listInputDevices = () => invoke<InputDevice[]>("list_input_devices");

export const systemAudioSupported = () => invoke<boolean>("system_audio_supported");

export function startSession(
  onLevel: (e: LevelEvent) => void,
  onTranscript: (e: SttEvent) => void,
) {
  const level = new Channel<LevelEvent>();
  level.onmessage = onLevel;
  const transcript = new Channel<SttEvent>();
  transcript.onmessage = onTranscript;
  return invoke<void>("start_session", { onLevel: level, onTranscript: transcript });
}

export const stopSession = () => invoke<void>("stop_session");

export const getSettings = () => invoke<Settings>("get_settings");

export const updateSettings = (patch: Partial<Settings>) =>
  invoke<Settings>("update_settings", { patch });

export const setApiKey = (provider: SttProvider, key: string) =>
  invoke<void>("set_api_key", { provider, key });

export const hasApiKey = (provider: SttProvider) => invoke<boolean>("has_api_key", { provider });

export const deleteApiKey = (provider: SttProvider) =>
  invoke<void>("delete_api_key", { provider });

export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
