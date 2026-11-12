import { open } from "@tauri-apps/plugin-dialog";
import {
  CheckCircle2,
  ChevronDown,
  FolderOpen,
  Loader2,
  NotebookPen,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { ParticipantsInput } from "@/components/ParticipantsInput";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { type CodePathInfo, errorMessage, inspectCodePath } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { MEETING_LIMITS, useMeetingStore } from "@/stores/meeting";

/** Attesa prima di verificare la cartella del codice mentre si digita. */
const CHECK_DELAY_MS = 500;

/** Nome, obiettivo, note e cartella del codice: modificabili anche in sessione. */
export function MeetingContextPanel() {
  const { t } = useTranslation();
  const { context, syncError, set } = useMeetingStore();
  const [openPanel, setOpenPanel] = useState(false);
  const people =
    context.participants.length > 0
      ? t("participants.count", { count: context.participants.length })
      : "";
  const summary = [context.name, context.goal, people].filter((s) => s.trim() !== "").join(" · ");

  return (
    <section className="rounded-2xl border bg-card shadow-xs">
      <button
        type="button"
        className="flex w-full items-center gap-2 rounded-2xl px-4 py-3 text-left text-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
        aria-expanded={openPanel}
        aria-controls="meeting-context"
        onClick={() => setOpenPanel((o) => !o)}
      >
        <NotebookPen className="size-4 shrink-0 text-primary" aria-hidden />
        <span className="font-semibold">{t("meeting.title")}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">
          {summary || t("meeting.empty")}
        </span>
        <ChevronDown
          className={cn(
            "size-4 shrink-0 text-muted-foreground transition-transform",
            openPanel && "rotate-180",
          )}
          aria-hidden
        />
      </button>

      {openPanel && (
        <div id="meeting-context" className="flex flex-col gap-2 border-t px-4 py-3">
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
          <ParticipantsInput
            value={context.participants}
            onChange={(participants) => set({ participants })}
          />
          <CodePathField value={context.codePath} onChange={(codePath) => set({ codePath })} />
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

/**
 * Cartella del progetto (facoltativa): campo, selettore cartelle e verifica
 * con debounce (esiste? quanti file? quali linguaggi?).
 */
export function CodePathField(props: {
  value: string;
  onChange: (path: string) => void;
  inputClassName?: string;
}) {
  const { t } = useTranslation();
  const path = props.value.trim();
  /** Esito legato al percorso che lo ha prodotto: cambiando testo si riparte. */
  const [result, setResult] = useState<{
    path: string;
    info: CodePathInfo | null;
    error: boolean;
  } | null>(null);
  const [pickError, setPickError] = useState<string | null>(null);

  // Verifica automatica (con debounce): la UI deve dire subito se la cartella
  // esiste e quanti file vede il live assist.
  useEffect(() => {
    if (path === "") return;
    let cancelled = false;
    const timer = setTimeout(() => {
      inspectCodePath(path).then(
        (info) => {
          if (!cancelled) setResult({ path, info, error: false });
        },
        () => {
          if (!cancelled) setResult({ path, info: null, error: true });
        },
      );
    }, CHECK_DELAY_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [path]);

  const current = result?.path === path ? result : null;
  const code = current?.info ?? null;
  const checkError = current?.error ?? false;
  const checking = path !== "" && current === null;

  const pickFolder = async () => {
    try {
      const chosen = await open({
        directory: true,
        multiple: false,
        title: t("meeting.codePathBrowse"),
      });
      if (typeof chosen === "string") {
        setPickError(null);
        props.onChange(chosen);
      }
    } catch (e) {
      setPickError(errorMessage(e));
    }
  };

  const status = (() => {
    if (pickError) return <StatusLine error>{pickError}</StatusLine>;
    if (checkError) return <StatusLine error>{t("meeting.codePathError")}</StatusLine>;
    if (path === "") {
      return <StatusLine muted>{t("meeting.codePathHint")}</StatusLine>;
    }
    if (checking || (!code && !checkError)) {
      return (
        <StatusLine muted icon={<Loader2 className="animate-spin" aria-hidden />}>
          {t("meeting.codePathChecking")}
        </StatusLine>
      );
    }
    if (!code) return null;
    if (!code.exists) return <StatusLine error>{t("meeting.codePathMissing")}</StatusLine>;
    if (!code.isDir) return <StatusLine error>{t("meeting.codePathNotDir")}</StatusLine>;
    const languages = code.languages.slice(0, 3).map((l) => l.name).join(", ");
    return (
      <StatusLine muted icon={<CheckCircle2 className="text-primary" aria-hidden />}>
        {t("meeting.codePathOk", { count: code.files.toLocaleString() })}
        {languages ? ` · ${languages}` : ""}
      </StatusLine>
    );
  })();

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex gap-2">
        <Input
          aria-label={t("meeting.codePath")}
          placeholder={t("meeting.codePathPlaceholder")}
          maxLength={MEETING_LIMITS.codePath}
          value={props.value}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => {
            setPickError(null);
            props.onChange(e.target.value);
          }}
          className={cn("min-w-0 font-mono text-xs", props.inputClassName)}
        />
        <Button
          type="button"
          variant="outline"
          size="icon"
          aria-label={t("meeting.codePathBrowse")}
          title={t("meeting.codePathBrowse")}
          onClick={() => void pickFolder()}
        >
          <FolderOpen aria-hidden />
        </Button>
      </div>
      {status}
    </div>
  );
}

function StatusLine({
  children,
  icon,
  muted = false,
  error = false,
}: {
  children: React.ReactNode;
  icon?: React.ReactNode;
  muted?: boolean;
  error?: boolean;
}) {
  return (
    <p
      role={error ? "alert" : undefined}
      className={cn(
        "flex items-start gap-1.5 text-xs [&_svg]:mt-0.5 [&_svg]:size-3.5 [&_svg]:shrink-0",
        error ? "text-destructive" : muted && "text-muted-foreground",
      )}
    >
      {error ? <TriangleAlert aria-hidden /> : icon}
      <span className="min-w-0 break-all">{children}</span>
    </p>
  );
}
