import type { TFunction } from "i18next";

import { formatDuration } from "@/lib/hooks";
import type { Contact, MeetingDetails, OutgoingEmail, Summary } from "@/lib/ipc";
import { findContact, nameKey } from "@/stores/contacts";

/** Partecipante mostrato nell'email: nome e, se in rubrica, ruolo. */
export interface EmailParticipant {
  name: string;
  role: string;
}

export interface SummaryEmailInput {
  details: MeetingDetails;
  summary: Summary;
  contacts: Contact[];
  /** Messaggio personale in testa all'email (facoltativo). */
  note: string;
  /** Nome del mittente (impostazioni), mostrato tra i partecipanti. */
  senderName: string;
  t: TFunction;
  locale: string;
}

/** Nomi delle persone del meeting: partecipanti attesi, poi parlanti nominati. */
export function meetingPeopleNames(details: MeetingDetails): string[] {
  return [...details.meeting.participants, ...details.speakers.map((s) => s.name)];
}

/** Partecipanti con un nome (attesi e parlanti nominati), con ruolo dalla rubrica. */
export function meetingParticipants(details: MeetingDetails, contacts: Contact[]): EmailParticipant[] {
  const seen = new Set<string>();
  const out: EmailParticipant[] = [];
  for (const name of meetingPeopleNames(details)) {
    const contact = findContact(contacts, name);
    const display = contact?.name ?? name;
    if (seen.has(nameKey(display))) continue;
    seen.add(nameKey(display));
    out.push({ name: display, role: contact?.role ?? "" });
  }
  return out;
}

/** Oggetto proposto per il riepilogo. */
export function summarySubject(details: MeetingDetails, t: TFunction): string {
  return t("email.subject", { name: details.meeting.name || t("history.untitled") });
}

// Palette dell'email: colori fissi (i client di posta ignorano le variabili
// CSS e spesso il tema scuro), contrasto AA su bianco.
const C = {
  page: "#f4f4f5",
  card: "#ffffff",
  border: "#e4e4e7",
  text: "#18181b",
  muted: "#52525b",
  subtle: "#71717a",
  accent: "#c2531f",
  accentSoft: "#fdf1ea",
};
const FONT = "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";

/**
 * Email del riepilogo: HTML a tabelle con stili in linea (resa uniforme in
 * Gmail, Outlook, Mail) più la versione solo testo.
 */
