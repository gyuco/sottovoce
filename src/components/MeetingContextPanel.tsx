import { ChevronDown, NotebookPen } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { MEETING_LIMITS, useMeetingStore } from "@/stores/meeting";

/** Nome, obiettivo e note del meeting: compatto, modificabile anche in sessione. */
export function MeetingContextPanel() {
  const { t } = useTranslation();
  const { context, syncError, set } = useMeetingStore();
  const [open, setOpen] = useState(false);

  const summary = [context.name, context.goal].filter((s) => s.trim() !== "").join(" · ");

  return (
    <section className="rounded-md border">
      <button
        type="button"
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm"
        aria-expanded={open}
        aria-controls="meeting-context"
        onClick={() => setOpen((o) => !o)}
      >
        <NotebookPen className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        <span className="font-medium">{t("meeting.title")}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">
          {summary || t("meeting.empty")}
        </span>
        <ChevronDown
          className={cn("size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-180")}
          aria-hidden
        />
      </button>

      {open && (
        <div id="meeting-context" className="flex flex-col gap-2 border-t px-3 py-3">
          <Input
            aria-label={t("meeting.name")}
            placeholder={t("meeting.name")}
            maxLength={MEETING_LIMITS.name}
            value={context.name}
            onChange={(e) => set({ name: e.target.value })}
          />
          <Input
            aria-label={t("meeting.goal")}
            placeholder={t("meeting.goalPlaceholder")}
            maxLength={MEETING_LIMITS.goal}
            value={context.goal}
            onChange={(e) => set({ goal: e.target.value })}
          />
          <textarea
            aria-label={t("meeting.notes")}
            placeholder={t("meeting.notesPlaceholder")}
            maxLength={MEETING_LIMITS.notes}
            rows={3}
            value={context.notes}
            onChange={(e) => set({ notes: e.target.value })}
            className="min-h-16 w-full resize-y rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:bg-input/30"
          />
          <p className="flex justify-between gap-3 text-xs text-muted-foreground">
            <span>{t("meeting.hint")}</span>
            <span>
              {context.notes.length}/{MEETING_LIMITS.notes}
            </span>
          </p>
          {syncError && (
            <p role="alert" className="text-xs text-destructive">
              {syncError}
            </p>
          )}
        </div>
      )}
    </section>
  );
}
