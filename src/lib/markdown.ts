import type { TFunction } from "i18next";

import { formatDuration } from "@/lib/hooks";
import type { MeetingDetails, SavedSegment, Summary } from "@/lib/ipc";

/** Riepilogo finale in Markdown (copia e export). */
export function summaryMarkdown(summary: Summary, t: TFunction): string {
  const lines: string[] = [`## ${t("summary.title")}`];
  if (summary.overview) lines.push("", summary.overview);
  if (summary.keyPoints.length > 0) {
    lines.push("", `### ${t("summary.keyPoints")}`);
    for (const topic of summary.keyPoints) {
      lines.push("", `**${inline(topic.topic)}**`);
      for (const point of topic.points) lines.push(`- ${inline(point)}`);
    }
  }
  if (summary.actionItems.length > 0) {
    lines.push("", `### ${t("summary.actions")}`);
    for (const item of summary.actionItems) {
      lines.push(`- [${item.done ? "x" : " "}] ${inline(item.text)}`);
    }
  }
  return lines.join("\n");
}

/** Meeting completo in Markdown: intestazione, riepilogo, trascrizione, suggerimenti. */
export function meetingMarkdown(
  details: MeetingDetails,
  summary: Summary | null,
  t: TFunction,
  locale: string,
): string {
  const { meeting } = details;
  const date = new Intl.DateTimeFormat(locale, { dateStyle: "long", timeStyle: "short" });
  const duration =
    meeting.endedAtMs === null ? 0 : Math.max(0, meeting.endedAtMs - meeting.startedAtMs);
  const names = new Map(details.speakers.map((s) => [s.speaker, s.name]));
  const translations = new Map(details.translations.map((tr) => [tr.sessionId, tr.text]));
  const speakerOf = (segment: SavedSegment) =>
    segment.channel === "me"
      ? t("channel.me")
      : segment.speaker === null
        ? t("channel.them")
        : (names.get(segment.speaker) ?? t("speakers.label", { n: segment.speaker + 1 }));

  const lines: string[] = [`# ${inline(meeting.name || t("history.untitled"))}`, ""];
  lines.push(`- **${t("export.date")}:** ${date.format(new Date(meeting.startedAtMs))}`);
  lines.push(`- **${t("export.duration")}:** ${formatDuration(duration)}`);
  lines.push(
    `- **${t("export.engine")}:** ${
      meeting.sttEngine === "local" ? t("header.mode.local") : t("header.mode.cloud")
    } · ${meeting.sttModel}`,
  );
  const participants = [...new Set(details.segments.map(speakerOf))];
  if (participants.length > 0) {
    lines.push(`- **${t("export.participants")}:** ${participants.map(inline).join(", ")}`);
  }
  if (meeting.goal) lines.push(`- **${t("export.goal")}:** ${inline(meeting.goal)}`);
  if (meeting.notes) lines.push("", `**${t("export.notes")}**`, "", meeting.notes.trim());

  if (summary) lines.push("", summaryMarkdown(summary, t));

  lines.push("", `## ${t("export.transcript")}`);
  if (details.segments.length === 0) {
    lines.push("", `_${t("history.noSegments")}_`);
  }
  for (const segment of details.segments) {
    lines.push(
      "",
      `**[${formatDuration(segment.startMs)}] ${inline(speakerOf(segment))}:** ${inline(segment.text)}`,
    );
    const translation = translations.get(segment.sessionId);
    if (translation) lines.push(`> ${inline(translation)}`);
  }

  if (details.assists.length > 0) {
    lines.push("", `## ${t("history.assists")}`, "");
    for (const assist of details.assists) {
      const kind = assist.kind === "say" ? t("assist.say") : t("assist.ask");
      lines.push(`- [${formatDuration(assist.atMs)}] **${kind}:** ${inline(assist.text)}`);
    }
  }

  return `${lines.join("\n")}\n`;
}

/** Testo su una riga: gli a capo romperebbero elenchi e citazioni. */
function inline(text: string): string {
  return text.replace(/\s*\n+\s*/g, " ").trim();
}
