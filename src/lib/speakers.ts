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
