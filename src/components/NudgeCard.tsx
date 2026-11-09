import { Gauge, MessageCircleQuestion, Mic, Timer, Users, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import type { Nudge, NudgeKind } from "@/lib/nudges";
import { cn } from "@/lib/utils";

const ICONS: Record<NudgeKind, typeof Mic> = {
  monologue: Timer,
  talkTooMuch: Mic,
  tooFast: Gauge,
  listenLong: Users,
  noQuestions: MessageCircleQuestion,
};

/** Avviso breve durante la call (finestra principale e widget). */
export function NudgeCard(props: { nudge: Nudge; onDismiss: () => void; className?: string }) {
  const { t } = useTranslation();
  const { nudge } = props;
  const Icon = ICONS[nudge.kind];
  const minutes = Math.floor(nudge.value / 60);
  const seconds = String(nudge.value % 60).padStart(2, "0");

  return (
    <div
      role="status"
      className={cn(
        "flex animate-in items-start gap-3 rounded-xl border px-3 py-2.5 text-sm fade-in slide-in-from-top-1",
        nudge.level === "medium"
          ? "border-rose-500/30 bg-rose-500/10 text-rose-900 dark:text-rose-200"
          : "border-amber-500/30 bg-amber-500/10 text-amber-900 dark:text-amber-200",
        props.className,
      )}
    >
      <Icon className="mt-0.5 size-4 shrink-0" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{t(`nudge.${nudge.kind}.title`, { value: nudge.value, minutes, seconds })}</p>
        <p className="text-xs opacity-80">{t(`nudge.${nudge.kind}.hint`)}</p>
      </div>
      <Button
        variant="ghost"
        size="icon-xs"
        className="-my-0.5 shrink-0"
        aria-label={t("nudge.dismiss")}
        title={t("nudge.dismiss")}
        onClick={props.onDismiss}
      >
        <X />
      </Button>
    </div>
  );
}
