import { ArrowDown, Check, Copy, ScrollText } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { formatDuration, useCopy } from "@/lib/hooks";
import type { TranslationEvent } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { type Line, useTranscriptStore } from "@/stores/transcript";

/** Distanza dal fondo (px) entro cui l'autoscroll resta agganciato. */
const STICK_PX = 48;

export function Transcript() {
  const { t } = useTranslation();
  const lines = useTranscriptStore((s) => s.lines);
  const partial = useTranscriptStore((s) => s.partial);
  const translations = useTranscriptStore((s) => s.translations);
  const showTranslations = useTranscriptStore((s) => s.showTranslations);
  const setShowTranslations = useTranscriptStore((s) => s.setShowTranslations);
  const { copied, copy } = useCopy();

  const rows = useMemo(() => {
    const live = [partial.me, partial.them].filter((p): p is Line => p !== null);
    return [...lines, ...live].sort((a, b) => a.start - b.start);
  }, [lines, partial]);

  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [detached, setDetached] = useState(false);

  const onScroll = () => {
    const el = box.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX;
    stick.current = atBottom;
    setDetached(!atBottom);
  };

  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [rows, translations, showTranslations]);

  const toBottom = () => {
    const el = box.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    stick.current = true;
    setDetached(false);
  };

  const copyAll = () => {
    const text = lines
      .map((l) => {
        const head = `[${formatDuration(l.start)}] ${t(`channel.${l.channel}`)}: ${l.text}`;
        const tr = showTranslations ? translations[l.id]?.text : undefined;
        return tr ? `${head}\n    ${tr}` : head;
      })
      .join("\n");
    copy(text);
  };

  return (
    <section className="relative flex min-h-80 flex-1 flex-col rounded-2xl border bg-card shadow-xs md:min-h-0">
      <header className="flex items-center gap-2 px-4 pt-3.5 pb-2.5">
        <ScrollText className="size-4 text-primary" aria-hidden />
        <h2 className="text-sm font-semibold">{t("transcript.title")}</h2>
        <label className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
          {t("transcript.showTranslations")}
          <Switch size="sm" checked={showTranslations} onCheckedChange={setShowTranslations} />
        </label>
        <Button
          variant="ghost"
          size="xs"
          disabled={lines.length === 0}
          onClick={copyAll}
          title={t("transcript.copyAll")}
        >
          {copied ? <Check className="text-emerald-600" /> : <Copy />}
          {copied ? t("common.copied") : t("transcript.copy")}
        </Button>
      </header>
      <div
        ref={box}
        onScroll={onScroll}
        className="min-h-0 flex-1 overflow-y-auto border-t px-2 py-2"
        aria-live="polite"
        aria-label={t("transcript.title")}
      >
        {rows.length === 0 ? (
          <p className="py-10 text-center text-sm text-muted-foreground">{t("transcript.empty")}</p>
        ) : (
          <ol className="flex flex-col">
            {rows.map((r) => (
              <TranscriptRow
                key={r.id === 0 ? `partial-${r.channel}` : r.id}
                line={r}
                translation={showTranslations && r.id !== 0 ? translations[r.id] : undefined}
              />
            ))}
          </ol>
        )}
      </div>
      {detached && (
        <Button
          size="sm"
          variant="secondary"
          className="absolute bottom-3 left-1/2 -translate-x-1/2 shadow-md"
          onClick={toBottom}
        >
          <ArrowDown /> {t("transcript.toBottom")}
        </Button>
      )}
    </section>
  );
}

function TranscriptRow({ line, translation }: { line: Line; translation: TranslationEvent | undefined }) {
  const { t } = useTranslation();
  const { copied, copy } = useCopy();
  const partial = line.id === 0;

  return (
    <li className="group grid grid-cols-[2.75rem_3.25rem_1fr_1.5rem] items-baseline gap-2 rounded-lg px-2 py-1.5 text-sm hover:bg-muted/60">
      <span className="font-mono text-xs text-muted-foreground tabular-nums">{formatDuration(line.start)}</span>
      <span
        className={cn(
          "justify-self-start rounded-md px-1.5 py-px text-xs font-medium",
          line.channel === "me" ? "bg-me/12 text-me" : "bg-them/12 text-them",
        )}
      >
        {t(`channel.${line.channel}`)}
      </span>
      <span className={cn("leading-relaxed select-text", partial && "text-muted-foreground italic")}>
        {line.text}
        {translation?.text && (
          <span className="mt-0.5 block text-muted-foreground">{translation.text}</span>
        )}
      </span>
      {!partial && (
        <Button
          variant="ghost"
          size="icon-xs"
          className="self-start opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
          aria-label={t("transcript.copyLine")}
          title={copied ? t("common.copied") : t("transcript.copyLine")}
          onClick={() => copy(translation?.text ? `${line.text}\n${translation.text}` : line.text)}
        >
          {copied ? <Check className="text-emerald-600" /> : <Copy />}
        </Button>
      )}
    </li>
  );
}