export function summaryEmail(
  input: SummaryEmailInput,
  to: string[],
  subject: string,
): OutgoingEmail {
  const { details, summary, t, locale } = input;
  const { meeting } = details;
  const title = meeting.name || t("history.untitled");
  const date = new Intl.DateTimeFormat(locale, { dateStyle: "full", timeStyle: "short" }).format(
    new Date(meeting.startedAtMs),
  );
  const duration =
    meeting.endedAtMs === null ? 0 : Math.max(0, meeting.endedAtMs - meeting.startedAtMs);
  const participants = meetingParticipants(details, input.contacts);
  const sender = input.senderName.trim();
  const note = input.note.trim();
  const meta = `${date} · ${t("email.duration", { duration: formatDuration(duration) })}`;

  // --- HTML -----------------------------------------------------------------
  const section = (heading: string, body: string) => `
<tr><td style="padding:24px 32px 0 32px;">
  <h2 style="margin:0 0 12px 0;font-size:13px;line-height:18px;font-weight:700;letter-spacing:0.06em;text-transform:uppercase;color:${C.accent};">${esc(heading)}</h2>
  ${body}
</td></tr>`;

  const rows: string[] = [];

  if (note) {
    rows.push(`
<tr><td style="padding:24px 32px 0 32px;">
  <p style="margin:0;font-size:15px;line-height:24px;color:${C.text};">${paragraphs(note)}</p>
</td></tr>`);
  }

  const people: string[] = [];
  if (sender) people.push(person(sender, ""));
  for (const p of participants) people.push(person(p.name, p.role));
  const infoRows: string[] = [];
  if (people.length > 0) infoRows.push(infoRow(t("email.participants"), people.join("")));
  if (meeting.goal) infoRows.push(infoRow(t("email.goal"), esc(meeting.goal)));
  if (infoRows.length > 0) {
    rows.push(`
<tr><td style="padding:24px 32px 0 32px;">
  <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="border-collapse:collapse;background:${C.page};border-radius:8px;">
    ${infoRows.join("")}
  </table>
</td></tr>`);
  }

  if (summary.overview) {
    rows.push(
      section(
        t("email.overview"),
        `<p style="margin:0;font-size:15px;line-height:24px;color:${C.text};">${paragraphs(summary.overview)}</p>`,
      ),
    );
  }

  if (summary.keyPoints.length > 0) {
    const topics = summary.keyPoints
      .map(
        (topic) => `
<p style="margin:0 0 6px 0;font-size:15px;line-height:22px;font-weight:600;color:${C.text};">${esc(topic.topic)}</p>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="margin:0 0 14px 0;">
  ${topic.points
    .map(
      (point) => `<tr>
    <td width="18" valign="top" style="padding:0 0 6px 0;font-size:15px;line-height:22px;color:${C.accent};">&bull;</td>
    <td valign="top" style="padding:0 0 6px 0;font-size:15px;line-height:22px;color:${C.muted};">${esc(point)}</td>
  </tr>`,
    )
    .join("")}
</table>`,
      )
      .join("");
    rows.push(section(t("email.keyPoints"), topics));
  }

  if (summary.actionItems.length > 0) {
    const items = summary.actionItems
      .map(
        (item) => `<tr>
    <td width="28" valign="top" style="padding:8px 0 8px 12px;font-size:16px;line-height:22px;color:${item.done ? C.subtle : C.accent};">${item.done ? "&#9745;" : "&#9744;"}</td>
    <td valign="top" style="padding:8px 12px 8px 4px;font-size:15px;line-height:22px;color:${item.done ? C.subtle : C.text};${item.done ? "text-decoration:line-through;" : ""}">${esc(item.text)}</td>
  </tr>`,
      )
      .join(`<tr><td colspan="2" style="border-top:1px solid ${C.border};font-size:0;line-height:0;">&nbsp;</td></tr>`);
    rows.push(
      section(
        t("email.actions"),
        `<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="border-collapse:separate;border:1px solid ${C.border};border-radius:8px;">${items}</table>`,
      ),
    );
  }

  const preheader = (note || summary.overview).replace(/\s+/g, " ").slice(0, 140);
  const html = `<!doctype html>
<html lang="${esc(summary.language)}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light">
<meta name="supported-color-schemes" content="light">
<title>${esc(subject)}</title>
</head>
<body style="margin:0;padding:0;background:${C.page};-webkit-text-size-adjust:100%;">
<div style="display:none;max-height:0;overflow:hidden;opacity:0;color:transparent;">${esc(preheader)}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="background:${C.page};">
<tr><td align="center" style="padding:32px 12px;">
  <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="max-width:600px;background:${C.card};border:1px solid ${C.border};border-radius:12px;font-family:${FONT};">
    <tr><td style="height:6px;background:${C.accent};border-radius:12px 12px 0 0;font-size:0;line-height:0;">&nbsp;</td></tr>
    <tr><td style="padding:28px 32px 0 32px;">
      <p style="margin:0 0 8px 0;font-size:12px;line-height:16px;font-weight:700;letter-spacing:0.08em;text-transform:uppercase;color:${C.accent};">${esc(t("email.heading"))}</p>
      <h1 style="margin:0 0 8px 0;font-size:24px;line-height:30px;font-weight:700;color:${C.text};">${esc(title)}</h1>
      <p style="margin:0;font-size:14px;line-height:20px;color:${C.subtle};">${esc(meta)}</p>
    </td></tr>
    ${rows.join("")}
    <tr><td style="padding:32px 32px 28px 32px;">
      <p style="margin:0;padding-top:16px;border-top:1px solid ${C.border};font-size:12px;line-height:18px;color:${C.subtle};">${esc(t("email.footer"))}</p>
    </td></tr>
  </table>
</td></tr>
</table>
</body>
</html>`;

  // --- Testo semplice ---------------------------------------------------------
  const lines: string[] = [title, meta, ""];
  if (note) lines.push(note, "");
  const names = [
    ...(sender ? [sender] : []),
    ...participants.map((p) => (p.role ? `${p.name} (${p.role})` : p.name)),
  ];
  if (names.length > 0) lines.push(`${t("email.participants")}: ${names.join(", ")}`);
  if (meeting.goal) lines.push(`${t("email.goal")}: ${meeting.goal}`);
  if (names.length > 0 || meeting.goal) lines.push("");
  if (summary.overview) lines.push(t("email.overview").toUpperCase(), summary.overview, "");
  if (summary.keyPoints.length > 0) {
    lines.push(t("email.keyPoints").toUpperCase());
    for (const topic of summary.keyPoints) {
      lines.push("", topic.topic, ...topic.points.map((p) => `  • ${p}`));
    }
    lines.push("");
  }
  if (summary.actionItems.length > 0) {
    lines.push(t("email.actions").toUpperCase());
    for (const item of summary.actionItems) lines.push(`[${item.done ? "x" : " "}] ${item.text}`);
    lines.push("");
  }
  lines.push("—", t("email.footer"));

  return { to, subject, html, text: `${lines.join("\n")}\n` };

  function person(name: string, role: string) {
    return `<span style="display:inline-block;margin:0 6px 6px 0;padding:3px 10px;border-radius:999px;background:${C.card};border:1px solid ${C.border};font-size:13px;line-height:20px;color:${C.text};">${esc(name)}${
      role ? `<span style="color:${C.subtle};"> · ${esc(role)}</span>` : ""
    }</span>`;
  }

  function infoRow(label: string, value: string) {
    return `<tr>
      <td valign="top" width="110" style="padding:12px 0 6px 16px;font-size:13px;line-height:20px;font-weight:600;color:${C.muted};">${esc(label)}</td>
      <td valign="top" style="padding:12px 16px 6px 8px;font-size:14px;line-height:20px;color:${C.text};">${value}</td>
    </tr>`;
  }
}

/** Testo sicuro dentro l'HTML. */
function esc(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Paragrafi separati da righe vuote; a capo singolo = `<br>`. */
function paragraphs(text: string): string {
  return text
    .trim()
    .split(/\n\s*\n/)
    .map((p) => esc(p.trim()).replace(/\n/g, "<br>"))
    .join(`<br><br>`);
}

/** Indirizzi da un campo libero (virgole, punti e virgola, spazi, a capo). */
export function parseAddresses(input: string): string[] {
  return input
    .split(/[\s,;]+/)
    .map((a) => a.trim().replace(/^<|>$/g, ""))
    .filter(Boolean);
}
