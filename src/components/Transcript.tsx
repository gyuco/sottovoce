import { AlertCircle, ArrowDown, Check, Copy, Languages, Loader2, ScrollText } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { SpeakerLabel } from "@/components/SpeakerLabel";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { formatDuration, useCopy } from "@/lib/hooks";
import { MOMENT_COLORS } from "@/lib/moments";
import { type TranslationEvent, errorMessage, translateSegment } from "@/lib/ipc";
import { useSpeakerName } from "@/lib/speakers";
import { cn } from "@/lib/utils";
import { useInsightsStore } from "@/stores/insights";
import { type Line, useTranscriptStore } from "@/stores/transcript";

/** Distanza dal fondo (px) entro cui l'autoscroll resta agganciato. */
const STICK_PX = 48;
/** Frasi precedenti passate come contesto alla traduzione su richiesta. */
const TRANSLATE_HISTORY = 3;

export function Transcript() {
  const { t } = useTranslation();
  const lines = useTranscriptStore((s) => s.lines);
  const partial = useTranscriptStore((s) => s.partial);
  const translations = useTranscriptStore((s) => s.translations);
  const showTranslations = useTranscriptStore((s) => s.showTranslations);
  const setShowTranslations = useTranscriptStore((s) => s.setShowTranslations);
  const applyTranslation = useTranscriptStore((s) => s.applyTranslation);
  const { copied, copy } = useCopy();
  const speakerName = useSpeakerName();
  // Il selettore deve restituire un riferimento stabile: derivare fuori dallo store.
  const blocks = useInsightsStore((s) => s.blocks);
  const moments = useMemo(() => blocks.flatMap((b) => b.moments), [blocks]);

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

  /** Traduce su richiesta; lancia l'errore al chiamante (mostrato sulla riga). */
  const translate = async (line: Line) => {
    const previous = lines
      .filter((l) => l.start < line.start)
      .slice(-TRANSLATE_HISTORY)
      .map((l) => `${speakerName(l.channel, l.speaker)}: ${l.text}`);
    applyTranslation(await translateSegment(line.id, line.channel, line.text, previous));
    setShowTranslations(true);
  };

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
        const head = `[${formatDuration(l.start)}] ${speakerName(l.channel, l.speaker)}: ${l.text}`;
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
      <MomentBar moments={moments} end={rows.reduce((max, r) => Math.max(max, r.end), 0)} />
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
                onTranslate={
                  r.id !== 0 && !translations[r.id]
                    ? () => translate(r)
                    : undefined
                }
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

/** Marcatori dei momenti importanti sulla linea del tempo della trascrizione. */
function MomentBar(props: {
  moments: { timeMs: number; kind: string; text: string }[];
  end: number;
}) {
  const { t } = useTranslation();
  if (props.moments.length === 0) return null;
  const total = Math.max(props.end, 1);
  return (
    <div
      className="relative mx-4 mb-1 h-1.5 rounded-full bg-muted"
      aria-label={t("insights.moments")}
    >
      {props.moments.map((moment, i) => (
        <span
          key={`${moment.timeMs}-${i}`}
          className={cn(
            "absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-current ring-2 ring-card",
            MOMENT_COLORS[moment.kind],
          )}
          style={{ left: `${Math.min(100, (moment.timeMs / total) * 100)}%` }}
          title={`${formatDuration(moment.timeMs)} · ${t(`moments.${moment.kind}`)}: ${moment.text}`}
        />
      ))}
    </div>
  );
}

function TranscriptRow(props: {
  line: Line;
  translation: TranslationEvent | undefined;
  /** Presente se la frase si può tradurre su richiesta. */
  onTranslate?: () => Promise<void>;
}) {
  const { line, translation } = props;
  const { t } = useTranslation();
  const { copied, copy } = useCopy();
  const partial = line.id === 0;

  return (
    <li className="group grid grid-cols-[2.75rem_6rem_1fr_auto_1.5rem] items-baseline gap-2 rounded-lg px-2 py-1.5 text-sm hover:bg-muted/60">
      <span className="font-mono text-xs text-muted-foreground tabular-nums">{formatDuration(line.start)}</span>
      <SpeakerLabel channel={line.channel} speaker={line.speaker} editable={!partial} />
      <span className={cn("leading-relaxed select-text", partial && "text-muted-foreground italic")}>
        {line.text}
        {translation?.text && (
          <span className="mt-0.5 block text-muted-foreground">
            {line.channel === "me" && (
              <span
                className="mr-1.5 rounded bg-muted px-1 py-px align-middle font-mono text-[10px] uppercase"
                title={t("transcript.mineTranslation")}
              >
                {translation.language}
              </span>
            )}
            {translation.text}
          </span>
        )}
      </span>
      {props.onTranslate ? <TranslateButton onTranslate={props.onTranslate} /> : <span />}
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

/** Pulsante "Traduci" della singola frase, con attesa ed errore. */
function TranslateButton({ onTranslate }: { onTranslate: () => Promise<void> }) {
  const { t } = useTranslation();
  const [state, setState] = useState<{ busy: boolean; error: string | null }>({
    busy: false,
    error: null,
  });

  const run = async () => {
    setState({ busy: true, error: null });
    try {
      await onTranslate();
    } catch (e) {
      setState({ busy: false, error: errorMessage(e) });
    }
  };

  return (
    <Button
      variant="ghost"
      size="xs"
      className={cn(
        "self-start text-muted-foreground hover:text-foreground",
        state.error && "text-destructive hover:text-destructive",
      )}
      disabled={state.busy}
      title={state.error ?? t("transcript.translate")}
      onClick={() => void run()}
    >
      {state.busy ? <Loader2 className="animate-spin" /> : state.error ? <AlertCircle /> : <Languages />}
      {state.error ? t("transcript.translateRetry") : t("transcript.translate")}
    </Button>
  );
}
