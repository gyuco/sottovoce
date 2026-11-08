import { effectiveEngine, requiredLocalModels } from "@/lib/privacy";
import { useModelsStore } from "@/stores/models";
import { useSettingsStore } from "@/stores/settings";

/** Cosa manca per avviare la trascrizione; `null` = pronto (o non ancora noto). */
export type SttBlocker = "needKey" | "needModels" | null;

export function useSttBlocker(): SttBlocker {
  const settings = useSettingsStore((s) => s.settings);
  const hasKey = useSettingsStore((s) => s.hasKey);
  const models = useModelsStore((s) => s.models);
  if (!settings) return null;
  if (effectiveEngine(settings) === "deepgram") return hasKey ? null : "needKey";
  // Catalogo non ancora caricato: decide il backend all'avvio.
  if (models.length === 0) return null;
  const ready = requiredLocalModels(settings).every(
    (id) => models.find((m) => m.id === id)?.status.state === "ready",
  );
  return ready ? null : "needModels";
}
