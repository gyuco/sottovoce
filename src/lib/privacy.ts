import { CHUNKED_ENGINES, type Settings, type SttEngine } from "@/lib/ipc";

/** Motore effettivo: "solo locale" forza il motore locale (come il backend). */
export function effectiveEngine(s: Settings): SttEngine {
  return s.localOnly ? "local" : s.sttEngine;
}

/** Traduzione o suggerimenti attivi (un modello scelto). */
export function llmUsed(s: Settings): boolean {
  return s.translationModel.trim() !== "" || s.assistModel.trim() !== "";
}

/** Il provider LLM gira su questa macchina (stessa regola di `stt::is_local_llm`). */
export function isLocalLlm(s: Settings): boolean {
  if (s.llmProvider === "ollama") return true;
  if (s.llmProvider !== "custom") return false;
  let url: URL;
  try {
    url = new URL(s.llmBaseUrl.trim());
  } catch {
    return false;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return false;
  const host = url.hostname.toLowerCase().replace(/\.$/, "");
  return (
    host === "localhost" ||
    host.endsWith(".localhost") ||
    host === "[::1]" ||
    /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)
  );
}

/** "Solo locale" attivo ma il modello linguistico uscirebbe dalla macchina. */
export function localOnlyViolation(s: Settings): boolean {
  return s.localOnly && llmUsed(s) && !isLocalLlm(s);
}

/** Nessun dato esce dalla macchina durante la sessione. */
export function isLocalSession(s: Settings): boolean {
  return effectiveEngine(s) === "local" && (!llmUsed(s) || isLocalLlm(s));
}

/** Motore cloud che taglia le frasi col VAD locale prima di inviarle. */
export function isChunkedEngine(e: SttEngine): boolean {
  return (CHUNKED_ENGINES as readonly SttEngine[]).includes(e);
}

/**
 * Id dei modelli locali che servono al motore: VAD + modello STT per il
 * locale, solo il VAD per i motori cloud a blocchi, nessuno per lo streaming.
 */
export function requiredLocalModels(s: Settings): string[] {
  const engine = effectiveEngine(s);
  if (engine === "local") return ["silero-vad", s.localSttModel];
  return isChunkedEngine(engine) ? ["silero-vad"] : [];
}
