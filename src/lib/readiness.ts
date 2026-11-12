import { sttKeyAccount } from "@/lib/ipc";
import { effectiveEngine, requiredLocalModels } from "@/lib/privacy";
import { useAudioStore, useSelectedMicId } from "@/stores/audio";
import { useModelsStore } from "@/stores/models";
import { useSettingsStore } from "@/stores/settings";

/** Cosa manca per avviare la trascrizione; `null` = pronto (o non ancora noto). */
export type SttBlocker = "needKey" | "needModels" | null;

export function useSttBlocker(): SttBlocker {
  const settings = useSettingsStore((s) => s.settings);
  const keys = useSettingsStore((s) => s.keys);
  const models = useModelsStore((s) => s.models);
  if (!settings) return null;
  const account = sttKeyAccount(effectiveEngine(settings));
  if (account && !keys[account]) return "needKey";
  const required = requiredLocalModels(settings);
  // Catalogo non ancora caricato: decide il backend all'avvio.
  if (required.length === 0 || models.length === 0) return null;
  const ready = required.every(
    (id) => models.find((m) => m.id === id)?.status.state === "ready",
  );
  return ready ? null : "needModels";
}

/** Si può avviare una call adesso (nessuna sessione, microfono e STT pronti). */
export function useCanStart(): boolean {
  const idle = useAudioStore((s) => s.status === "idle");
  const micId = useSelectedMicId();
  const blocked = useSttBlocker() !== null;
  return idle && !!micId && !blocked;
}
