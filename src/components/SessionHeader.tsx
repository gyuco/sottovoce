import { Clock, Cloud, Loader2, PictureInPicture2, Play, ShieldCheck, Square } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";

import { CodePathField } from "@/components/MeetingContextPanel";
import { ParticipantsInput } from "@/components/ParticipantsInput";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { formatDuration, useNow } from "@/lib/hooks";
import { CONCRETE_LANGUAGES, type Language, type Participant, toggleWidget } from "@/lib/ipc";
import { isLocalSession } from "@/lib/privacy";
import { useSttBlocker } from "@/lib/readiness";
import { cn } from "@/lib/utils";
import { useAudioStore, useSelectedMicId } from "@/stores/audio";
import { MEETING_LIMITS, useMeetingStore } from "@/stores/meeting";
import { useSettingsStore } from "@/stores/settings";

/** Nome del meeting, ora di inizio, cronometro e comando avvia/ferma. */
export function SessionHeader() {
  const { t, i18n } = useTranslation();
  const { status, startedAt, startedWall, start, stop } = useAudioStore();
  const settings = useSettingsStore((s) => s.settings);
  const blocked = useSttBlocker() !== null;
  const name = useMeetingStore((s) => s.context.name.trim());
  const micId = useSelectedMicId();
  const [choosing, setChoosing] = useState(false);

  const busy = status === "starting" || status === "stopping";
  const running = status === "running";
  const now = useNow(running, 1000);

  const since = new Date(startedWall).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });

  return (
    <header className="flex flex-wrap items-center gap-x-5 gap-y-3 px-6 pt-5 pb-4">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2.5">
          <h1 className="truncate text-2xl font-semibold tracking-tight">{name || t("header.untitled")}</h1>
          {settings && <ModeBadge local={isLocalSession(settings)} />}
        </div>
        <p className="mt-1 flex items-center gap-1.5 text-sm text-muted-foreground">
          <Clock className="size-3.5" aria-hidden />
          {running ? t("header.startedAt", { time: since }) : t("header.ready")}
        </p>
      </div>

      {running && (
        <div className="flex items-center gap-2.5" role="timer" aria-label={t("header.elapsed")}>
          <span className="relative flex size-2.5">
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-destructive/60" />
            <span className="relative inline-flex size-2.5 rounded-full bg-destructive" />
          </span>
          <span className="font-mono text-2xl font-medium tabular-nums">{formatDuration(now - startedAt)}</span>
        </div>
      )}

      <Button
        variant="outline"
        size="icon-lg"
        className="rounded-xl"
        aria-label={t("widget.toggle")}
        title={t("widget.toggleHint")}
        onClick={() => void toggleWidget()}
      >
        <PictureInPicture2 />
      </Button>

      {running || status === "stopping" ? (
        <Button
          variant="destructive"
          size="lg"
          className="rounded-xl shadow-sm"
          onClick={() => void stop()}
          disabled={busy}
        >
          {busy ? <Loader2 className="animate-spin" /> : <Square className="fill-current" />}
          {t("controls.stop")}
        </Button>
      ) : (
        <Button
          size="lg"
          className="rounded-xl shadow-sm"
          onClick={() => setChoosing(true)}
          disabled={busy || !micId || blocked}
        >
          {busy ? <Loader2 className="animate-spin" /> : <Play className="fill-current" />}
          {busy ? t("controls.starting") : t("controls.start")}
        </Button>
      )}

      {choosing && settings && (
        <StartDialog
          initial={settings.theirLanguage}
          onCancel={() => setChoosing(false)}
          onStart={(language) => {
            setChoosing(false);
            // La scelta diventa la proposta per la prossima call.
            void useSettingsStore.getState().update({ theirLanguage: language });
            void start(language);
          }}
        />
      )}
    </header>
  );
}

const CALL_LANGUAGES: Language[] = [...CONCRETE_LANGUAGES, "multi"];

