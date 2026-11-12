import { BookUser, History, type LucideIcon, Radio, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Logo } from "@/components/Logo";
import { cn } from "@/lib/utils";
import { useAudioStore } from "@/stores/audio";

export type View = "live" | "history" | "contacts" | "settings";

const NAV: { view: View; icon: LucideIcon; label: string }[] = [
  { view: "live", icon: Radio, label: "nav.live" },
  { view: "history", icon: History, label: "nav.history" },
  { view: "contacts", icon: BookUser, label: "nav.contacts" },
  { view: "settings", icon: Settings, label: "nav.settings" },
];

/** Barra laterale a icone: logo e navigazione tra le viste. */
export function Sidebar({ view, onChange }: { view: View; onChange: (v: View) => void }) {
  const { t } = useTranslation();
  const running = useAudioStore((s) => s.status === "running");

  return (
    <nav
      className="flex w-16 shrink-0 flex-col items-center gap-2 border-r bg-sidebar py-4"
      aria-label={t("nav.label")}
    >
      <div className="mb-4 size-10" title={t("app.title")}>
        <Logo className="size-10" label={t("app.title")} />
      </div>
      {NAV.map(({ view: v, icon: Icon, label }) => (
        <button
          key={v}
          type="button"
          aria-label={t(label)}
          aria-current={view === v ? "page" : undefined}
          title={t(label)}
          onClick={() => onChange(v)}
          className={cn(
            "relative flex size-10 items-center justify-center rounded-xl text-muted-foreground transition-colors outline-none hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50",
            view === v && "bg-sidebar-accent text-sidebar-accent-foreground",
          )}
        >
          <Icon className="size-5" aria-hidden />
          {v === "live" && running && (
            <span className="absolute top-2 right-2 size-2 rounded-full bg-destructive ring-2 ring-sidebar" />
          )}
        </button>
      ))}
    </nav>
  );
}
