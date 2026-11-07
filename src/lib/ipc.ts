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
/** Motore di trascrizione. */
export type SttEngine = "local" | "deepgram";
export type LlmProvider =
  | "ollama"
  | "opencode"
  | "opencode-go"
  | "openai"
  | "anthropic"
  | "custom";
/** Account delle API key nel Keychain. */
export type KeyProvider = "deepgram" | "opencode" | "openai" | "anthropic" | "custom";

/** Account della chiave del provider LLM; `null` = nessuna chiave. */
export function llmKeyAccount(p: LlmProvider): KeyProvider | null {
  if (p === "ollama") return null;
  return p === "opencode-go" ? "opencode" : p;
}

export interface Settings {
  /** Con `localOnly` vale comunque il motore locale. */
  sttEngine: SttEngine;
  /** Modello Deepgram. */
  sttModel: string;
  /** Id del modello di trascrizione locale (catalogo modelli). */
  localSttModel: string;
  /** Privacy: niente audio né testo verso host esterni durante la sessione. */
  localOnly: boolean;
  /** Riconoscimento dei parlanti su "Loro" (modello CAM++). */
  diarization: boolean;
  /** Altri partecipanti noti (1–8); 0 = automatico. */
  theirSpeakers: number;
  myLanguage: Language;
  theirLanguage: Language;
  micDeviceId: string | null;
  systemAudio: boolean;
  llmProvider: LlmProvider;
  llmBaseUrl: string;
  translationModel: string;
  assistModel: string;
  /** Riepilogo finale automatico allo stop. */
  autoSummary: boolean;
  /** `null` = la lingua dell'utente. */
  targetLanguage: Language | null;
}

export interface ModelInfo {
  id: string;
  name: string | null;
  /** Usabile da Sottovoce (per i provider OpenAI-compatible: chat completions). */
  usable: boolean;
}

export interface LlmTestResult {
  model: string;
  latencyMs: number;
  reply: string;
}

/** Tempi in ms dall'avvio della cattura. */
export interface Segment {
  /** Id della frase nella sessione (solo nei `final`; 0 nei parziali). */
  id: number;
  text: string;
  start: number;
  end: number;
  confidence: number | null;
  /** Lingua rilevata (es. "en"), se disponibile. */
  language: string | null;
  /** Parlante su "Loro" (0 = "Speaker 1"); `null` = sconosciuto o "Io". */
  speaker: number | null;
}

export interface SpeakerUpdate {
  segmentId: number;
  speaker: number | null;
}

export type AssistKind = "say" | "ask";

/** Suggerimento del live assist; `at` = ms dall'avvio della sessione. */
export interface AssistEvent {
  id: number;
  kind: AssistKind;
  text: string;
  at: number;
}

export interface TranslationEvent {
  segmentId: number;
  text: string;
  latencyMs: number;
}

export type MomentKind = "objection" | "commitment" | "question" | "pain_point" | "decision";

/** Momento importante emerso dalla compressione dei 5 minuti. */
export interface ImportantMoment {
  timeMs: number;
  kind: MomentKind;
  text: string;
}

/** Blocco di compressione di 5 minuti. */
export interface CompressionEvent {
  index: number;
  start: number;
  end: number;
  summary: string;
  topics: string[];
  moments: ImportantMoment[];
}

export interface KeyPoint {
  topic: string;
  points: string[];
}

export interface ActionItem {
  text: string;
  done: boolean;
}

/** Riepilogo finale salvato. */
export interface Summary {
  meetingId: number;
  language: Language;
  overview: string;
  keyPoints: KeyPoint[];
  actionItems: ActionItem[];
  createdAtMs: number;
}

export type SttEvent = { channel: ChannelId } & (
  | { type: "connected" }
  | ({ type: "partial" } & Segment)
  | ({ type: "final" } & Segment)
  | { type: "reconnecting"; attempt: number }
  | { type: "error"; message: string }
  | { type: "disconnected" }
  | { type: "speakers"; updates: SpeakerUpdate[] }
);

export const listInputDevices = () => invoke<InputDevice[]>("list_input_devices");

export const systemAudioSupported = () => invoke<boolean>("system_audio_supported");

/** Contesto del meeting: vive nella sessione, non nei settings. */
export interface MeetingContext {
  name: string;
  goal: string;
  notes: string;
  /** Nomi dei parlanti di "Loro" per indice ("0" = Speaker 1), per la sessione. */
  speakers: Record<string, string>;
}

