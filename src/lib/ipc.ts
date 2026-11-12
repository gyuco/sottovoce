import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

export type Language = "multi" | "it" | "en" | "es" | "fr" | "de";
/** Lingue concrete (STT, traduzione, riepilogo). */
export const CONCRETE_LANGUAGES = ["it", "en", "es", "fr", "de"] as const satisfies Language[];
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
export type KeyProvider =
  | "deepgram"
  | "opencode"
  | "openai"
  | "anthropic"
  | "custom"
  | "smtp"
  | "google-client-secret";

/** Cifratura SMTP: STARTTLS (587) o TLS dall'inizio (465). */
export type SmtpSecurity = "starttls" | "tls";

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
  /** Profili vocali (dato biometrico, opt-in): riconosce i parlanti già nominati. */
  voiceProfiles: boolean;
  myLanguage: Language;
  theirLanguage: Language;
  micDeviceId: string | null;
  systemAudio: boolean;
  llmProvider: LlmProvider;
  llmBaseUrl: string;
  translationModel: string;
  /** `null` = stesso provider di `llmProvider`. */
  translationProvider: LlmProvider | null;
  /** Traduce da sola ogni frase di "Loro"; spento = pulsante "Traduci". */
  autoTranslate: boolean;
  /** Con la traduzione automatica traduce anche "Io" nella lingua del meeting. */
  translateMine: boolean;
  assistModel: string;
  /** `null` = stesso provider di `llmProvider` (vale anche per compressione e riepilogo). */
  assistProvider: LlmProvider | null;
  /** Riepilogo finale automatico allo stop. */
  autoSummary: boolean;
  /** `null` = la lingua dell'utente. */
  targetLanguage: Language | null;
  /** Avvisi durante la call calcolati dalle metriche (senza LLM). */
  nudges: boolean;
  /** Suggerimenti del live assist (cosa dire/chiedere); spenti di default. */
  liveAssist: boolean;
  /** Cancellazione d'eco sul microfono (serve l'audio di sistema). */
  echoCancellation: boolean;
  /** Ricerca per significato nello storico (modello locale di embedding). */
  semanticSearch: boolean;
  /** Mittente delle email del riepilogo. */
  emailFromName: string;
  /** Vuoto = invio SMTP non configurato (resta la bozza nel client di posta). */
  emailFrom: string;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: SmtpSecurity;
  /** Vuoto = l'indirizzo del mittente. La password è nel Keychain (`smtp`). */
  smtpUser: string;
  /** ID del client OAuth "App desktop" di Google Cloud; il secret è nel Keychain. */
  googleClientId: string;
  /** Account Google collegato (vuoto = non collegato). */
  googleCalendarEmail: string;
  /** ID applicazione (client) registrata su Microsoft Entra ID (client pubblico). */
  microsoftClientId: string;
  /** Tenant di Entra ID; vuoto = `common` (account aziendali e personali). */
  microsoftTenantId: string;
  /** Account Microsoft collegato (vuoto = non collegato). */
  microsoftCalendarEmail: string;
  /** Setup guidato del primo avvio completato (o saltato). */
  onboardingDone: boolean;
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
  /** Lingua della traduzione (es. "en"). */
  language: string;
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

/** Permesso del microfono (macOS); `unsupported` = nessun permesso da chiedere. */
export type MicPermission = "granted" | "denied" | "restricted" | "undetermined" | "unsupported";

export const micPermission = () => invoke<MicPermission>("mic_permission");

/** Mostra il dialogo di sistema se il permesso non è mai stato chiesto; `true` = concesso. */
export const requestMicPermission = () => invoke<boolean>("request_mic_permission");

/** Prova audio del setup: solo livelli di microfono e audio di sistema. */
export function startAudioCheck(onLevel: (e: LevelEvent) => void) {
  const level = new Channel<LevelEvent>();
  level.onmessage = onLevel;
  return invoke<void>("start_audio_check", { onLevel: level });
}

export const stopAudioCheck = () => invoke<void>("stop_audio_check");

/** Apre le Impostazioni di Sistema sul permesso indicato. */
export const openPrivacySettings = (pane: "microphone" | "systemAudio") =>
  invoke<void>("open_privacy_settings", { pane });

/** Partecipante atteso; ruolo e descrizione copiati dalla rubrica. */
export interface Participant {
  name: string;
  role: string;
  description: string;
}

