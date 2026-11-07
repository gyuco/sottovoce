import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./en.json";
import it from "./it.json";

void i18n.use(initReactI18next).init({
  resources: { it: { translation: it }, en: { translation: en } },
  lng: "it",
  fallbackLng: "it",
  interpolation: { escapeValue: false },
});

export default i18n;
