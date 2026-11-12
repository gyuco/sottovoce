/** Sezioni delle Impostazioni. */
export const SETTINGS_TABS = ["general", "audio", "ai", "models", "integrations"] as const;

export type SettingsTab = (typeof SETTINGS_TABS)[number];

const STORAGE_KEY = "sottovoce.settingsTab";

/** Ultima sezione aperta (preferenza locale, se il browser la conserva). */
export function savedSettingsTab(): SettingsTab {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    return SETTINGS_TABS.find((tab) => tab === saved) ?? "general";
  } catch {
    return "general";
  }
}

export function saveSettingsTab(tab: SettingsTab) {
  try {
    localStorage.setItem(STORAGE_KEY, tab);
  } catch {
    // Nessun salvataggio: si riparte da "Generale".
  }
}
