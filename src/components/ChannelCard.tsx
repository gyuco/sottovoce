import { Mic, Volume2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { LevelMeter } from "@/components/LevelMeter";
import { Badge } from "@/components/ui/badge";
import type { ChannelId } from "@/lib/ipc";
import { FLOOR_DB, useAudioStore } from "@/stores/audio";
import { useSettingsStore } from "@/stores/settings";
import { type ConnState, useTranscriptStore } from "@/stores/transcript";

/** Dopo quanto tempo senza eventi il canale è considerato "senza segnale". */
const NO_SIGNAL_MS = 1500;

const CONN_VARIANT: Record<ConnState, "default" | "secondary" | "destructive" | "outline"> = {
  idle: "secondary",
  connected: "default",
  reconnecting: "outline",
  error: "destructive",
  closed: "secondary",
};

function useNow(enabled: boolean, intervalMs = 250) {
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    if (!enabled) return;
    const id = setInterval(() => setNow(performance.now()), intervalMs);
    return () => clearInterval(id);
  }, [enabled, intervalMs]);
  return now;
}

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
    <div className="flex flex-col gap-2 rounded-xl border bg-card p-3">
      <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
        <Icon className="size-4 text-muted-foreground" aria-hidden />
        {t(`channel.${channel}`)}
        <span className="text-xs font-normal text-muted-foreground">{t(`channel.${channel}Hint`)}</span>
        <span className="ml-auto flex gap-1">
          <Badge variant={live ? "default" : noSignal ? "destructive" : "secondary"} className="font-normal">
            {audioLabel}
          </Badge>
          {running && (
            <Badge variant={CONN_VARIANT[conn]} className="font-normal" title={connError ?? undefined}>
              {t(`stt.${conn}`)}
            </Badge>
          )}
        </span>
      </div>
      <div className="flex items-end gap-3">
        <div className="flex-1">
          <LevelMeter rmsDb={level.rmsDb} holdDb={level.holdDb} active={live} label={t(`channel.${channel}`)} />
        </div>
        <span className="w-14 pb-4 text-right font-mono text-xs tabular-nums">
          {live && level.rmsDb > FLOOR_DB ? `${level.rmsDb.toFixed(1)}` : "—"}
          <span className="ml-0.5 text-muted-foreground">dB</span>
        </span>
      </div>
      {noSignal && <p className="text-xs text-muted-foreground">{t("channel.noSignalHint")}</p>}
      {conn === "error" && connError && <p className="text-xs text-destructive">{connError}</p>}
    </div>
  );
}
