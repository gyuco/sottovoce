import { ChevronDown, Loader2, MessagesSquare, RotateCcw, Send } from "lucide-react";
import { Fragment, useId, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { splitCitations } from "@/lib/citations";
import { formatDuration } from "@/lib/hooks";
import type { HistorySource } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { type HistoryChatLine, useHistoryChatStore } from "@/stores/historyChat";

/**
 * "Chiedi allo storico": domande su tutti i meeting salvati. Le risposte
 * vengono solo dai passaggi recuperati (backend) e citano le fonti `[n]`;
 * un clic su una fonte apre il meeting alla frase giusta.
 */
export function HistoryChat(props: {
  /** Meeting aperto nel dettaglio (per il filtro "solo questo meeting"). */
  selectedMeeting: number | null;
  onOpenSource: (meetingId: number, segmentId: number | null) => void;
}) {
  const { t } = useTranslation();
  const lines = useHistoryChatStore((s) => s.lines);
  const sending = useHistoryChatStore((s) => s.sending);
  const error = useHistoryChatStore((s) => s.error);
  const filters = useHistoryChatStore((s) => s.filters);
  const setFilters = useHistoryChatStore((s) => s.setFilters);
  const ask = useHistoryChatStore((s) => s.ask);
  const reset = useHistoryChatStore((s) => s.reset);
  const [openPanel, setOpenPanel] = useState(false);
  const [text, setText] = useState("");
  const list = useRef<HTMLDivElement>(null);
  const ids = useId();

  useLayoutEffect(() => {
    const el = list.current;
    if (openPanel && el) el.scrollTop = el.scrollHeight;
  }, [openPanel, lines, sending]);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const question = text.trim();
    if (question === "" || sending) return;
    setText("");
    void ask(question, props.selectedMeeting);
  };

  return (
    <section className="rounded-2xl border bg-card shadow-xs">
      <button
        type="button"
        className="flex w-full items-center gap-2 rounded-2xl px-4 py-3 text-left text-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
        aria-expanded={openPanel}
        aria-controls={`${ids}-panel`}
        onClick={() => setOpenPanel((o) => !o)}
      >
        <MessagesSquare className="size-4 shrink-0 text-primary" aria-hidden />
        <span className="font-semibold">{t("historyChat.title")}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">
          {t("historyChat.subtitle")}
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
        <div id={`${ids}-panel`} className="flex flex-col gap-2 border-t px-3 pt-2 pb-3">
          <div
            ref={list}
            className="flex max-h-72 min-h-14 flex-col gap-2 overflow-y-auto overscroll-contain"
            aria-live="polite"
          >
            {lines.length === 0 ? (
              <p className="m-auto max-w-md py-3 text-center text-xs text-muted-foreground">
                {t("historyChat.empty")}
              </p>
            ) : (
              lines.map((line) =>
                line.role === "user" ? (
                  <div
                    key={line.id}
                    className="max-w-[80%] self-end rounded-xl bg-primary/12 px-3 py-2 text-sm leading-relaxed whitespace-pre-wrap select-text"
                  >
                    {line.text}
                  </div>
                ) : (
                  <Answer key={line.id} line={line} onOpenSource={props.onOpenSource} />
                ),
              )
            )}
            {sending && (
              <p className="flex items-center gap-1.5 self-start px-1 text-xs text-muted-foreground">
                <Loader2 className="size-3.5 animate-spin" aria-hidden />
                {t("historyChat.thinking")}
              </p>
            )}
          </div>

          {error && (
            <p role="alert" className="px-1 text-xs text-destructive">
              {error}
            </p>
          )}

          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-1 text-xs text-muted-foreground">
            <label className="flex items-center gap-1.5" htmlFor={`${ids}-from`}>
              {t("historyChat.from")}
            </label>
            <Input
              id={`${ids}-from`}
              type="date"
              value={filters.from}
              max={filters.to || undefined}
              onChange={(e) => setFilters({ from: e.target.value })}
              className="h-7 w-36 text-xs"
            />
            <label className="flex items-center gap-1.5" htmlFor={`${ids}-to`}>
              {t("historyChat.to")}
            </label>
            <Input
              id={`${ids}-to`}
              type="date"
              value={filters.to}
              min={filters.from || undefined}
              onChange={(e) => setFilters({ to: e.target.value })}
              className="h-7 w-36 text-xs"
            />
            <label className="flex items-center gap-1.5">
              <Switch
                size="sm"
                checked={filters.onlySelected}
                disabled={props.selectedMeeting === null}
                onCheckedChange={(onlySelected) => setFilters({ onlySelected })}
              />
              {t("historyChat.onlySelected")}
            </label>
            {lines.length > 0 && (
              <Button
                size="xs"
                variant="ghost"
                className="ml-auto"
                disabled={sending}
                onClick={reset}
              >
                <RotateCcw aria-hidden />
                {t("historyChat.clear")}
              </Button>
            )}
          </div>

          <form onSubmit={submit} className="flex gap-2">
            <Input
              aria-label={t("historyChat.placeholder")}
              placeholder={t("historyChat.placeholder")}
              maxLength={2000}
              value={text}
              onChange={(e) => setText(e.target.value)}
              className="min-w-0"
            />
            <Button
              type="submit"
              size="icon"
              aria-label={t("historyChat.send")}
              title={t("historyChat.send")}
              disabled={sending || text.trim() === ""}
            >
              <Send aria-hidden />
            </Button>
          </form>
        </div>
      )}
    </section>
  );
}