/** Contesto del meeting: vive nella sessione, non nei settings. */
export interface MeetingContext {
  name: string;
  goal: string;
  notes: string;
  /** Cartella del progetto locale da cui il live assist legge il codice. */
  codePath: string;
  /** Nomi dei parlanti di "Loro" per indice ("0" = Speaker 1), per la sessione. */
  speakers: Record<string, string>;
  /** Partecipanti attesi (facoltativi), chiesti all'avvio. */
  participants: Participant[];
}

/** Esito della verifica di una cartella di codice. */
export interface CodePathInfo {
  path: string;
  exists: boolean;
  isDir: boolean;
  files: number;
  truncated: boolean;
  languages: { name: string; files: number }[];
  outline: string;
}

export const inspectCodePath = (path: string) =>
  invoke<CodePathInfo>("inspect_code_path", { path });

export type ChatRole = "user" | "assistant";

export interface ChatTurn {
  role: ChatRole;
  content: string;
}

/** Domanda libera al modello durante la call; risposta completa (no streaming). */
export const askModel = (input: {
  question: string;
  history: ChatTurn[];
  transcript: string;
  context: MeetingContext;
}) => invoke<string>("ask_model", input);

/** Filtri opzionali delle domande sullo storico (ms epoch). */
export interface HistoryFilter {
  fromMs?: number | null;
  /** Escluso. */
  toMs?: number | null;
  meetingId?: number | null;
}

/** Fonte citata in una risposta sullo storico. */
export interface HistorySource {
  n: number;
  meetingId: number;
  meetingName: string;
  startedAtMs: number;
  kind: PassageKind;
  segmentId: number | null;
  startMs: number;
  text: string;
}

export interface HistoryAnswer {
  /** Risposta con citazioni `[n]`; vuota se `found` è falso. */
  answer: string;
  /** `false` = nei meeting non c'è la risposta. */
  found: boolean;
  /** `false` = solo ricerca per parole (modello di embedding assente). */
  semantic: boolean;
  sources: HistorySource[];
}

/** Domanda su tutti i meeting salvati (RAG); risposta con le fonti citate. */
export const askHistory = (input: {
  question: string;
  history: ChatTurn[];
  filter: HistoryFilter | null;
}) =>
  invoke<HistoryAnswer>("ask_history", {
    ...input,
    utcOffsetMin: -new Date().getTimezoneOffset(),
  });

export function startSession(
  onLevel: (e: LevelEvent) => void,
  onTranscript: (e: SttEvent) => void,
  onTranslation: (e: TranslationEvent) => void,
  onAssist: (e: AssistEvent) => void,
  onCompression: (e: CompressionEvent) => void,
  onVoice: (e: VoiceEvent) => void,
  context: MeetingContext,
  /** Lingua della call: rilevamento di entrambi i canali. */
  language: Language,
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
  const voice = new Channel<VoiceEvent>();
  voice.onmessage = onVoice;
  return invoke<void>("start_session", {
    onLevel: level,
    onTranscript: transcript,
    onTranslation: translation,
    onAssist: assist,
    onCompression: compression,
    onVoice: voice,
    context,
    language,
  });
}

/** Parlante di "Loro" riconosciuto in un profilo vocale. */
export interface VoiceEvent {
  speaker: number;
  profileId: string;
  name: string;
  similarity: number;
  /** `assign` = nome usato subito; `suggest` = da confermare. */
  verdict: "assign" | "suggest";
}

/** Profilo vocale salvato (l'impronta resta nel backend). */
export interface VoiceProfile {
  id: string;
  name: string;
  createdMs: number;
  /** Ultimo meeting in cui è stato aggiornato. */
  lastSeenMs: number;
  meetings: number;
}

export const listVoiceProfiles = () => invoke<VoiceProfile[]>("list_voice_profiles");

export const renameVoiceProfile = (id: string, name: string) =>
  invoke<VoiceProfile>("rename_voice_profile", { id, name });

export const deleteVoiceProfile = (id: string) => invoke<void>("delete_voice_profile", { id });

export const deleteAllVoiceProfiles = () => invoke<void>("delete_all_voice_profiles");

/** Esito dello stop: il riepilogo, se previsto, arriva dopo con `onSummaryStatus`. */
export interface StopResult {
  meetingId: number | null;
  summaryPending: boolean;
}

export const stopSession = () => invoke<StopResult>("stop_session");

/** Esito del riepilogo finale generato in background dopo lo stop. */
export type SummaryStatus =
  | { state: "ready"; meetingId: number; summary: Summary }
  | { state: "failed"; meetingId: number; message: string };

export const onSummaryStatus = (handler: (status: SummaryStatus) => void) =>
  listen<SummaryStatus>("summary-status", (e) => handler(e.payload));

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
  /** Partecipanti attesi indicati all'avvio (nomi). */
  participants: string[];
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

