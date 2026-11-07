import { useTranslation } from "react-i18next";

import type { ChannelId } from "@/lib/ipc";
import { useMeetingStore } from "@/stores/meeting";

/** Nome da mostrare: "Io", "Loro", il nome dato dall'utente o "Speaker N". */
export function useSpeakerName() {
  const { t } = useTranslation();
  const names = useMeetingStore((s) => s.context.speakers);
  return (channel: ChannelId, speaker: number | null) => {
    if (channel === "me" || speaker === null) return t(`channel.${channel}`);
    return names[speaker] ?? t("speakers.label", { n: speaker + 1 });
  };
}

/** Colori dei parlanti di "Loro", per indice (si ripetono oltre l'ottavo). */
export const SPEAKER_COLORS = [
  "bg-them/12 text-them",
  "bg-amber-500/14 text-amber-700 dark:text-amber-400",
  "bg-violet-500/12 text-violet-700 dark:text-violet-400",
  "bg-rose-500/12 text-rose-700 dark:text-rose-400",
  "bg-teal-500/12 text-teal-700 dark:text-teal-400",
  "bg-lime-500/14 text-lime-700 dark:text-lime-400",
  "bg-fuchsia-500/12 text-fuchsia-700 dark:text-fuchsia-400",
  "bg-orange-500/14 text-orange-700 dark:text-orange-400",
];

/** Classe colore dell'etichetta di un parlante. */
export function speakerColor(channel: ChannelId, speaker: number | null) {
  if (channel === "me") return "bg-me/12 text-me";
  return speaker === null
    ? "bg-them/12 text-them"
    : SPEAKER_COLORS[speaker % SPEAKER_COLORS.length];
}
