import { KeyRound } from "lucide-react";
import { useTranslation } from "react-i18next";

import { KeyForm } from "@/components/KeyForm";
import { LlmSettings } from "@/components/LlmSettings";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { Language } from "@/lib/ipc";
import { useAudioStore } from "@/stores/audio";
import { useSettingsStore } from "@/stores/settings";

const LANGUAGES: Language[] = ["multi", "it", "en"];

export function SettingsPanel() {
  const { t } = useTranslation();
  const { settings, hasKey, error, update } = useSettingsStore();
  const systemSupported = useAudioStore((s) => s.systemSupported);

  if (!settings) return null;

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <KeyRound className="size-4 text-muted-foreground" aria-hidden />
            {t("settings.stt.title")}
            <Badge variant={hasKey ? "default" : "secondary"} className="ml-auto font-normal">
              {hasKey ? t("settings.key.saved") : t("settings.key.missing")}
            </Badge>
          </CardTitle>
          <CardDescription>{t("settings.stt.hint")}</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <KeyForm provider={settings.sttProvider} label={t("settings.stt.keyLabel")} />

          <LanguageSelect
            label={t("settings.language.mine")}
            hint={t("settings.language.mineHint")}
            value={settings.myLanguage}
            onChange={(myLanguage) => void update({ myLanguage })}
          />
          <LanguageSelect
            label={t("settings.language.theirs")}
            hint={t("settings.language.theirsHint")}
            value={settings.theirLanguage}
            onChange={(theirLanguage) => void update({ theirLanguage })}
          />

          <label className="flex items-center justify-between gap-4 text-sm">
            <span>
              {t("controls.systemAudio")}
              <span className="block text-xs text-muted-foreground">
                {systemSupported ? t("settings.systemAudioHint") : t("channel.unsupported")}
              </span>
            </span>
            <Switch
              checked={settings.systemAudio && systemSupported}
              disabled={!systemSupported}
              onCheckedChange={(v) => void update({ systemAudio: v })}
            />
          </label>
        </CardContent>
      </Card>

      <LlmSettings />

      {error && (
        <p role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      )}
      <p className="text-xs text-muted-foreground">{t("settings.applyHint")}</p>
    </div>
  );
}

function LanguageSelect(props: {
  label: string;
  hint: string;
  value: Language;
  onChange: (l: Language) => void;
}) {
  const { t } = useTranslation();
  return (
    <label className="flex items-center justify-between gap-4 text-sm">
      <span>
        {props.label}
        <span className="block text-xs text-muted-foreground">{props.hint}</span>
      </span>
      <Select value={props.value} onValueChange={(v) => props.onChange(v as Language)}>
        <SelectTrigger className="w-48">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {LANGUAGES.map((l) => (
            <SelectItem key={l} value={l}>
              {t(`settings.language.${l}`)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </label>
  );
}