export type PassageKind = "transcript" | "block" | "overview" | "keyPoints" | "actionItems";

/** Passaggio trovato dalla ricerca per significato, con il testo attuale. */
export interface SearchPassage {
  kind: PassageKind;
  /** Prima frase del passaggio (solo trascrizione). */
  segmentId: number | null;
  startMs: number;
  text: string;
  score: number;
}

export interface SearchResult extends MeetingSummary {
  score: number;
  /** Trovato anche dalla ricerca per parole. */
  lexical: boolean;
  passages: SearchPassage[];
}

/** Stato dell'indice della ricerca per significato. */
export interface IndexStatus {
  modelReady: boolean;
  enabled: boolean;
  indexed: number;
  total: number;
  pending: number;
  error: string | null;
}

/** Ricerca per significato + per parole (senza modello: solo parole). */
export const searchMeetings = (query: string) =>
  invoke<SearchResult[]>("search_meetings", { query });

export const semanticIndexStatus = () => invoke<IndexStatus>("semantic_index_status");

export const reindexMeetings = () => invoke<void>("reindex_meetings");

/** Aggiornamenti dello stato dell'indice (evento `search:status`). */
export const onIndexStatus = (cb: (status: IndexStatus) => void) =>
  listen<IndexStatus>("search:status", (e) => cb(e.payload));

export const getMeeting = (id: number) => invoke<MeetingDetails>("get_meeting", { id });

export const deleteMeeting = (id: number) => invoke<void>("delete_meeting", { id });

export const renameMeeting = (id: number, name: string) =>
  invoke<Meeting>("rename_meeting", { id, name });

export const generateSummary = (id: number, language?: Language) =>
  invoke<Summary>("generate_summary", { id, language: language ?? null });

/** Salva il meeting in Markdown nel file scelto dall'utente; `null` se annulla. */
export const exportMeetingMarkdown = (fileName: string, contents: string) =>
  invoke<string | null>("export_meeting_markdown", { fileName, contents });

/** Nome di un parlante di "Loro" in un meeting salvato (vuoto = "Speaker N"). */
export const setMeetingSpeaker = (id: number, speaker: number, name: string) =>
  invoke<SavedSpeaker[]>("set_meeting_speaker", { id, speaker, name });

/** Scheda della rubrica, legata ai parlanti per nome. */
export interface Contact {
  id: number;
  name: string;
  role: string;
  description: string;
  email: string;
  createdAtMs: number;
  updatedAtMs: number;
}

/** Senza `id` si crea, o si aggiorna il contatto con lo stesso nome. */
export interface ContactInput {
  id?: number | null;
  name: string;
  role: string;
  description: string;
  email: string;
}

export const listContacts = () => invoke<Contact[]>("list_contacts");

export const saveContact = (contact: ContactInput) => invoke<Contact>("save_contact", { contact });

export const deleteContact = (id: number) => invoke<void>("delete_contact", { id });

/** Riunione programmata: giorno, ora e contesto pronti per l'avvio. */
export interface ScheduledMeeting {
  id: number;
  name: string;
  goal: string;
  notes: string;
  codePath: string;
  participants: Participant[];
  /** Lingua della call. */
  language: Language;
  startsAtMs: number;
  durationMin: number;
  /** Calendario di provenienza; vuoto = creata a mano. */
  source: "" | CalendarProvider;
  /** Link della call (Meet, Teams, Zoom), se noto. */
  joinUrl: string;
  createdAtMs: number;
  updatedAtMs: number;
}

/** Senza `id` si crea, con `id` si aggiorna. */
export type ScheduledMeetingInput = Omit<
  ScheduledMeeting,
  "id" | "createdAtMs" | "updatedAtMs" | "source" | "joinUrl"
> & {
  id?: number | null;
};

/** Riunioni che finiscono dopo `sinceMs`, dalla più vicina. */
export const listScheduledMeetings = (sinceMs: number) =>
  invoke<ScheduledMeeting[]>("list_scheduled_meetings", { sinceMs });

export const saveScheduledMeeting = (meeting: ScheduledMeetingInput) =>
  invoke<ScheduledMeeting>("save_scheduled_meeting", { meeting });

export const deleteScheduledMeeting = (id: number) => invoke<void>("delete_scheduled_meeting", { id });

/** L'Agenda è cambiata (sincronizzazione del calendario). */
export const onScheduledChanged = (cb: () => void) => listen("scheduled:changed", () => cb());

export interface CalendarSyncReport {
  added: number;
  updated: number;
  removed: number;
}

/** Calendari che si possono collegare. */
export type CalendarProvider = "google" | "microsoft";

