import { AlertCircle, CheckCircle2, X } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useInsightsStore } from "@/stores/insights";

/** Dopo quanto l'avviso di riepilogo pronto sparisce da solo. */
const AUTO_HIDE_MS = 12_000;

/** Avviso in basso a destra quando il riepilogo in background è pronto o fallisce. */
export function SummaryNotice(props: { onOpen: (meetingId: number) => void }) {
  const { t } = useTranslation();
  const notice = useInsightsStore((s) => s.notice);
  const dismiss = useInsightsStore((s) => s.dismissNotice);

  useEffect(() => {
    if (notice?.kind !== "ready") return;
    const timer = setTimeout(dismiss, AUTO_HIDE_MS);
    return () => clearTimeout(timer);
  }, [notice, dismiss]);

  if (!notice) return null;
  const ready = notice.kind === "ready";
  return (
    <div
      role={ready ? "status" : "alert"}
      className={cn(
        "fixed right-4 bottom-4 z-50 flex w-80 max-w-[calc(100vw-2rem)] items-start gap-3 rounded-xl border bg-card px-4 py-3 text-sm shadow-lg",
        !ready && "border-destructive/40",
      )}
    >
      {ready ? (
        <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden />
      ) : (
        <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
      )}
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <span className="font-medium">
          {ready ? t("summary.ready") : t("summary.failed", { message: notice.message })}
        </span>
        {ready && (
          <Button
            size="xs"
            variant="outline"
            className="self-start"
            onClick={() => {
              props.onOpen(notice.meetingId);
              dismiss();
            }}
          >
            {t("summary.open")}
          </Button>
        )}
      </div>
      <button
        type="button"
        className="rounded-md p-0.5 text-muted-foreground hover:text-foreground"
        onClick={dismiss}
        aria-label={t("summary.dismiss")}
      >
        <X className="size-4" aria-hidden />
      </button>
    </div>
  );
}
