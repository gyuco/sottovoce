import { Activity, AlertCircle, ChevronDown, Loader2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { SummaryPanel } from "@/components/SummaryPanel";
import { formatDuration } from "@/lib/hooks";
import { MOMENT_COLORS } from "@/lib/moments";
import { cn } from "@/lib/utils";
import { useInsightsStore } from "@/stores/insights";

/** Riepilogo progressivo dei blocchi + riepilogo finale della sessione. */
export function InsightsPanel() {
  const { t } = useTranslation();
  const blocks = useInsightsStore((s) => s.blocks);
  const summary = useInsightsStore((s) => s.summary);
  const setSummary = useInsightsStore((s) => s.setSummary);
  const pending = useInsightsStore((s) => s.pendingMeetingId !== null);
  const summaryError = useInsightsStore((s) => s.summaryError);
  const [open, setOpen] = useState(true);

  if (summary) {
    return <SummaryPanel summary={summary} onChange={setSummary} />;
  }
  const status = pending ? (
    <div
      role="status"
      className="flex items-start gap-3 rounded-2xl border border-primary/30 bg-primary/5 px-4 py-3 text-sm shadow-xs"
    >
      <Loader2 className="mt-0.5 size-4 shrink-0 animate-spin text-primary" aria-hidden />
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="font-semibold">{t("summary.generating")}</span>
        <span className="text-xs text-muted-foreground">{t("summary.generatingHint")}</span>
      </div>
    </div>
  ) : summaryError ? (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-2xl border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive"
    >
      <AlertCircle className="mt-0.5 size-4 shrink-0" aria-hidden />
      <span className="min-w-0">{t("summary.failed", { message: summaryError })}</span>
    </div>
  ) : null;
  if (blocks.length === 0) return status;

  return (
    <>
      {status}
      <section className="rounded-2xl border bg-card shadow-xs">
        <button
          type="button"
          className="flex w-full items-center gap-2 rounded-2xl px-4 py-3 text-left text-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
          aria-expanded={open}
          onClick={() => setOpen((o) => !o)}
        >
          <Activity className="size-4 shrink-0 text-primary" aria-hidden />
          <span className="font-semibold">{t("insights.title")}</span>
          <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
            {t("insights.blocks", { count: blocks.length })}
          </span>
          <ChevronDown
            className={cn("size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-180")}
            aria-hidden
          />
        </button>
        {open && (
          <div className="flex max-h-64 flex-col gap-3 overflow-y-auto border-t px-4 py-3">
            {blocks.map((block) => (
              <article key={block.index} className="flex flex-col gap-1 text-sm">
                <p className="font-mono text-xs text-muted-foreground tabular-nums">
                  {formatDuration(block.start)}–{formatDuration(block.end)}
                </p>
                <p className="leading-relaxed whitespace-pre-line select-text">{block.summary}</p>
                {block.topics.length > 0 && (
                  <p className="text-xs text-muted-foreground">{block.topics.join(" · ")}</p>
                )}
                {block.moments.length > 0 && (
                  <ul className="flex flex-col gap-0.5">
                    {block.moments.map((moment, i) => (
                      <li key={`${moment.timeMs}-${i}`} className="flex gap-1.5 text-xs">
                        <span className="font-mono text-muted-foreground tabular-nums">
                          {formatDuration(moment.timeMs)}
                        </span>
                        <span className={cn("font-medium", MOMENT_COLORS[moment.kind])}>
                          {t(`moments.${moment.kind}`)}
                        </span>
                        <span className="min-w-0 flex-1 text-muted-foreground">{moment.text}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </article>
            ))}
          </div>
        )}
      </section>
    </>
  );
}