export const CALENDAR_PROVIDERS: CalendarProvider[] = ["google", "microsoft"];

/** Stato della sincronizzazione con un calendario. */
export interface CalendarStatus {
  /** ID client (e per Google client secret) presenti. */
  configured: boolean;
  connected: boolean;
  email: string;
  connecting: boolean;
  syncing: boolean;
  lastSyncMs: number | null;
  lastReport: CalendarSyncReport | null;
  lastError: string | null;
  /** Accesso revocato o scaduto: va ricollegato. */
  needsReconnect: boolean;
}

export const calendarStatus = (provider: CalendarProvider) =>
  invoke<CalendarStatus>("calendar_status", { provider });

/** Apre il consenso nel browser; si risolve al ritorno (o dopo 5 minuti). */
export const connectCalendar = (provider: CalendarProvider, uiLanguage: string) =>
  invoke<CalendarStatus>("connect_calendar", { provider, uiLanguage });

export const cancelCalendarConnect = (provider: CalendarProvider) =>
  invoke<void>("cancel_calendar_connect", { provider });

export const disconnectCalendar = (provider: CalendarProvider) =>
  invoke<CalendarStatus>("disconnect_calendar", { provider });

export const syncCalendar = (provider: CalendarProvider) => invoke<CalendarStatus>("sync_calendar", { provider });

export type ConsolePage =
  | "googleProject"
  | "googleCalendarApi"
  | "googleBranding"
  | "googleAudience"
  | "googleDataAccess"
  | "googleClients"
  | "microsoftAppRegistrations"
  | "microsoftNewRegistration"
  | "microsoftMyApps";

/** Apre una pagina della console del provider (guide di configurazione). */
export const openProviderConsole = (page: ConsolePage) => invoke<void>("open_provider_console", { page });

/** Apre il link della call (Meet, Teams, Zoom) nel browser. */
export const openMeetingLink = (url: string) => invoke<void>("open_meeting_link", { url });

/** Email composta dal frontend (`src/lib/email.ts`). */
export interface OutgoingEmail {
  to: string[];
  subject: string;
  html: string;
  text: string;
}

/** Invio SMTP; ritorna il numero di destinatari. */
export const sendEmail = (email: OutgoingEmail) => invoke<number>("send_email", { email });

/** Apre il messaggio come bozza nel client di posta predefinito. */
export const openEmailDraft = (email: OutgoingEmail) => invoke<void>("open_email_draft", { email });

/** Prova connessione e login SMTP senza inviare nulla. */
export const testSmtp = () => invoke<void>("test_smtp");

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

/** Modelli di `provider` (default: il provider principale). */
export const listLlmModels = (provider?: LlmProvider) =>
  invoke<ModelInfo[]>("list_llm_models", { provider });

export const testLlm = (model?: string, provider?: LlmProvider) =>
  invoke<LlmTestResult>("test_llm", { model, provider });

/** Traduce su richiesta una frase finale; `previous` = frasi precedenti (contesto). */
/** "Loro" → lingua dell'utente; "Io" → lingua del meeting. */
export const translateSegment = (segmentId: number, channel: ChannelId, text: string, previous: string[]) =>
  invoke<TranslationEvent>("translate_segment", { segmentId, channel, text, previous });

export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

// --- Aggiornamenti ----------------------------------------------------------

export interface UpdateInfo {
  /** Versione della release (`0.1.0`). */
  version: string;
  /** Sha corto della build disponibile. */
  build: string;
  notes: string;
  url: string;
  /** Data di pubblicazione (RFC 3339). */
  publishedAt: string | null;
}

/** Ultima release pubblicata più recente di questa build; `null` se non c'è nulla. */
export const checkUpdate = () => invoke<UpdateInfo | null>("check_update");

/** Apre nel browser la pagina della release. */
export const openReleasePage = (url: string) => invoke<void>("open_release_page", { url });

// --- Modelli locali ---------------------------------------------------------

export type LocalModelKind = "vad" | "stt" | "speakerEmbedding" | "textEmbedding";

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

/** Mostra o nasconde il widget overlay; `true` se ora è aperto. */
export const toggleWidget = () => invoke<boolean>("toggle_widget");

export const closeWidget = () => invoke<void>("close_widget");

/** Versione e build in esecuzione. */
export interface AppInfo {
  version: string;
  /** Sha corto del commit; `-dirty` = modifiche non committate. */
  commit: string;
  /** Ora del build (RFC 3339 UTC). */
  builtAt: string;
  debug: boolean;
}

export const appInfo = () => invoke<AppInfo>("app_info");
