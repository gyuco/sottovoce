import { Download, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { openReleasePage } from "@/lib/ipc";
import { useUpdateStore } from "@/stores/update";

/** Riga in cima alla finestra quando è disponibile una release più recente. */
export function UpdateBanner() {
  const { t, i18n } = useTranslation();
  const info = useUpdateStore((s) => s.info);
  const dismiss = useUpdateStore((s) => s.dismiss);

  if (!info) return null;

  const date = info.publishedAt ? new Date(info.publishedAt) : null;
  const when =
    date && !Number.isNaN(date.getTime())
      ? date.toLocaleDateString(i18n.resolvedLanguage ?? "it", {
          day: "2-digit",
          month: "short",
          year: "numeric",
        })
      : null;

  return (
    <div
      role="status"
      className="flex shrink-0 items-center gap-3 border-b bg-card px-4 py-2 text-sm"
    >
      <Download className="size-4 shrink-0 text-primary" aria-hidden />
      <span className="font-medium">{t("update.available")}</span>
      <span className="min-w-0 flex-1 truncate text-muted-foreground">
        {t("update.detail", { version: info.version, build: info.build })}
        {when ? ` · ${when}` : null}
      </span>
      <Button size="xs" onClick={() => void openReleasePage(info.url)}>
        {t("update.download")}
      </Button>
      <button
        type="button"
        className="rounded-md p-0.5 text-muted-foreground hover:text-foreground"
        onClick={dismiss}
        aria-label={t("update.dismiss")}
      >
        <X className="size-4" aria-hidden />
      </button>
    </div>
  );
}
