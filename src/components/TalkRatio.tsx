import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { formatDuration } from "@/lib/hooks";
import type { ChannelId } from "@/lib/ipc";
import { type ChannelMetrics, type MetricLine, computeMetrics } from "@/lib/metrics";
import { cn } from "@/lib/utils";
import { useTranscriptStore } from "@/stores/transcript";

/** Metriche della sessione in corso. */
export function TalkRatio() {
  const lines = useTranscriptStore((s) => s.lines);
  return <ConversationMetrics lines={lines} />;
}

/** Tempo di parola Io/Loro più velocità, domande e monologo più lungo per canale. */
export function ConversationMetrics(props: { lines: readonly MetricLine[]; className?: string }) {
  const { t } = useTranslation();
  const metrics = useMemo(() => computeMetrics(props.lines), [props.lines]);

  const me = metrics.mySharePct;
  const them = me === null ? null : 100 - me;
  const label = me === null ? t("talk.empty") : t("talk.label", { me, them });

  return (
    <section
      className={cn("flex flex-col gap-2 rounded-2xl border bg-card px-4 py-3 text-sm shadow-xs", props.className)}
      aria-label={t("talk.title")}
    >
      <div className="flex items-center gap-3" title={label}>
        <span className="shrink-0 text-muted-foreground">
          {t("channel.me")}:{" "}
          <span className="font-medium text-foreground tabular-nums">{me === null ? "—" : `${me}%`}</span>
        </span>
        <div role="img" aria-label={label} className="flex h-2 flex-1 overflow-hidden rounded-full bg-muted">
          {me !== null && (
            <>
              <div className="bg-me transition-[width] duration-500" style={{ width: `${me}%` }} />
              <div className="flex-1 bg-them" />
            </>
          )}
        </div>
        <span className="shrink-0 text-muted-foreground">
          {t("channel.them")}:{" "}
          <span className="font-medium text-foreground tabular-nums">{them === null ? "—" : `${them}%`}</span>
        </span>
      </div>
      {me !== null && (
        <div className="grid grid-cols-2 gap-3 text-xs text-muted-foreground">
          <ChannelStats channel="me" metrics={metrics.me} />
          <ChannelStats channel="them" metrics={metrics.them} />
        </div>
      )}
    </section>
  );
}

function ChannelStats(props: { channel: ChannelId; metrics: ChannelMetrics }) {
  const { t } = useTranslation();
  const { metrics } = props;
  return (
    <dl className={cn("flex flex-col gap-0.5", props.channel === "them" && "items-end text-right")}>
      <Stat label={t("talk.wpm")} value={metrics.wpm === null ? "—" : String(metrics.wpm)} />
      <Stat label={t("talk.questions")} value={String(metrics.questions)} />
      <Stat
        label={t("talk.monologue")}
        value={metrics.longestMonologueMs > 0 ? formatDuration(metrics.longestMonologueMs) : "—"}
      />
    </dl>
  );
}

function Stat(props: { label: string; value: string }) {
  return (
    <div className="flex gap-1">
      <dt>{props.label}</dt>
      <dd className="font-medium text-foreground tabular-nums">{props.value}</dd>
    </div>
  );
}
