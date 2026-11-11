import { Check, CircleHelp, Copy, Lightbulb, MessageCircle, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useCopy } from "@/lib/hooks";
import type { AssistEvent, AssistKind } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useAssistStore } from "@/stores/assist";
import { useSettingsStore } from "@/stores/settings";

const KIND_STYLE: Record<AssistKind, { icon: typeof MessageCircle; text: string; card: string; check: string }> = {
  say: {
    icon: MessageCircle,
    text: "text-say",
    card: "border-transparent hover:bg-muted/60",
    check: "border-say/50 data-[checked=true]:bg-say data-[checked=true]:border-say",
  },
  ask: {
    icon: CircleHelp,
    text: "text-ask",
    card: "border-transparent hover:bg-muted/60",
    check: "border-ask/50 data-[checked=true]:bg-ask data-[checked=true]:border-ask",
  },
};

/**
 * Live assist: cosa dire e cosa chiedere, in due gruppi, i più recenti in
 * alto. Pannello secondario e discreto; spento di default (setting
 * `liveAssist`), si accende anche a call in corso.
 */
export function AssistPanel() {
  const { t } = useTranslation();
  const items = useAssistStore((s) => s.items);
  const enabled = useSettingsStore((s) => s.settings?.liveAssist ?? false);
  const update = useSettingsStore((s) => s.update);

  return (
    <section className="flex flex-col rounded-2xl border bg-card/60" aria-label={t("assist.title")}>
      <header className="flex items-center gap-2 px-4 py-3">
        <Lightbulb className="size-4 text-muted-foreground" aria-hidden />
        <h2 className="flex-1 text-sm font-medium text-muted-foreground">{t("assist.toggle")}</h2>
        <Switch
          checked={enabled}
          onCheckedChange={(liveAssist) => void update({ liveAssist })}
          aria-label={t("assist.toggle")}
        />
      </header>
      {!enabled ? (
        <p className="px-4 pb-3 text-xs text-muted-foreground">{t("assist.off")}</p>
      ) : (
      <div className="flex max-h-80 flex-col gap-4 overflow-y-auto px-3 pb-3" aria-live="polite">
        {items.length === 0 ? (
          <p className="px-1 py-2 text-xs text-muted-foreground">{t("assist.empty")}</p>
        ) : (
          <>
            <AssistGroup kind="say" items={items} />
            <AssistGroup kind="ask" items={items} />
          </>
        )}
      </div>
      )}
    </section>
  );
}

function AssistGroup({ kind, items }: { kind: AssistKind; items: AssistEvent[] }) {
  const { t } = useTranslation();
  const style = KIND_STYLE[kind];
  const Icon = style.icon;
  const mine = items.filter((i) => i.kind === kind);

  return (
    <div className="flex flex-col gap-2">
      <h3 className="flex items-center gap-2 px-1 text-xs font-medium text-muted-foreground">
        <Icon className={cn("size-4", style.text)} aria-hidden />
        {t(`assist.${kind}Title`)}
        {mine.length > 0 && <span className="text-xs font-normal text-muted-foreground">{mine.length}</span>}
      </h3>
      {mine.length === 0 ? (
        <p className="px-1 text-xs text-muted-foreground">{t("assist.none")}</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {mine.map((item) => (
            <AssistItem key={item.id} item={item} />
          ))}
        </ul>
      )}
    </div>
  );
}

function AssistItem({ item }: { item: AssistEvent }) {
  const { t } = useTranslation();
  const dismiss = useAssistStore((s) => s.dismiss);
  const toggleUsed = useAssistStore((s) => s.toggleUsed);
  const used = useAssistStore((s) => s.used.includes(item.id));
  const { copied, copy } = useCopy();
  const style = KIND_STYLE[item.kind];

  return (
    <li
      className={cn(
        "group flex animate-in items-start gap-2.5 rounded-lg border px-2 py-1.5 text-[13px] text-foreground/85 transition-colors duration-200 fade-in slide-in-from-top-1",
        style.card,
        used && "opacity-55",
      )}
    >
      <button
        type="button"
        role="checkbox"
        aria-checked={used}
        data-checked={used}
        aria-label={t(used ? "assist.markUnused" : "assist.markUsed")}
        title={t(used ? "assist.markUnused" : "assist.markUsed")}
        onClick={() => toggleUsed(item.id)}
        className={cn(
          "mt-0.5 flex size-4.5 shrink-0 items-center justify-center rounded-[5px] border-[1.5px] bg-card text-white transition-colors outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
          style.check,
        )}
      >
        {used && <Check className="size-3" strokeWidth={3} />}
      </button>
      <p
        className={cn(
          "min-w-0 flex-1 leading-relaxed select-text",
          used && "line-through decoration-foreground/30",
        )}
      >
        {item.text}
      </p>
      <div className="-my-0.5 flex shrink-0 gap-0.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={t("assist.copy")}
          title={copied ? t("common.copied") : t("assist.copy")}
          onClick={() => copy(item.text)}
        >
          {copied ? <Check className="text-emerald-600" /> : <Copy />}
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
      </div>
    </li>
  );
}
