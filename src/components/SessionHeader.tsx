import { Clock, Cloud, Loader2, PictureInPicture2, Play, ShieldCheck, Square } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { formatDuration, useNow } from "@/lib/hooks";
import { toggleWidget } from "@/lib/ipc";
import { isLocalSession } from "@/lib/privacy";
import { useSttBlocker } from "@/lib/readiness";
import { cn } from "@/lib/utils";
import { useAudioStore, useSelectedMicId } from "@/stores/audio";
import { useMeetingStore } from "@/stores/meeting";
import { useSettingsStore } from "@/stores/settings";

/** Nome del meeting, ora di inizio, cronometro e comando avvia/ferma. */
export function SessionHeader() {
  const { t, i18n } = useTranslation();
  const { status, startedAt, startedWall, start, stop } = useAudioStore();
  const settings = useSettingsStore((s) => s.settings);
  const blocked = useSttBlocker() !== null;
  const name = useMeetingStore((s) => s.context.name.trim());
  const micId = useSelectedMicId();

  const busy = status === "starting" || status === "stopping";
  const running = status === "running";
  const now = useNow(running, 1000);

  const since = new Date(startedWall).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });

  return (
    <header className="flex flex-wrap items-center gap-x-5 gap-y-3 px-6 pt-5 pb-4">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2.5">
          <h1 className="truncate text-2xl font-semibold tracking-tight">{name || t("header.untitled")}</h1>
          {settings && <ModeBadge local={isLocalSession(settings)} />}
        </div>
        <p className="mt-1 flex items-center gap-1.5 text-sm text-muted-foreground">
          <Clock className="size-3.5" aria-hidden />
          {running ? t("header.startedAt", { time: since }) : t("header.ready")}
        </p>
      </div>

      {running && (
        <div className="flex items-center gap-2.5" role="timer" aria-label={t("header.elapsed")}>
          <span className="relative flex size-2.5">
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-destructive/60" />
            <span className="relative inline-flex size-2.5 rounded-full bg-destructive" />
          </span>
          <span className="font-mono text-2xl font-medium tabular-nums">{formatDuration(now - startedAt)}</span>
        </div>
      )}

      <Button
        variant="outline"
        size="icon-lg"
        className="rounded-xl"
        aria-label={t("widget.toggle")}
        title={t("widget.toggleHint")}
        onClick={() => void toggleWidget()}
      >
        <PictureInPicture2 />
      </Button>

      {running || status === "stopping" ? (
        <Button
          variant="destructive"
          size="lg"
          className="rounded-xl shadow-sm"
          onClick={() => void stop()}
          disabled={busy}
        >
          {busy ? <Loader2 className="animate-spin" /> : <Square className="fill-current" />}
          {t("controls.stop")}
        </Button>
      ) : (
        <Button
          size="lg"
          className="rounded-xl shadow-sm"
          onClick={() => void start()}
          disabled={busy || !micId || blocked}
        >
          {busy ? <Loader2 className="animate-spin" /> : <Play className="fill-current" />}
          {busy ? t("controls.starting") : t("controls.start")}
        </Button>
      )}
    </header>
  );
}

/** "Locale": nessun dato esce dalla macchina; "Cloud": audio o testo a terzi. */
function ModeBadge({ local }: { local: boolean }) {
  const { t } = useTranslation();
  const Icon = local ? ShieldCheck : Cloud;
  const hint = t(local ? "header.mode.localHint" : "header.mode.cloudHint");
  return (
    <Badge
      variant="secondary"
      title={hint}
      aria-label={`${t(local ? "header.mode.local" : "header.mode.cloud")}: ${hint}`}
      className={cn(
        "shrink-0 gap-1 font-normal",
        local && "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
      )}
    >
      <Icon className="size-3" aria-hidden />
      {t(local ? "header.mode.local" : "header.mode.cloud")}
    </Badge>
  );
}
