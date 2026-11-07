import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { ChannelId } from "@/lib/ipc";
import { useSpeakerName } from "@/lib/speakers";
import { cn } from "@/lib/utils";
import { MEETING_LIMITS, useMeetingStore } from "@/stores/meeting";

/** Colori dei parlanti di "Loro", per indice (si ripetono oltre l'ottavo). */
const SPEAKER_COLORS = [
  "bg-them/12 text-them",
  "bg-amber-500/14 text-amber-700 dark:text-amber-400",
  "bg-violet-500/12 text-violet-700 dark:text-violet-400",
  "bg-rose-500/12 text-rose-700 dark:text-rose-400",
  "bg-teal-500/12 text-teal-700 dark:text-teal-400",
  "bg-lime-500/14 text-lime-700 dark:text-lime-400",
  "bg-fuchsia-500/12 text-fuchsia-700 dark:text-fuchsia-400",
  "bg-orange-500/14 text-orange-700 dark:text-orange-400",
];

/** Etichetta del parlante; per "Loro" con parlante noto si rinomina con un clic. */
export function SpeakerLabel(props: { channel: ChannelId; speaker: number | null; editable?: boolean }) {
  const { t } = useTranslation();
  const name = useSpeakerName();
  const rename = useMeetingStore((s) => s.renameSpeaker);
  const [editing, setEditing] = useState(false);
  const { channel, speaker } = props;
  const label = name(channel, speaker);
  const color =
    channel === "me"
      ? "bg-me/12 text-me"
      : speaker === null
        ? "bg-them/12 text-them"
        : SPEAKER_COLORS[speaker % SPEAKER_COLORS.length];
  const base = "justify-self-start truncate rounded-md px-1.5 py-px text-xs font-medium max-w-full";

  if (channel === "me" || speaker === null || !props.editable) {
    return <span className={cn(base, color)}>{label}</span>;
  }
  if (editing) {
    const commit = (value: string) => {
      rename(speaker, value);
      setEditing(false);
    };
    return (
      <input
        autoFocus
        defaultValue={label}
        maxLength={MEETING_LIMITS.speakerName}
        aria-label={t("speakers.rename", { name: label })}
        className={cn(base, color, "w-full outline-none ring-1 ring-current")}
        onFocus={(e) => e.currentTarget.select()}
        onBlur={(e) => commit(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit(e.currentTarget.value);
          if (e.key === "Escape") setEditing(false);
        }}
      />
    );
  }
  return (
    <button
      type="button"
      className={cn(base, color, "cursor-text hover:ring-1 hover:ring-current/40")}
      title={t("speakers.renameHint")}
      aria-label={t("speakers.rename", { name: label })}
      onClick={() => setEditing(true)}
    >
      {label}
    </button>
  );
}
