import { AppWindow, Check, CircleHelp, MessageCircle, Square, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { NudgeCard } from "@/components/NudgeCard";
import { Button } from "@/components/ui/button";
import { formatDuration } from "@/lib/hooks";
import { closeWidget } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { type WidgetSnapshot, listenWidgetState, sendWidgetAction } from "@/lib/widget";

/**
 * Widget overlay sempre in primo piano: ultime frasi di "Loro" con la
 * traduzione, suggerimenti, nudge, cronometro e stop. Lo stato arriva dalla
 * finestra principale.
 */
export function Widget() {
  const { t, i18n } = useTranslation();
  const [state, setState] = useState<WidgetSnapshot | null>(null);

  useEffect(() => listenWidgetState(setState), []);

  // Stessa lingua della finestra principale, anche se cambiata dopo.
  useEffect(() => {
    if (state && state.language !== i18n.resolvedLanguage) void i18n.changeLanguage(state.language);
  }, [state, i18n]);

  const running = state?.status === "running";
  const elapsed = useElapsed(running ? state.startedWall : null);

  return (
    <div className="flex h-screen flex-col overflow-hidden rounded-2xl border bg-background/95 text-foreground shadow-lg">
      <header data-tauri-drag-region className="flex shrink-0 cursor-grab items-center gap-2 border-b px-3 py-2">
        {running ? (
          <span data-tauri-drag-region className="flex items-center gap-2" role="timer" aria-label={t("header.elapsed")}>
            <span className="relative flex size-2">
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-destructive/60" />
              <span className="relative inline-flex size-2 rounded-full bg-destructive" />
            </span>
            <span data-tauri-drag-region className="font-mono text-sm font-medium tabular-nums">
              {formatDuration(elapsed)}
            </span>
          </span>
        ) : null}
        <span data-tauri-drag-region className="min-w-0 flex-1 truncate text-sm font-medium">
          {state?.meetingName || t("header.untitled")}
        </span>
        {running && (
          <Button
            variant="destructive"
            size="xs"
            onClick={() => sendWidgetAction({ type: "stop" })}
            aria-label={t("controls.stop")}
            title={t("controls.stop")}
          >
            <Square className="fill-current" />
            {t("controls.stop")}
          </Button>
        )}
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={t("widget.openMain")}
          title={t("widget.openMain")}
          onClick={() => sendWidgetAction({ type: "openMain" })}
        >
          <AppWindow />
        </Button>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={t("widget.close")}
          title={t("widget.close")}
          onClick={() => void closeWidget()}
        >
          <X />
        </Button>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3" aria-live="polite">
        {!state || state.status === "idle" ? (
          <p className="m-auto max-w-60 text-center text-sm text-muted-foreground">{t("widget.idle")}</p>
        ) : (
          <>
            {state.nudge && (
              <NudgeCard nudge={state.nudge} onDismiss={() => sendWidgetAction({ type: "dismissNudge" })} />
            )}
            <WidgetLines state={state} />
            <WidgetAssists state={state} />
          </>
        )}
      </div>
    </div>
  );
}

function WidgetLines({ state }: { state: WidgetSnapshot }) {
  const { t } = useTranslation();
  if (state.lines.length === 0) {
    return <p className="text-xs text-muted-foreground">{t("widget.noLines")}</p>;
  }
  return (
    <ul className="flex flex-col gap-2.5">
      {state.lines.map((line) => (
        <li key={line.id} className="flex flex-col gap-0.5 text-sm">
          <span className={cn("w-fit rounded-md px-1.5 py-px text-[11px] font-medium", line.color)}>
            {line.speaker}
          </span>
          {line.translation ? (
            <>
              <p className="leading-snug select-text">{line.translation}</p>
              <p className="text-xs leading-snug text-muted-foreground select-text">{line.text}</p>
            </>
          ) : (
            <p className="leading-snug select-text">{line.text}</p>
          )}
        </li>
      ))}
    </ul>
  );
}

function WidgetAssists({ state }: { state: WidgetSnapshot }) {
  const { t } = useTranslation();
  if (state.assists.length === 0) return null;
  return (
    <section className="flex flex-col gap-1.5 border-t pt-3" aria-label={t("assist.title")}>
      {state.assists.map((item) => {
        const Icon = item.kind === "say" ? MessageCircle : CircleHelp;
        return (
          <div
            key={item.id}
            className={cn(
              "group flex items-start gap-2 rounded-lg border px-2.5 py-2 text-sm",
              item.kind === "say" ? "border-say/20 bg-say/6" : "border-ask/20 bg-ask/6",
            )}
          >
            <Icon
              className={cn("mt-0.5 size-3.5 shrink-0", item.kind === "say" ? "text-say" : "text-ask")}
              aria-label={t(`assist.${item.kind}Title`)}
            />
            <p className="min-w-0 flex-1 leading-snug select-text">{item.text}</p>
            <Button
              variant="ghost"
              size="icon-xs"
              className="-my-0.5 shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
              aria-label={t("assist.markUsed")}
              title={t("assist.markUsed")}
              onClick={() => sendWidgetAction({ type: "assistUsed", id: item.id })}
            >
              <Check />
            </Button>
          </div>
        );
      })}
    </section>
  );
}

/** Ms trascorsi da `since` (epoch ms), aggiornati ogni secondo. */
function useElapsed(since: number | null) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (since === null) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [since]);
  return since === null ? 0 : now - since;
}
