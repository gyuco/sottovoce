import { Mic, Volume2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { LevelMeter } from "@/components/LevelMeter";
import { Badge } from "@/components/ui/badge";
import type { ChannelId } from "@/lib/ipc";
import { useNow } from "@/lib/hooks";
import { cn } from "@/lib/utils";
import { useAudioStore } from "@/stores/audio";
import { useSettingsStore } from "@/stores/settings";
import { type ConnState, useTranscriptStore } from "@/stores/transcript";

/** Dopo quanto tempo senza eventi il canale è considerato "senza segnale". */
const NO_SIGNAL_MS = 1500;

const CONN_VARIANT: Record<ConnState, "default" | "secondary" | "destructive" | "outline"> = {
  idle: "secondary",
  connected: "secondary",
  reconnecting: "outline",
  error: "destructive",
  closed: "secondary",
};

/** Riga compatta di un canale: nome, stato audio/STT e livello. */
export function ChannelCard({ channel }: { channel: ChannelId }) {
  const { t } = useTranslation();
  const level = useAudioStore((s) => s.levels[channel]);
  const status = useAudioStore((s) => s.status);
  const startedAt = useAudioStore((s) => s.startedAt);
  const supported = useAudioStore((s) => channel === "me" || s.systemSupported);
  const enabled = useSettingsStore((s) => channel === "me" || (s.settings?.systemAudio ?? true));
  const conn = useTranscriptStore((s) => s.conn[channel]);
  const connError = useTranscriptStore((s) => s.connError[channel]);

  const running = status === "running" && supported && enabled;
  const now = useNow(running);
  const since = level.lastAt || startedAt;
  const noSignal = running && now - since > NO_SIGNAL_MS;
  const live = running && !noSignal;

  const Icon = channel === "me" ? Mic : Volume2;
  const audioLabel = !supported
    ? t("channel.unsupported")
    : !enabled
      ? t("channel.disabled")
      : noSignal
        ? t("channel.noSignal")
        : live
          ? t("channel.live")
          : t("channel.idle");

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span
          className={cn(
            "flex size-6 items-center justify-center rounded-md",
            channel === "me" ? "bg-me/12 text-me" : "bg-them/12 text-them",
          )}
        >
          <Icon className="size-3.5" aria-hidden />
        </span>
        <span className="font-medium">{t(`channel.${channel}`)}</span>
        <span className="text-xs text-muted-foreground">{t(`channel.${channel}Hint`)}</span>
        <span className="ml-auto flex gap-1">
          <Badge
            variant={noSignal ? "destructive" : "secondary"}
            className={cn("font-normal", live && "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400")}
          >
            {audioLabel}
          </Badge>
          {running && (
            <Badge variant={CONN_VARIANT[conn]} className="font-normal" title={connError ?? undefined}>
              {t(`stt.${conn}`)}
            </Badge>
          )}
        </span>
      </div>
      <LevelMeter
        compact
        rmsDb={level.rmsDb}
        holdDb={level.holdDb}
        active={live}
        label={t(`channel.${channel}`)}
      />
      {noSignal && <p className="text-xs text-muted-foreground">{t("channel.noSignalHint")}</p>}
      {conn === "error" && connError && <p className="text-xs text-destructive">{connError}</p>}
    </div>
  );
}
