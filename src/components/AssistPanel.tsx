import { Check, Copy, Lightbulb, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import type { AssistEvent } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useAssistStore } from "@/stores/assist";

/** Suggerimenti del live assist: cosa dire / cosa chiedere, i più recenti in alto. */
export function AssistPanel() {
  const { t } = useTranslation();
  const items = useAssistStore((s) => s.items);

  return (
    <section className="rounded-md border" aria-label={t("assist.title")}>
      <h2 className="flex items-center gap-2 px-3 py-2 text-sm font-medium">
        <Lightbulb className="size-4 text-muted-foreground" aria-hidden />
        {t("assist.title")}
      </h2>
      {items.length === 0 ? (
        <p className="border-t px-3 py-2 text-xs text-muted-foreground">{t("assist.empty")}</p>
      ) : (
        <ul className="flex flex-col divide-y border-t" aria-live="polite">
          {items.map((item) => (
            <AssistItem key={item.id} item={item} />
          ))}
        </ul>
      )}
    </section>
  );
}

function AssistItem({ item }: { item: AssistEvent }) {
  const { t } = useTranslation();
  const dismiss = useAssistStore((s) => s.dismiss);
  const [copied, setCopied] = useState(false);

  const copy = () => {
    navigator.clipboard.writeText(item.text).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      () => setCopied(false),
    );
  };

  return (
    <li className="flex items-start gap-2 px-3 py-2 text-sm">
      <span
        className={cn(
          "mt-0.5 shrink-0 rounded px-1.5 py-0.5 text-xs font-medium",
          item.kind === "say" ? "bg-chart-1/15 text-chart-1" : "bg-chart-2/15 text-chart-2",
        )}
      >
        {t(`assist.${item.kind}`)}
      </span>
      <span className="min-w-0 flex-1">{item.text}</span>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={t("assist.copy")}
        title={t("assist.copy")}
        onClick={copy}
      >
        {copied ? <Check /> : <Copy />}
      </Button>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={t("assist.dismiss")}
        title={t("assist.dismiss")}
        onClick={() => dismiss(item.id)}
      >
        <X />
      </Button>
    </li>
  );
}