/** Risposta con citazioni cliccabili ed elenco delle fonti. */
function Answer(props: {
  line: HistoryChatLine;
  onOpenSource: (meetingId: number, segmentId: number | null) => void;
}) {
  const { t, i18n } = useTranslation();
  const { line } = props;
  const sources = new Map((line.sources ?? []).map((s) => [s.n, s]));
  const date = new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium" });
  const open = (s: HistorySource) => props.onOpenSource(s.meetingId, s.segmentId);
  const where = (s: HistorySource) =>
    s.kind === "transcript" ? formatDuration(s.startMs) : t(`history.semantic.kind.${s.kind}`);

  return (
    <div className="flex max-w-[92%] flex-col gap-1.5 self-start rounded-xl border bg-muted/40 px-3 py-2 text-sm">
      {line.found === false ? (
        <p className="text-muted-foreground">{t("historyChat.notFound")}</p>
      ) : (
        <p className="leading-relaxed whitespace-pre-wrap select-text">
          {splitCitations(line.text).map((part, i) =>
            "text" in part ? (
              <Fragment key={i}>{part.text}</Fragment>
            ) : (
              <span key={i} className="whitespace-nowrap">
                {part.cites.map((n) => {
                  const s = sources.get(n);
                  return s ? (
                    <button
                      key={n}
                      type="button"
                      onClick={() => open(s)}
                      title={t("historyChat.openSource", {
                        meeting: s.meetingName || t("history.untitled"),
                      })}
                      className="mx-px rounded bg-primary/15 px-1 align-baseline text-[0.7rem] font-semibold text-primary tabular-nums outline-none hover:bg-primary/25 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                    >
                      {n}
                    </button>
                  ) : null;
                })}
              </span>
            ),
          )}
        </p>
      )}

      {line.found !== false && line.sources && line.sources.length > 0 && (
        <div className="flex flex-col gap-1 border-t pt-1.5">
          <span className="text-[0.7rem] font-semibold tracking-wide text-muted-foreground uppercase">
            {t("historyChat.sources")}
          </span>
          {line.sources.map((s) => (
            <button
              key={s.n}
              type="button"
              onClick={() => open(s)}
              className="flex flex-col gap-0.5 rounded-md border-l-2 border-primary/40 py-0.5 pl-2 text-left text-xs outline-none hover:border-primary focus-visible:ring-[3px] focus-visible:ring-ring/50"
            >
              <span className="text-muted-foreground">
                <span className="font-semibold text-primary tabular-nums">[{s.n}]</span>{" "}
                {s.meetingName || t("history.untitled")} · {date.format(new Date(s.startedAtMs))} ·{" "}
                {where(s)}
              </span>
              <span className="line-clamp-2 whitespace-pre-line text-foreground/90">{s.text}</span>
            </button>
          ))}
        </div>
      )}

      {line.semantic === false && (
        <p className="text-xs text-muted-foreground">{t("historyChat.wordsOnly")}</p>
      )}
    </div>
  );
}
