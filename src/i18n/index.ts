import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./en.json";
import es from "./es.json";
import it from "./it.json";

/**
 * Lingue dell'interfaccia. `label` è l'endonimo: resta uguale in ogni lingua,
 * così l'utente riconosce sempre la propria voce nel selettore.
 */
export const UI_LANGUAGES = [
  { code: "it", label: "Italiano" },
  { code: "en", label: "English" },
  { code: "es", label: "Español" },
] as const;

export type UiLanguage = (typeof UI_LANGUAGES)[number]["code"];

const STORAGE_KEY = "sottovoce.uiLanguage";

function isUiLanguage(value: string): value is UiLanguage {
  return UI_LANGUAGES.some((language) => language.code === value);
}

/** Codice lingua a due lettere (`it-IT` → `it`). */
function baseLanguage(value: string): string {
  const normalized = value.toLowerCase();
  return normalized.split("-")[0] ?? normalized;
}

/**
 * Lingua iniziale: scelta salvata → lingua di sistema → italiano.
 * La scelta esplicita dell'utente ha sempre la precedenza.
 */
function initialLanguage(): UiLanguage {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved && isUiLanguage(saved)) return saved;

  const systemLanguages =
    typeof navigator === "undefined"
      ? []
      : navigator.languages?.length
        ? navigator.languages
        : [navigator.language];

  for (const language of systemLanguages) {
    const code = baseLanguage(language);
    if (isUiLanguage(code)) return code;
  }

  return "it";
}

function syncDocumentLanguage(language: string) {
  document.documentElement.lang = baseLanguage(language);
}

const language = initialLanguage();

void i18n.use(initReactI18next).init({
  resources: {
    it: { translation: it },
    en: { translation: en },
    es: { translation: es },
  },
  lng: language,
  fallbackLng: "it",
  supportedLngs: UI_LANGUAGES.map((item) => item.code),
  // `en-US` deve ricadere su `en`, non su una risorsa inesistente.
  load: "languageOnly",
  nonExplicitSupportedLngs: true,
  interpolation: { escapeValue: false },
  returnNull: false,
});

i18n.on("languageChanged", (next) => {
  const code = baseLanguage(next);
  if (isUiLanguage(code)) localStorage.setItem(STORAGE_KEY, code);
  syncDocumentLanguage(code);
});

syncDocumentLanguage(language);

/** Cambia la lingua dell'interfaccia e la salva per i prossimi avvii. */
export async function setUiLanguage(next: UiLanguage): Promise<void> {
  await i18n.changeLanguage(next);
}

export default i18n;
