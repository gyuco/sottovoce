import { Check, Clock, FileDown, Loader2, Pencil, Search, Sparkles, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { HistoryChat } from "@/components/HistoryChat";
import { SpeakerCard } from "@/components/SpeakerCard";
import { SummaryPanel } from "@/components/SummaryPanel";
import { ConversationMetrics } from "@/components/TalkRatio";
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
import { Input } from "@/components/ui/input";
import { formatDuration } from "@/lib/hooks";
import {
  type MeetingDetails,
  type MeetingSummary,
  type SearchPassage,
  type SearchResult,
  deleteMeeting,
  errorMessage,
  exportMeetingMarkdown,
  generateSummary,
  getMeeting,
  listMeetings,
  renameMeeting,
  searchMeetings,
  setMeetingSpeaker,
} from "@/lib/ipc";
import { meetingMarkdown } from "@/lib/markdown";
import { speakerColor } from "@/lib/speakers";
import { cn } from "@/lib/utils";
import { useInsightsStore } from "@/stores/insights";
import { semanticReady, useSearchIndexStore } from "@/stores/searchIndex";

const PER_PAGE = 20;

/** Storico: elenco dei meeting salvati e dettaglio in sola lettura. */
export function History(props: { focusId?: number | null }) {
  const { t, i18n } = useTranslation();
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [data, setData] = useState<{ meetings: MeetingSummary[]; total: number } | null>(null);
  // Con una ricerca: meeting con i passaggi trovati (parole + significato).
  const [results, setResults] = useState<SearchResult[] | null>(null);
  // Frase da mostrare nel dettaglio dopo il clic su un passaggio.
  const [focusSegment, setFocusSegment] = useState<number | null>(null);
  // Cambia a ogni apertura: un secondo clic sulla stessa fonte riscorre.
  const [focusTick, setFocusTick] = useState(0);
  // Aperto da un avviso: parte da quel meeting (il componente si rimonta).
  const [selected, setSelected] = useState<number | null>(props.focusId ?? null);
  const [details, setDetails] = useState<MeetingDetails | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [renaming, setRenaming] = useState(false);
  const [nameDraft, setNameDraft] = useState("");
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [generatingSummary, setGeneratingSummary] = useState(false);
  const [exported, setExported] = useState<{ id: number; path: string } | null>(null);
  // Parlante di "Loro" di cui è aperta la scheda.
  const [cardSpeaker, setCardSpeaker] = useState<number | null>(null);
  const lastReady = useInsightsStore((s) => s.lastReady);
  const pendingId = useInsightsStore((s) => s.pendingMeetingId);
  const indexStatus = useSearchIndexStore((s) => s.status);
  const loadIndexStatus = useSearchIndexStore((s) => s.load);
  const semantic = semanticReady(indexStatus);

  useEffect(() => {
    void loadIndexStatus();
  }, [loadIndexStatus]);

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
    const keep = (ids: number[]) =>
      setSelected((current) =>
        current !== null && ids.includes(current) ? current : (ids[0] ?? null),
      );
    const request = query
      ? searchMeetings(query).then((found) => {
          if (!alive) return;
          setResults(found);
          setData({ meetings: found, total: found.length });
          setError(null);
          keep(found.map((m) => m.id));
        })
      : listMeetings(page, PER_PAGE).then((result) => {
          if (!alive) return;
          setResults(null);
          setData(result);
          setError(null);
          keep(result.meetings.map((m) => m.id));
        });
    request
      .catch((e: unknown) => alive && setError(errorMessage(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [page, query, reload]);

  // Porta in vista la frase del passaggio scelto, quando il dettaglio è pronto.
  useEffect(() => {
    if (focusSegment === null || details?.meeting.id !== selected) return;
    document
      .getElementById(`segment-${focusSegment}`)
      ?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [focusSegment, focusTick, details, selected]);

  const openSegment = (meetingId: number, segmentId: number | null) => {
    setSelected(meetingId);
    setFocusSegment(segmentId);
    setFocusTick((n) => n + 1);
  };
  const openPassage = (meetingId: number, passage: SearchPassage | null) =>
    openSegment(meetingId, passage?.segmentId ?? null);

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
  const metricLines = useMemo(
    () =>
      (details?.segments ?? []).map((s) => ({
        channel: s.channel,
        text: s.text,
        start: s.startMs,
        end: s.endMs,
      })),
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
    if (selected === null) return;
    setConfirmingDelete(false);
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

  const saveSpeaker = async (speaker: number, name: string) => {
    if (!details) return;
    const id = details.meeting.id;
    const speakers = await setMeetingSpeaker(id, speaker, name);
    setDetails((d) => (d && d.meeting.id === id ? { ...d, speakers } : d));
  };

  const exportMarkdown = async () => {
    if (!details) return;
    try {
      const contents = meetingMarkdown(details, summary, t, i18n.language);
      const day = new Date(details.meeting.startedAtMs).toISOString().slice(0, 10);
      const name = `${day} ${details.meeting.name || t("history.untitled")}`;
      const path = await exportMeetingMarkdown(name, contents);
      if (path) setExported({ id: details.meeting.id, path });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const total = data?.total ?? 0;
  const from = total === 0 ? 0 : page * PER_PAGE + 1;
  const to = Math.min((page + 1) * PER_PAGE, total);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 px-6 pt-5 pb-6">
      <HistoryChat selectedMeeting={selected} onOpenSource={openSegment} />
      <div className="grid min-h-0 flex-1 gap-4 md:grid-cols-[22rem_minmax(0,1fr)]">
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
              placeholder={semantic ? t("history.searchPlaceholderSemantic") : t("history.searchPlaceholder")}
              aria-label={semantic ? t("history.searchPlaceholderSemantic") : t("history.searchPlaceholder")}
              className="pl-8"
            />
          </div>
          <SemanticHint />
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
          {results && (
            <ul className="flex flex-col gap-1">
              {results.map((m) => (
                <li key={m.id}>
                  <ResultRow
                    result={m}
                    selected={m.id === selected}
                    date={date.format(new Date(m.startedAtMs))}
                    onOpen={(passage) => openPassage(m.id, passage)}
                  />
                </li>
              ))}
            </ul>
          )}
          {!results && (
            <ul className="flex flex-col gap-1">
              {data?.meetings.map((m) => (
                <li key={m.id}>
                  <button
                    type="button"
                    onClick={() => openPassage(m.id, null)}
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
          )}
        </div>

        <footer className="flex items-center justify-between gap-2 border-t px-3 py-2 text-xs text-muted-foreground">
          <span>
            {results ? t("history.results", { count: results.length }) : t("history.range", { from, to, total })}
          </span>
          <span className="flex gap-1">
            <Button
              size="xs"
              variant="outline"
              disabled={page === 0 || loading || !!results}
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
              disabled={to >= total || loading || !!results}
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
                      aria-label={t("history.export")}
                      title={t("history.export")}
                      onClick={() => void exportMarkdown()}
                    >
                      <FileDown />
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      className="text-destructive hover:text-destructive"
                      aria-label={t("history.delete")}
                      title={t("history.delete")}
                      onClick={() => setConfirmingDelete(true)}
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
              {meeting.participants.length > 0 && (
                <p className="truncate text-xs text-muted-foreground">
                  {t("export.participants")}: {meeting.participants.join(", ")}
                </p>
              )}
              {(meeting.goal || meeting.notes) && (
                <p className="line-clamp-2 text-xs text-muted-foreground">
                  {[meeting.goal, meeting.notes].filter(Boolean).join(" · ")}
                </p>
              )}
              {exported?.id === meeting.id && (
                <p
                  role="status"
                  className="truncate text-xs text-muted-foreground"
                  title={exported.path}
                >
                  {t("history.exported", { path: exported.path })}
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
              {details.segments.length > 0 && (
                <ConversationMetrics lines={metricLines} className="mx-2 mb-2 shadow-none" />
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
                        id={`segment-${segment.id}`}
                        className={cn(
                          "grid scroll-mt-4 grid-cols-[2.75rem_6rem_1fr] items-baseline gap-2 rounded-lg px-2 py-1.5 text-sm hover:bg-muted/60",
                          segment.id === focusSegment && "bg-primary/10 ring-1 ring-primary/30",
                        )}
                      >
                        <span className="font-mono text-xs text-muted-foreground tabular-nums">
                          {formatDuration(segment.startMs)}
                        </span>
                        {segment.channel === "them" && segment.speaker !== null ? (
                          <button
                            type="button"
                            className={cn(
                              "max-w-full cursor-pointer justify-self-start truncate rounded-md px-1.5 py-px text-xs font-medium hover:ring-1 hover:ring-current/40",
                              speakerColor(segment.channel, segment.speaker),
                            )}
                            title={t("speakers.renameHint")}
                            onClick={() => setCardSpeaker(segment.speaker)}
                          >
                            {speakerNames.get(segment.speaker) ??
                              t("speakers.label", { n: segment.speaker + 1 })}
                          </button>
                        ) : (
                          <span
                            className={cn(
                              "max-w-full justify-self-start truncate rounded-md px-1.5 py-px text-xs font-medium",
                              speakerColor(segment.channel, segment.speaker),
                            )}
                          >
                            {segment.channel === "me" ? t("channel.me") : t("channel.them")}
                          </span>
                        )}
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

      {details && cardSpeaker !== null && (
        <SpeakerCard
          open
          onOpenChange={(open) => !open && setCardSpeaker(null)}
          name={speakerNames.get(cardSpeaker) ?? ""}
          fallback={t("speakers.label", { n: cardSpeaker + 1 })}
          onSave={(name) => saveSpeaker(cardSpeaker, name)}
          participants={details.meeting.participants}
          taken={details.speakers.filter((s) => s.speaker !== cardSpeaker).map((s) => s.name)}
        />
      )}

      <AlertDialog open={confirmingDelete} onOpenChange={setConfirmingDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("history.deleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("history.deleteConfirm")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className={cn(buttonVariants({ variant: "destructive" }))}
              onClick={() => void remove()}
            >
              {t("history.deleteAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function durationOf(m: { startedAtMs: number; endedAtMs: number | null }) {
  return m.endedAtMs === null ? 0 : Math.max(0, m.endedAtMs - m.startedAtMs);
}

/** Stato della ricerca per significato sotto il campo di ricerca. */
function SemanticHint() {
  const { t } = useTranslation();
  const status = useSearchIndexStore((s) => s.status);
  if (!status || !status.enabled) return null;
  if (!status.modelReady) {
    return <p className="text-xs text-muted-foreground">{t("history.semantic.noModel")}</p>;
  }
  const working = status.pending > 0;
  return (
    <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
      {working ? (
        <Loader2 className="size-3 animate-spin" aria-hidden />
      ) : (
        <Sparkles className="size-3 text-primary" aria-hidden />
      )}
      {working
        ? t("history.semantic.indexing", { indexed: status.indexed, total: status.total })
        : t("history.semantic.ready")}
    </p>
  );
}

const PASSAGE_PREVIEW = 2;

/** Meeting trovato dalla ricerca, con i passaggi più vicini alla query. */
function ResultRow(props: {
  result: SearchResult;
  selected: boolean;
  date: string;
  onOpen: (passage: SearchPassage | null) => void;
}) {
  const { t } = useTranslation();
  const { result } = props;
  return (
    <div
      className={cn(
        "flex flex-col gap-1 rounded-xl px-3 py-2 transition-colors hover:bg-muted/70",
        props.selected && "bg-muted",
      )}
    >
      <button
        type="button"
        onClick={() => props.onOpen(null)}
        aria-current={props.selected ? "true" : undefined}
        className="flex flex-col gap-0.5 rounded-md text-left outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
      >
        <span className="truncate text-sm font-medium">{result.name || t("history.untitled")}</span>
        <span className="text-xs text-muted-foreground">{props.date}</span>
      </button>
      {result.passages.slice(0, PASSAGE_PREVIEW).map((p, i) => (
        <button
          key={`${p.kind}-${p.segmentId ?? i}-${i}`}
          type="button"
          onClick={() => props.onOpen(p)}
          className="flex flex-col gap-0.5 rounded-md border-l-2 border-primary/40 py-0.5 pl-2 text-left text-xs outline-none hover:border-primary focus-visible:ring-[3px] focus-visible:ring-ring/50"
        >
          <span className="text-muted-foreground">
            {p.kind === "transcript"
              ? formatDuration(p.startMs)
              : t(`history.semantic.kind.${p.kind}`)}
          </span>
          <span className="line-clamp-3 whitespace-pre-line text-foreground/90">{p.text}</span>
        </button>
      ))}
      {result.passages.length === 0 && result.lexical && (
        <span className="text-xs text-muted-foreground">{t("history.semantic.wordMatch")}</span>
      )}
    </div>
  );
}
