import { CalendarClock, CalendarPlus, Loader2, Pencil, Play, Trash2, Users, X } from "lucide-react";
import { type FormEvent, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { CodePathField } from "@/components/MeetingContextPanel";
import { ParticipantsInput } from "@/components/ParticipantsInput";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
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
import { CONCRETE_LANGUAGES, type Language, type Participant, type ScheduledMeeting, errorMessage } from "@/lib/ipc";
import { useCanStart } from "@/lib/readiness";
import { cn } from "@/lib/utils";
import { MEETING_LIMITS } from "@/stores/meeting";
import { SCHEDULED_DURATIONS, endsAt, freshParticipants, isDue, useScheduledStore } from "@/stores/scheduled";
import { useSettingsStore } from "@/stores/settings";

const CALL_LANGUAGES: Language[] = [...CONCRETE_LANGUAGES, "multi"];

const pad = (n: number) => String(n).padStart(2, "0");

/** `YYYY-MM-DD` nell'ora locale (valore di `<input type="date">`). */
function dateValue(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** `HH:MM` nell'ora locale (valore di `<input type="time">`). */
function timeValue(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Data e ora locali → ms epoch; `null` se incompleti. */
function toMs(date: string, time: string): number | null {
  const [y, mo, d] = date.split("-").map(Number);
  const [h, mi] = time.split(":").map(Number);
  if (!y || !mo || !d || h === undefined || mi === undefined || Number.isNaN(h) || Number.isNaN(mi)) return null;
  return new Date(y, mo - 1, d, h, mi).getTime();
}

/** Prossima mezz'ora piena (proposta per una riunione nuova). */
function nextSlot(now: number): number {
  const d = new Date(now);
  d.setSeconds(0, 0);
  d.setMinutes(d.getMinutes() < 30 ? 30 : 60);
  return d.getTime();
}

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

function useFormatters() {
  const { t, i18n } = useTranslation();
  return useMemo(() => {
    const time = (ms: number) =>
      new Date(ms).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });
    const day = (ms: number, now: number) => {
      const diff = Math.round((startOfDay(ms) - startOfDay(now)) / 86_400_000);
      if (diff === 0) return t("scheduled.today");
      if (diff === 1) return t("scheduled.tomorrow");
      return new Date(ms).toLocaleDateString(i18n.language, { weekday: "long", day: "numeric", month: "long" });
    };
    return { time, day };
  }, [t, i18n.language]);
}

/** Agenda: riunioni programmate con contesto pronto, da avviare con un clic. */
export function Scheduled({ onStart }: { onStart: (m: ScheduledMeeting) => void }) {
  const { t } = useTranslation();
  const { meetings, loaded, error, now, load, remove } = useScheduledStore();
  const canStart = useCanStart();
  const fmt = useFormatters();
  // `null` = chiuso; "new" = nuova riunione.
  const [editing, setEditing] = useState<ScheduledMeeting | "new" | null>(null);
  const [deleting, setDeleting] = useState<ScheduledMeeting | null>(null);

  useEffect(() => {
    void load();
  }, [load]);

  const { groups, missed, hasUpcoming } = useMemo(() => {
    const byDay = new Map<number, ScheduledMeeting[]>();
    for (const m of meetings) {
      if (endsAt(m) <= now) continue;
      const key = startOfDay(m.startsAtMs);
      byDay.set(key, [...(byDay.get(key) ?? []), m]);
    }
    return {
      groups: [...byDay.entries()],
      missed: meetings.filter((m) => endsAt(m) <= now).reverse(),
      hasUpcoming: byDay.size > 0,
    };
  }, [meetings, now]);

  const row = (m: ScheduledMeeting, past: boolean) => {
    const due = isDue(m, now);
    return (
      <li key={m.id} className="flex items-start gap-3 px-4 py-3">
        <div className="w-20 shrink-0 pt-0.5 text-sm tabular-nums">
          <p className={cn("font-medium", due && "text-primary")}>{fmt.time(m.startsAtMs)}</p>
          <p className="text-xs text-muted-foreground">{t("scheduled.minutes", { count: m.durationMin })}</p>
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <p className="flex flex-wrap items-center gap-2 text-sm">
            <span className="truncate font-medium">{m.name}</span>
            {due && !past && (
              <Badge className="font-normal">
                {now >= m.startsAtMs ? t("scheduled.now") : t("scheduled.soon")}
              </Badge>
            )}
            {past && (
              <span className="text-xs text-muted-foreground">
                {fmt.day(m.startsAtMs, now)} · {t("scheduled.notStarted")}
              </span>
            )}
            <Badge variant="secondary" className="font-normal">
              {t(`settings.language.${m.language}`)}
            </Badge>
          </p>
          {m.goal && <p className="line-clamp-2 text-xs text-muted-foreground">{m.goal}</p>}
          {m.participants.length > 0 && (
            <p className="flex items-center gap-1 truncate text-xs text-muted-foreground">
              <Users className="size-3 shrink-0" aria-hidden />
              {m.participants.map((p) => p.name).join(", ")}
            </p>
          )}
        </div>
        {!past && (
          <Button
            size="sm"
            variant={due ? "default" : "outline"}
            disabled={!canStart}
            title={t("scheduled.startHint")}
            onClick={() => onStart(m)}
          >
            <Play className="fill-current" /> {t("controls.start")}
          </Button>
        )}
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={t("scheduled.edit", { name: m.name })}
          title={t("scheduled.edit", { name: m.name })}
          onClick={() => setEditing(m)}
        >
          <Pencil />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          className="text-destructive hover:text-destructive"
          aria-label={t("scheduled.delete", { name: m.name })}
          title={t("scheduled.delete", { name: m.name })}
          onClick={() => setDeleting(m)}
        >
          <Trash2 />
        </Button>
      </li>
    );
  };

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 px-6 py-5">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">{t("scheduled.title")}</h1>
          <p className="mt-1 text-sm text-muted-foreground">{t("scheduled.hint")}</p>
        </div>
        <Button onClick={() => setEditing("new")}>
          <CalendarPlus /> {t("scheduled.add")}
        </Button>
      </header>

      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}

      {!loaded ? (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" aria-hidden /> {t("history.loading")}
        </p>
      ) : !hasUpcoming ? (
        <p className="rounded-2xl border border-dashed px-6 py-10 text-center text-sm text-muted-foreground">
          {t("scheduled.empty")}
        </p>
      ) : (
        groups.map(([day, items]) => (
          <section key={day} className="flex flex-col gap-2">
            <h2 className="text-sm font-medium text-muted-foreground first-letter:uppercase">
              {fmt.day(day, now)}
            </h2>
            <ul className="flex flex-col divide-y rounded-2xl border bg-card shadow-xs">
              {items.map((m) => row(m, false))}
            </ul>
          </section>
        ))
      )}

      {missed.length > 0 && (
        <section className="flex flex-col gap-2">
          <h2 className="text-sm font-medium text-muted-foreground">{t("scheduled.missed")}</h2>
          <ul className="flex flex-col divide-y rounded-2xl border bg-card opacity-80 shadow-xs">
            {missed.map((m) => row(m, true))}
          </ul>
        </section>
      )}

      {editing !== null && (
        <ScheduledDialog meeting={editing === "new" ? null : editing} onClose={() => setEditing(null)} />
      )}

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("scheduled.deleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("scheduled.deleteConfirm", { name: deleting?.name ?? "" })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className={cn(buttonVariants({ variant: "destructive" }))}
              onClick={() => deleting && void remove(deleting.id)}
            >
              {t("scheduled.deleteAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** Nuova riunione o modifica: giorno, ora, durata, lingua e contesto. */
function ScheduledDialog(props: { meeting: ScheduledMeeting | null; onClose: () => void }) {
  const { t } = useTranslation();
  const save = useScheduledStore((s) => s.save);
  const defaultLanguage = useSettingsStore((s) => s.settings?.theirLanguage ?? "multi");
  const { meeting } = props;
  const [start] = useState(() => meeting?.startsAtMs ?? nextSlot(Date.now()));
  const [date, setDate] = useState(dateValue(start));
  const [time, setTime] = useState(timeValue(start));
  const [duration, setDuration] = useState(meeting?.durationMin ?? 30);
  const [language, setLanguage] = useState<Language>(meeting?.language ?? defaultLanguage);
  const [name, setName] = useState(meeting?.name ?? "");
  const [goal, setGoal] = useState(meeting?.goal ?? "");
  const [notes, setNotes] = useState(meeting?.notes ?? "");
  const [codePath, setCodePath] = useState(meeting?.codePath ?? "");
  const [participants, setParticipants] = useState<Participant[]>(() =>
    freshParticipants(meeting?.participants ?? []),
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const startsAtMs = toMs(date, time);
  const durations = SCHEDULED_DURATIONS.includes(duration as (typeof SCHEDULED_DURATIONS)[number])
    ? SCHEDULED_DURATIONS
    : [...SCHEDULED_DURATIONS, duration].sort((a, b) => a - b);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (startsAtMs === null || !name.trim()) return;
    setSaving(true);
    setError(null);
    try {
      await save({
        id: meeting?.id ?? null,
        name: name.trim(),
        goal: goal.trim(),
        notes: notes.trim(),
        codePath: codePath.trim(),
        participants,
        language,
        startsAtMs,
        durationMin: duration,
      });
      props.onClose();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && props.onClose()}>
      <DialogContent closeLabel={t("common.close")} className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <form onSubmit={(e) => void submit(e)} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{meeting ? t("scheduled.edit", { name: meeting.name }) : t("scheduled.add")}</DialogTitle>
            <DialogDescription>{t("scheduled.formHint")}</DialogDescription>
          </DialogHeader>

          <label className="grid gap-1.5 text-sm">
            <span className="font-medium">{t("meeting.name")}</span>
            <Input
              autoFocus
              required
              value={name}
              maxLength={MEETING_LIMITS.name}
              placeholder={t("startDialog.namePlaceholder")}
              onChange={(e) => setName(e.target.value)}
            />
          </label>

          <div className="grid gap-3 sm:grid-cols-[1fr_auto_auto]">
            <label className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("scheduled.date")}</span>
              <Input type="date" required value={date} onChange={(e) => setDate(e.target.value)} />
            </label>
            <label className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("scheduled.time")}</span>
              <Input type="time" required value={time} onChange={(e) => setTime(e.target.value)} />
            </label>
            <label className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("scheduled.duration")}</span>
              <Select value={String(duration)} onValueChange={(v) => setDuration(Number(v))}>
                <SelectTrigger className="w-full sm:w-32" aria-label={t("scheduled.duration")}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {durations.map((d) => (
                    <SelectItem key={d} value={String(d)}>
                      {t("scheduled.minutes", { count: d })}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          </div>

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
              <span className="font-medium">{t("meeting.goal")}</span>
              <Input
                value={goal}
                maxLength={MEETING_LIMITS.goal}
                placeholder={t("meeting.goalPlaceholder")}
                className="bg-background"
                onChange={(e) => setGoal(e.target.value)}
              />
            </label>
            <label className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("meeting.notes")}</span>
              <textarea
                placeholder={t("meeting.notesPlaceholder")}
                maxLength={MEETING_LIMITS.notes}
                rows={3}
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                className="min-h-16 w-full resize-y rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:bg-input/30"
              />
            </label>
            <div className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("meeting.codePath")}</span>
              <CodePathField value={codePath} onChange={setCodePath} inputClassName="bg-background" />
            </div>
          </fieldset>

          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}

          <DialogFooter>
            <Button type="button" variant="outline" onClick={props.onClose}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={saving || !name.trim() || startsAtMs === null}>
              {saving && <Loader2 className="animate-spin" />}
              {t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/**
 * In Live: la riunione programmata di adesso (da 10 minuti prima dell'inizio
 * alla fine prevista), con "Avvia" che parte subito con il contesto pronto.
 */
export function ScheduledBanner({ meeting }: { meeting: ScheduledMeeting }) {
  const { t } = useTranslation();
  const now = useScheduledStore((s) => s.now);
  const start = useScheduledStore((s) => s.start);
  const dismiss = useScheduledStore((s) => s.dismiss);
  const canStart = useCanStart();
  const fmt = useFormatters();
  const minutes = Math.ceil((meeting.startsAtMs - now) / 60_000);

  return (
    <div className="flex items-center gap-3 rounded-xl border border-primary/30 bg-primary/5 px-3 py-2 text-sm shadow-xs">
      <CalendarClock className="size-4 shrink-0 text-primary" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="truncate font-medium">{meeting.name}</p>
        <p className="truncate text-xs text-muted-foreground">
          {fmt.time(meeting.startsAtMs)} ·{" "}
          {minutes > 0 ? t("scheduled.startsIn", { count: minutes }) : t("scheduled.started")}
          {meeting.participants.length > 0 &&
            ` · ${t("scheduled.participants", { count: meeting.participants.length })}`}
        </p>
      </div>
      <Button size="sm" disabled={!canStart} title={t("scheduled.startHint")} onClick={() => void start(meeting)}>
        <Play className="fill-current" /> {t("controls.start")}
      </Button>
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label={t("scheduled.dismiss")}
        title={t("scheduled.dismiss")}
        onClick={() => dismiss(meeting.id)}
      >
        <X />
      </Button>
    </div>
  );
}
