import type { Language } from "@/lib/ipc";

/** Lingue in cui esiste il messaggio di consenso da mandare ai partecipanti. */
export type ConsentLanguage = "it" | "en" | "es" | "fr" | "de";

export const CONSENT_LANGUAGES: ConsentLanguage[] = ["it", "en", "es", "fr", "de"];

/**
 * Messaggio da leggere o incollare nella chat della call per chiedere il
 * consenso alla trascrizione. Non è testo dell'interfaccia: va nella lingua
 * della call, quindi copre tutte le lingue della call (anche fr/de).
 * `local` = nessun dato esce dalla macchina; altrimenti trascrizione o
 * modello linguistico passano da servizi esterni e il messaggio lo dice.
 */
const MESSAGES: Record<ConsentLanguage, { local: string; cloud: string }> = {
  it: {
    local:
      "Prima di iniziare: per prendere appunti uso un assistente che trascrive questa chiamata. Non registro né l'audio né il video e la trascrizione resta solo sul mio computer. Siete d'accordo?",
    cloud:
      "Prima di iniziare: per prendere appunti uso un assistente che trascrive questa chiamata. Non registro né l'audio né il video; la trascrizione è conservata solo sul mio computer, ma per elaborarla passa da servizi esterni di trascrizione e intelligenza artificiale. Siete d'accordo?",
  },
  en: {
    local:
      "Before we start: I use an assistant that transcribes this call so I can take notes. I'm not recording audio or video, and the transcript stays only on my computer. Is that OK with you?",
    cloud:
      "Before we start: I use an assistant that transcribes this call so I can take notes. I'm not recording audio or video; the transcript is stored only on my computer, but it is processed by external transcription and AI services. Is that OK with you?",
  },
  es: {
    local:
      "Antes de empezar: para tomar notas uso un asistente que transcribe esta llamada. No grabo ni el audio ni el vídeo y la transcripción se queda solo en mi ordenador. ¿Estáis de acuerdo?",
    cloud:
      "Antes de empezar: para tomar notas uso un asistente que transcribe esta llamada. No grabo ni el audio ni el vídeo; la transcripción se guarda solo en mi ordenador, pero se procesa con servicios externos de transcripción e inteligencia artificial. ¿Estáis de acuerdo?",
  },
  fr: {
    local:
      "Avant de commencer : pour prendre des notes, j'utilise un assistant qui transcrit cet appel. Je n'enregistre ni l'audio ni la vidéo, et la transcription reste uniquement sur mon ordinateur. Êtes-vous d'accord ?",
    cloud:
      "Avant de commencer : pour prendre des notes, j'utilise un assistant qui transcrit cet appel. Je n'enregistre ni l'audio ni la vidéo ; la transcription est conservée uniquement sur mon ordinateur, mais elle est traitée par des services externes de transcription et d'intelligence artificielle. Êtes-vous d'accord ?",
  },
  de: {
    local:
      "Bevor wir anfangen: Ich nutze einen Assistenten, der dieses Gespräch für meine Notizen transkribiert. Ich zeichne weder Audio noch Video auf, und das Transkript bleibt nur auf meinem Computer. Sind Sie damit einverstanden?",
    cloud:
      "Bevor wir anfangen: Ich nutze einen Assistenten, der dieses Gespräch für meine Notizen transkribiert. Ich zeichne weder Audio noch Video auf; das Transkript wird nur auf meinem Computer gespeichert, aber von externen Transkriptions- und KI-Diensten verarbeitet. Sind Sie damit einverstanden?",
  },
};

function isConsentLanguage(value: string): value is ConsentLanguage {
  return (CONSENT_LANGUAGES as string[]).includes(value);
}

/** Lingua del messaggio: quella della call se fissa, altrimenti quella dell'interfaccia. */
export function consentLanguage(call: Language, uiLanguage: string): ConsentLanguage {
  if (call !== "multi") return call;
  const ui = uiLanguage.toLowerCase().split("-")[0] ?? "";
  return isConsentLanguage(ui) ? ui : "it";
}

export function consentMessage(language: ConsentLanguage, local: boolean): string {
  return MESSAGES[language][local ? "local" : "cloud"];
}
