import { cn } from "@/lib/utils";

/** Range visualizzato in dBFS. */
const MIN_DB = -60;
const TICKS = [-60, -48, -36, -24, -12, -6, 0];
/** Verde fino a −12 dB, giallo fino a −3 dB, rosso verso 0 dB. */
const GRADIENT =
  "linear-gradient(to right, #10b981 0%, #10b981 78%, #f59e0b 88%, #f59e0b 94%, #ef4444 100%)";

const toPct = (db: number) => Math.min(100, Math.max(0, ((db - MIN_DB) / -MIN_DB) * 100));

interface LevelMeterProps {
  rmsDb: number;
  holdDb: number;
  active: boolean;
  label: string;
  /** Barra sottile senza scala in dB. */
  compact?: boolean;
}

/** Barra orizzontale RMS + marcatore del picco mantenuto, scala in dBFS. */
export function LevelMeter({ rmsDb, holdDb, active, label, compact = false }: LevelMeterProps) {
  const rms = active ? toPct(rmsDb) : 0;
  const hold = active ? toPct(holdDb) : 0;
  const clipping = active && holdDb > -1;

  return (
    <div className="flex flex-col gap-1.5">
      <div
        role="meter"
        aria-label={label}
        aria-valuemin={MIN_DB}
        aria-valuemax={0}
        aria-valuenow={Math.round(Math.max(rmsDb, MIN_DB))}
        className={cn("relative overflow-hidden bg-muted", compact ? "h-1.5 rounded-full" : "h-4 rounded-sm")}
      >
        {/* Gradiente a tutta larghezza, rivelato fino al livello RMS. */}
        <div
          className="absolute inset-0"
          style={{
            background: GRADIENT,
            clipPath: `inset(0 ${100 - rms}% 0 0)`,
            transition: "clip-path 60ms linear",
          }}
        />
        {hold > 0 && (
          <div
            className={cn(
              "absolute inset-y-0 w-0.5",
              clipping ? "bg-red-500" : "bg-foreground/70",
            )}
            style={{ left: `calc(${hold}% - 1px)` }}
          />
        )}
      </div>
      {!compact && (
        <div className="relative h-3 text-[10px] tabular-nums text-muted-foreground">
          {TICKS.map((db) => (
            <span
              key={db}
              className="absolute -translate-x-1/2 first:translate-x-0 last:-translate-x-full"
              style={{ left: `${toPct(db)}%` }}
            >
              {db}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
