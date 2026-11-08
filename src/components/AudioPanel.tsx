import { AudioLines } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ChannelCard } from "@/components/ChannelCard";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useAudioStore, useSelectedMicId } from "@/stores/audio";

/** Scelta del microfono e livelli dei due canali. */
export function AudioPanel() {
  const { t } = useTranslation();
  const devices = useAudioStore((s) => s.devices);
  const status = useAudioStore((s) => s.status);
  const setMic = useAudioStore((s) => s.setMic);
  const micId = useSelectedMicId();
  const busy = status === "starting" || status === "stopping";

  return (
    <section className="flex flex-col gap-3 rounded-2xl border bg-card p-4 shadow-xs" aria-label={t("audio.title")}>
      <div className="flex items-center gap-2">
        <AudioLines className="size-4 text-muted-foreground" aria-hidden />
        <h2 className="text-sm font-semibold">{t("audio.title")}</h2>
        <Select value={micId} onValueChange={(id) => void setMic(id)} disabled={busy}>
          <SelectTrigger size="sm" className="ml-auto max-w-60 min-w-0" aria-label={t("controls.microphone")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {devices.map((d) => (
              <SelectItem key={d.id} value={d.id}>
                {d.name}
                {d.isDefault && <span className="text-muted-foreground"> · {t("controls.defaultDevice")}</span>}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <ChannelCard channel="me" />
      <ChannelCard channel="them" />
      <p className="text-xs text-muted-foreground">{t("permissions.hint")}</p>
    </section>
  );
}
