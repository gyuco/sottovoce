import { ArrowDown } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { type Line, useTranscriptStore } from "@/stores/transcript";

/** Distanza dal fondo (px) entro cui l'autoscroll resta agganciato. */
const STICK_PX = 48;

function clock(ms: number) {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function Transcript() {
  const { t } = useTranslation();
  const lines = useTranscriptStore((s) => s.lines);
  const partial = useTranscriptStore((s) => s.partial);

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
  }, [rows]);

  const toBottom = () => {
    const el = box.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    stick.current = true;
    setDetached(false);
  };

  return (
    <section className="relative flex min-h-0 flex-1 flex-col rounded-xl border bg-card">
      <div
        ref={box}
        onScroll={onScroll}
        className="min-h-0 flex-1 overflow-y-auto px-4 py-3"
        aria-live="polite"
        aria-label={t("transcript.title")}
      >
        {rows.length === 0 ? (
          <p className="py-10 text-center text-sm text-muted-foreground">{t("transcript.empty")}</p>
        ) : (
          <ol className="flex flex-col gap-2">
            {rows.map((r) => (
              <li
                key={r.id === 0 ? `partial-${r.channel}` : r.id}
                className="grid grid-cols-[3rem_3rem_1fr] items-baseline gap-2 text-sm"
              >
                <span className="font-mono text-xs text-muted-foreground tabular-nums">
                  {clock(r.start)}
                </span>
                <span
                  className={
                    r.channel === "me"
                      ? "font-medium text-chart-1"
                      : "font-medium text-chart-2"
                  }
                >
                  {t(`channel.${r.channel}`)}
                </span>
                <span className={r.id === 0 ? "text-muted-foreground italic" : undefined}>
                  {r.text}
                </span>
              </li>
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