/**
 * All'avvio: lingua della call (rilevamento di "Io" e "Loro") e contesto
 * facoltativo: nome, obiettivo e partecipanti attesi. Il contesto parte dai
 * valori correnti del pannello "Contesto meeting" e lo aggiorna.
 */
function StartDialog(props: {
  initial: Language;
  onCancel: () => void;
  onStart: (language: Language) => void;
}) {
  const { t } = useTranslation();
  const context = useMeetingStore((s) => s.context);
  const [language, setLanguage] = useState<Language>(props.initial);
  const [name, setName] = useState(context.name);
  const [goal, setGoal] = useState(context.goal);
  const [participants, setParticipants] = useState<Participant[]>(context.participants);
  const [codePath, setCodePath] = useState(context.codePath);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    useMeetingStore.getState().set({
      name: name.trim(),
      goal: goal.trim(),
      participants,
      codePath: codePath.trim(),
    });
    props.onStart(language);
  };

  return (
    <Dialog open onOpenChange={(open) => !open && props.onCancel()}>
      <DialogContent closeLabel={t("common.close")} className="sm:max-w-xl">
        <form onSubmit={submit} className="grid gap-5">
          <DialogHeader>
            <DialogTitle>{t("startDialog.heading")}</DialogTitle>
            <DialogDescription>{t("startDialog.intro")}</DialogDescription>
          </DialogHeader>

          <label className="grid gap-1.5 text-sm">
            <span className="font-medium">{t("startDialog.title")}</span>
            <Select value={language} onValueChange={(v) => setLanguage(v as Language)}>
              <SelectTrigger className="w-full" aria-label={t("startDialog.title")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CALL_LANGUAGES.map((l) => (
                  <SelectItem key={l} value={l}>
                    {t(`settings.language.${l}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <span className="text-xs text-muted-foreground">{t("startDialog.description")}</span>
          </label>

          <fieldset className="grid gap-3 rounded-xl border bg-muted/30 p-4">
            <legend className="px-1 text-sm font-medium">
              {t("startDialog.context")}{" "}
              <span className="font-normal text-muted-foreground">· {t("startDialog.optional")}</span>
            </legend>
            <div className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("startDialog.participants")}</span>
              <ParticipantsInput value={participants} onChange={setParticipants} className="bg-background" />
              <span className="text-xs text-muted-foreground">{t("startDialog.participantsHint")}</span>
            </div>
            <label className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("meeting.name")}</span>
              <Input
                value={name}
                maxLength={MEETING_LIMITS.name}
                placeholder={t("startDialog.namePlaceholder")}
                className="bg-background"
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <label className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("meeting.goal")}</span>
              <Input
                value={goal}
                maxLength={MEETING_LIMITS.goal}
                placeholder={t("meeting.goalPlaceholder")}
                className="bg-background"
                onChange={(e) => setGoal(e.target.value)}
              />
            </label>
            <div className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("meeting.codePath")}</span>
              <CodePathField value={codePath} onChange={setCodePath} inputClassName="bg-background" />
            </div>
          </fieldset>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={props.onCancel}>
              {t("common.cancel")}
            </Button>
            <Button type="submit">
              <Play className="fill-current" />
              {t("controls.start")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** "Locale": nessun dato esce dalla macchina; "Cloud": audio o testo a terzi. */
function ModeBadge({ local }: { local: boolean }) {
  const { t } = useTranslation();
  const Icon = local ? ShieldCheck : Cloud;
  const hint = t(local ? "header.mode.localHint" : "header.mode.cloudHint");
  return (
    <Badge
      variant="secondary"
      title={hint}
      aria-label={`${t(local ? "header.mode.local" : "header.mode.cloud")}: ${hint}`}
      className={cn(
        "shrink-0 gap-1 font-normal",
        local && "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
      )}
    >
      <Icon className="size-3" aria-hidden />
      {t(local ? "header.mode.local" : "header.mode.cloud")}
    </Badge>
  );
}