export function startSession(
  onLevel: (e: LevelEvent) => void,
  onTranscript: (e: SttEvent) => void,
  onTranslation: (e: TranslationEvent) => void,
  onAssist: (e: AssistEvent) => void,
  onCompression: (e: CompressionEvent) => void,
  context: MeetingContext,
) {
  const level = new Channel<LevelEvent>();
  level.onmessage = onLevel;
  const transcript = new Channel<SttEvent>();
  transcript.onmessage = onTranscript;
  const translation = new Channel<TranslationEvent>();
  translation.onmessage = onTranslation;
  const assist = new Channel<AssistEvent>();
  assist.onmessage = onAssist;
  const compression = new Channel<CompressionEvent>();
  compression.onmessage = onCompression;
  return invoke<void>("start_session", {
    onLevel: level,
    onTranscript: transcript,
    onTranslation: translation,
    onAssist: assist,
    onCompression: compression,
    context,
  });
}

export const stopSession = () => invoke<Summary | null>("stop_session");

// --- Storico meeting ---------------------------------------------------------

/** Meeting salvato; le date sono ms Unix, i tempi delle frasi ms dalla sessione. */
export interface Meeting {
  id: number;
  name: string;
  goal: string;
  notes: string;
  startedAtMs: number;
  endedAtMs: number | null;
  sttEngine: SttEngine;
  sttModel: string;
  myLanguage: Language;
  theirLanguage: Language;
}

export interface MeetingSummary extends Meeting {
  segmentCount: number;
}

export interface MeetingPage {
  meetings: MeetingSummary[];
  total: number;
}

export interface SavedSegment {
  id: number;
  meetingId: number;
  sessionId: number;
  channel: ChannelId;
  speaker: number | null;
  text: string;
  startMs: number;
  endMs: number;
  language: string | null;
  confidence: number | null;
}

export interface SavedTranslation {
  id: number;
  segmentId: number;
  sessionId: number;
  text: string;
  language: string;
  latencyMs: number;
}

export interface SavedSpeaker {
  speaker: number;
  name: string;
}

export interface SavedAssist {
  id: number;
  kind: AssistKind;
  text: string;
  atMs: number;
}

export interface MeetingDetails {
  meeting: Meeting;
  segments: SavedSegment[];
  translations: SavedTranslation[];
  speakers: SavedSpeaker[];
  assists: SavedAssist[];
  blocks: CompressionEvent[];
  summary: Summary | null;
}

export const listMeetings = (page: number, perPage: number, search?: string) =>
  invoke<MeetingPage>("list_meetings", { page, perPage, search: search?.trim() || null });

export const getMeeting = (id: number) => invoke<MeetingDetails>("get_meeting", { id });

export const deleteMeeting = (id: number) => invoke<void>("delete_meeting", { id });

export const renameMeeting = (id: number, name: string) =>
  invoke<Meeting>("rename_meeting", { id, name });

export const generateSummary = (id: number, language?: Language) =>
  invoke<Summary>("generate_summary", { id, language: language ?? null });

export const updateSummaryChecklist = (id: number, items: ActionItem[]) =>
  invoke<Summary>("update_summary_checklist", { id, items });

export const updateMeetingContext = (context: MeetingContext) =>
  invoke<void>("update_meeting_context", { context });

export const getSettings = () => invoke<Settings>("get_settings");

export const updateSettings = (patch: Partial<Settings>) =>
  invoke<Settings>("update_settings", { patch });

export const setApiKey = (provider: KeyProvider, key: string) =>
  invoke<void>("set_api_key", { provider, key });

export const hasApiKey = (provider: KeyProvider) => invoke<boolean>("has_api_key", { provider });

export const deleteApiKey = (provider: KeyProvider) =>
  invoke<void>("delete_api_key", { provider });

export const listLlmModels = () => invoke<ModelInfo[]>("list_llm_models");

export const testLlm = (model?: string) => invoke<LlmTestResult>("test_llm", { model });

export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

// --- Modelli locali ---------------------------------------------------------

export type LocalModelKind = "vad" | "stt" | "speakerEmbedding";

export type LocalModelStatus =
  | { state: "missing" }
  | { state: "partial"; downloaded: number }
  | { state: "ready" }
  | { state: "corrupt" };

export interface LocalModel {
  id: string;
  kind: LocalModelKind;
  name: string;
  files: { name: string; size: number }[];
  size: number;
  status: LocalModelStatus;
  onDisk: number;
  downloading: boolean;
}

export interface DownloadProgress {
  downloaded: number;
  total: number;
}

export const listLocalModels = () => invoke<LocalModel[]>("list_local_models");

export function downloadLocalModel(id: string, onProgress: (p: DownloadProgress) => void) {
  const channel = new Channel<DownloadProgress>();
  channel.onmessage = onProgress;
  return invoke<LocalModel>("download_local_model", { id, onProgress: channel });
}

export const cancelLocalModelDownload = (id: string) =>
  invoke<boolean>("cancel_local_model_download", { id });

export const deleteLocalModel = (id: string) => invoke<LocalModel>("delete_local_model", { id });

export const verifyLocalModel = (id: string) => invoke<LocalModel>("verify_local_model", { id });
