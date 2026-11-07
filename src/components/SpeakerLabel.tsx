import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { ChannelId } from "@/lib/ipc";
import { speakerColor, useSpeakerName } from "@/lib/speakers";
import { cn } from "@/lib/utils";
import { MEETING_LIMITS, useMeetingStore } from "@/stores/meeting";

/** Etichetta del parlante; per "Loro" con parlante noto si rinomina con un clic. */
export function SpeakerLabel(props: { channel: ChannelId; speaker: number | null; editable?: boolean }) {
  const { t } = useTranslation();
  const name = useSpeakerName();
  const rename = useMeetingStore((s) => s.renameSpeaker);
  const [editing, setEditing] = useState(false);
  const { channel, speaker } = props;
  const label = name(channel, speaker);
  const color = speakerColor(channel, speaker);
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
