import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import type { ChannelId } from "@/lib/ipc";
import { useTranscriptStore } from "@/stores/transcript";

/** Quota del tempo di parola Io/Loro, dalla durata delle frasi finali. */
export function TalkRatio() {
  const { t } = useTranslation();
  const lines = useTranscriptStore((s) => s.lines);

  const totals = useMemo(() => {
    const acc: Record<ChannelId, number> = { me: 0, them: 0 };
    for (const l of lines) acc[l.channel] += Math.max(0, l.end - l.start);
    return acc;
  }, [lines]);

  const total = totals.me + totals.them;
  const me = total > 0 ? Math.round((totals.me / total) * 100) : 0;
  const them = total > 0 ? 100 - me : 0;
  const label = total > 0 ? t("talk.label", { me, them }) : t("talk.empty");

  return (
    <section
      className="flex items-center gap-3 rounded-2xl border bg-card px-4 py-3 text-sm shadow-xs"
      aria-label={t("talk.title")}
      title={label}
    >
      <span className="shrink-0 text-muted-foreground">
        {t("channel.me")}: <span className="font-medium text-foreground tabular-nums">{total > 0 ? `${me}%` : "—"}</span>
      </span>
      <div role="img" aria-label={label} className="flex h-2 flex-1 overflow-hidden rounded-full bg-muted">
        {total > 0 && (
          <>
            <div className="bg-me transition-[width] duration-500" style={{ width: `${me}%` }} />
            <div className="flex-1 bg-them" />
          </>
        )}
      </div>
      <span className="shrink-0 text-muted-foreground">
        {t("channel.them")}: <span className="font-medium text-foreground tabular-nums">{total > 0 ? `${them}%` : "—"}</span>
      </span>
    </section>
  );
}
