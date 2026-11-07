import { Check, Clock, Loader2, Pencil, Search, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { SummaryPanel } from "@/components/SummaryPanel";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { formatDuration } from "@/lib/hooks";
import {
  type MeetingDetails,
  type MeetingSummary,
  deleteMeeting,
  errorMessage,
  generateSummary,
  getMeeting,
  listMeetings,
  renameMeeting,
} from "@/lib/ipc";
import { speakerColor } from "@/lib/speakers";
import { cn } from "@/lib/utils";
import { useInsightsStore } from "@/stores/insights";

const PER_PAGE = 20;

/** Storico: elenco dei meeting salvati e dettaglio in sola lettura. */
export function History(props: { focusId?: number | null }) {
  const { t, i18n } = useTranslation();
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [data, setData] = useState<{ meetings: MeetingSummary[]; total: number } | null>(null);
  // Aperto da un avviso: parte da quel meeting (il componente si rimonta).
  const [selected, setSelected] = useState<number | null>(props.focusId ?? null);
  const [details, setDetails] = useState<MeetingDetails | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [renaming, setRenaming] = useState(false);
  const [nameDraft, setNameDraft] = useState("");
  const [generatingSummary, setGeneratingSummary] = useState(false);
  const lastReady = useInsightsStore((s) => s.lastReady);
  const pendingId = useInsightsStore((s) => s.pendingMeetingId);

  // Ricerca con debounce: niente richiesta a ogni tasto.
  useEffect(() => {
    const timer = setTimeout(() => {
      setLoading(true);
      setPage(0);
      setQuery(search.trim());
    }, 300);
    return () => clearTimeout(timer);
  }, [search]);

  useEffect(() => {
    let alive = true;
    listMeetings(page, PER_PAGE, query).then(
      (result) => {
        if (!alive) return;
        setData(result);
        setError(null);
        setSelected((current) =>
          current !== null && result.meetings.some((m) => m.id === current)
            ? current
            : (result.meetings[0]?.id ?? null),
        );
      },
      (e: unknown) => alive && setError(errorMessage(e)),
    ).finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [page, query, reload]);

  useEffect(() => {
    if (selected === null) return;
    let alive = true;
    getMeeting(selected).then(
      (result) => {
        if (!alive) return;
        setDetails(result);
        setNameDraft(result.meeting.name);
        setError(null);
      },
      (e: unknown) => alive && setError(errorMessage(e)),
    );
    return () => {
      alive = false;
    };
  }, [selected, reload]);

  const meeting = selected === null ? null : (details?.meeting ?? null);
  // Riepilogo arrivato in background mentre il meeting era già aperto.
  const summary =
    details?.summary ?? (lastReady && lastReady.meetingId === meeting?.id ? lastReady : null);
  const detailLoading = selected !== null && details?.meeting.id !== selected;
  const translations = useMemo(
    () => new Map((details?.translations ?? []).map((tr) => [tr.sessionId, tr])),
    [details],
  );
  const speakerNames = useMemo(
    () => new Map((details?.speakers ?? []).map((s) => [s.speaker, s.name])),
    [details],
  );

  const date = new Intl.DateTimeFormat(i18n.language, {
    dateStyle: "medium",
    timeStyle: "short",
  });

  const remove = async () => {
    if (selected === null || !window.confirm(t("history.deleteConfirm"))) return;
    try {
      await deleteMeeting(selected);
      setSelected(null);
      setDetails(null);
      setLoading(true);
      setReload((r) => r + 1);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const saveName = async () => {
    if (selected === null) return;
    try {
      const updated = await renameMeeting(selected, nameDraft);
      setDetails((d) => (d ? { ...d, meeting: updated } : d));
      setRenaming(false);
      setReload((r) => r + 1);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const createSummary = async () => {
    if (selected === null) return;
    setGeneratingSummary(true);
    try {
      const summary = await generateSummary(selected);
      setDetails((d) => (d ? { ...d, summary } : d));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setGeneratingSummary(false);
    }
  };

  const total = data?.total ?? 0;
  const from = total === 0 ? 0 : page * PER_PAGE + 1;
  const to = Math.min((page + 1) * PER_PAGE, total);

  return (
    <div className="grid min-h-0 flex-1 gap-4 px-6 pt-5 pb-6 md:grid-cols-[22rem_minmax(0,1fr)]">
      <section className="flex min-h-0 flex-col rounded-2xl border bg-card shadow-xs">
        <header className="flex flex-col gap-2 border-b px-4 py-3">
          <h1 className="text-lg font-semibold tracking-tight">{t("history.title")}</h1>
          <div className="relative">
            <Search
              className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden
            />
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("history.searchPlaceholder")}
              aria-label={t("history.searchPlaceholder")}
              className="pl-8"
            />
          </div>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          {error && (
            <p role="alert" className="px-2 py-3 text-sm text-destructive">
              {error}
            </p>
          )}
          {!error && loading && !data && (
            <p className="flex items-center gap-2 px-2 py-3 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden /> {t("history.loading")}
            </p>
          )}
          {!error && data && data.meetings.length === 0 && (
            <p className="px-2 py-10 text-center text-sm text-muted-foreground">
              {query ? t("history.noResults") : t("history.empty")}
            </p>
          )}
          <ul className="flex flex-col gap-1">
            {data?.meetings.map((m) => (
              <li key={m.id}>
                <button
                  type="button"
                  onClick={() => setSelected(m.id)}
                  aria-current={m.id === selected ? "true" : undefined}
                  className={cn(
                    "flex w-full flex-col gap-1 rounded-xl px-3 py-2 text-left outline-none transition-colors hover:bg-muted/70 focus-visible:ring-[3px] focus-visible:ring-ring/50",
                    m.id === selected && "bg-muted",
                  )}
                >
                  <span className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">
                      {m.name || t("history.untitled")}
                    </span>
                    <Badge variant="secondary">
                      {m.sttEngine === "local" ? t("header.mode.local") : t("header.mode.cloud")}
                    </Badge>
                  </span>
                  <span className="flex items-center gap-2 text-xs text-muted-foreground">
                    <span>{date.format(new Date(m.startedAtMs))}</span>
                    <span aria-hidden>·</span>
                    <span className="tabular-nums">{formatDuration(durationOf(m))}</span>
                    <span aria-hidden>·</span>
                    <span>{t("history.segments", { count: m.segmentCount })}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>

        <footer className="flex items-center justify-between gap-2 border-t px-3 py-2 text-xs text-muted-foreground">
          <span>{t("history.range", { from, to, total })}</span>
          <span className="flex gap-1">
            <Button
              size="xs"
              variant="outline"
              disabled={page === 0 || loading}
              onClick={() => {
                setLoading(true);
                setPage((p) => Math.max(0, p - 1));
              }}
            >
              {t("history.previous")}
            </Button>
            <Button
              size="xs"
              variant="outline"
              disabled={to >= total || loading}
              onClick={() => {
                setLoading(true);
                setPage((p) => p + 1);
              }}
            >
              {t("history.next")}
            </Button>
          </span>
        </footer>
      </section>

      <section className="flex min-h-0 flex-col rounded-2xl border bg-card shadow-xs">
        {detailLoading ? (
          <p className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden /> {t("history.loading")}
          </p>
        ) : !meeting || !details ? (
          <p className="flex flex-1 items-center justify-center px-6 text-center text-sm text-muted-foreground">
            {t("history.select")}
          </p>
        ) : (
          <>
            <header className="flex flex-col gap-1 border-b px-4 py-3">
              <div className="flex items-center gap-2">
                {renaming ? (
                  <div className="flex min-w-0 flex-1 items-center gap-1">
                    <Input
                      autoFocus
                      value={nameDraft}
                      maxLength={200}
                      aria-label={t("history.rename")}
                      onChange={(e) => setNameDraft(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") void saveName();
                        if (e.key === "Escape") setRenaming(false);
                      }}
                    />
                    <Button size="icon-sm" variant="ghost" aria-label={t("common.save")} onClick={() => void saveName()}>
                      <Check />
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("common.cancel")}
                      onClick={() => setRenaming(false)}
                    >
                      <X />
                    </Button>
                  </div>
                ) : (
                  <>
                    <h2 className="min-w-0 flex-1 truncate text-lg font-semibold">
                      {meeting.name || t("history.untitled")}
                    </h2>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("history.rename")}
                      title={t("history.rename")}
                      onClick={() => setRenaming(true)}
                    >
                      <Pencil />
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      className="text-destructive hover:text-destructive"
                      aria-label={t("history.delete")}
                      title={t("history.delete")}
                      onClick={() => void remove()}
                    >
                      <Trash2 />
                    </Button>
                  </>
                )}
              </div>
              <p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <Badge variant="outline">
                  {meeting.sttEngine === "local" ? t("header.mode.local") : t("header.mode.cloud")} ·{" "}
                  {meeting.sttModel}
                </Badge>
                <span className="inline-flex items-center gap-1">
                  <Clock className="size-3" aria-hidden />
                  {date.format(new Date(meeting.startedAtMs))}
                </span>
                <span>
                  {t("history.duration", { duration: formatDuration(durationOf(meeting)) })}
                </span>
              </p>
              {(meeting.goal || meeting.notes) && (
                <p className="line-clamp-2 text-xs text-muted-foreground">
                  {[meeting.goal, meeting.notes].filter(Boolean).join(" · ")}
                </p>
              )}
            </header>

            <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
              {summary ? (
                <div className="p-2">
                  <SummaryPanel
                    summary={summary}
                    onChange={(summary) => setDetails((d) => (d ? { ...d, summary } : d))}
                    onRegenerate={(language) => generateSummary(meeting.id, language)}
                  />
                </div>
              ) : (
                <div className="flex items-center justify-between gap-3 px-2 py-2 text-xs text-muted-foreground">
                  {pendingId === meeting.id ? (
                    <span className="flex items-center gap-2">
                      <Loader2 className="size-3.5 animate-spin" aria-hidden />
                      {t("summary.generating")}
                    </span>
                  ) : (
                    <>
                      <span>{t("summary.missing")}</span>
                      <Button
                        size="xs"
                        variant="outline"
                        disabled={generatingSummary}
                        onClick={() => void createSummary()}
                      >
                        {generatingSummary && <Loader2 className="animate-spin" />}
                        {t("summary.generate")}
                      </Button>
                    </>
                  )}
                </div>
              )}
              {details.segments.length === 0 ? (
                <p className="py-10 text-center text-sm text-muted-foreground">
                  {t("history.noSegments")}
                </p>
              ) : (
                <ol className="flex flex-col">
                  {details.segments.map((segment) => {
                    const translation = translations.get(segment.sessionId);
                    return (
                      <li
                        key={segment.id}
                        className="grid grid-cols-[2.75rem_6rem_1fr] items-baseline gap-2 rounded-lg px-2 py-1.5 text-sm hover:bg-muted/60"
                      >
                        <span className="font-mono text-xs text-muted-foreground tabular-nums">
                          {formatDuration(segment.startMs)}
                        </span>
                        <span
                          className={cn(
                            "max-w-full justify-self-start truncate rounded-md px-1.5 py-px text-xs font-medium",
                            speakerColor(segment.channel, segment.speaker),
                          )}
                        >
                          {segment.channel === "me"
                            ? t("channel.me")
                            : segment.speaker === null
                              ? t("channel.them")
                              : (speakerNames.get(segment.speaker) ??
                                t("speakers.label", { n: segment.speaker + 1 }))}
                        </span>
                        <span className="leading-relaxed select-text">
                          {segment.text}
                          {translation && (
                            <span className="mt-0.5 block text-muted-foreground">
                              {translation.text}
                            </span>
                          )}
                        </span>
                      </li>
                    );
                  })}
                </ol>
              )}
            </div>

            {details.assists.length > 0 && (
              <div className="border-t px-4 py-3">
                <h3 className="mb-2 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
                  {t("history.assists")}
                </h3>
                <ul className="flex max-h-40 flex-col gap-1 overflow-y-auto">
                  {details.assists.map((assist) => (
                    <li key={assist.id} className="flex gap-2 text-sm">
                      <span className="shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
                        {formatDuration(assist.atMs)}
                      </span>
                      <Badge variant="secondary" className="self-start">
                        {assist.kind === "say" ? t("assist.say") : t("assist.ask")}
                      </Badge>
                      <span className="min-w-0 flex-1">{assist.text}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </>
        )}
      </section>
    </div>
  );
}

function durationOf(m: { startedAtMs: number; endedAtMs: number | null }) {
  return m.endedAtMs === null ? 0 : Math.max(0, m.endedAtMs - m.startedAtMs);
}
