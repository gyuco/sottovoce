/**
 * Marchio di Sottovoce: la stessa onda a cinque voci dell'icona dell'app
 * (`src-tauri/icons/source.svg`), così il logo in interfaccia e l'icona di
 * sistema sono la stessa cosa. Gradienti con id fissi: il logo è reso una volta.
 */
export function Logo({ className, label }: { className?: string; label: string }) {
  return (
    <svg viewBox="0 0 1024 1024" className={className} role="img" aria-label={label}>
      <defs>
        <linearGradient id="logo-bg" x1="180" y1="120" x2="860" y2="910" gradientUnits="userSpaceOnUse">
          <stop stopColor="#272420" />
          <stop offset="0.55" stopColor="#14120F" />
          <stop offset="1" stopColor="#0A0908" />
        </linearGradient>
        <radialGradient
          id="logo-sheen"
          cx="0"
          cy="0"
          r="1"
          gradientUnits="userSpaceOnUse"
          gradientTransform="translate(318 268) rotate(54) scale(600)"
        >
          <stop stopColor="#FFFFFF" stopOpacity="0.13" />
          <stop offset="1" stopColor="#FFFFFF" stopOpacity="0" />
        </radialGradient>
        <linearGradient id="logo-voice" x1="228" y1="512" x2="796" y2="512" gradientUnits="userSpaceOnUse">
          <stop stopColor="#FF8B4A" />
          <stop offset="0.32" stopColor="#F9B27E" />
          <stop offset="0.5" stopColor="#F6EDE3" />
          <stop offset="0.7" stopColor="#B9C6EA" />
          <stop offset="1" stopColor="#5578F0" />
        </linearGradient>
      </defs>

      <rect x="100" y="100" width="824" height="824" rx="186" fill="url(#logo-bg)" />
      <rect x="100" y="100" width="824" height="824" rx="186" fill="url(#logo-sheen)" />
      <rect
        x="102.5"
        y="102.5"
        width="819"
        height="819"
        rx="183.5"
        stroke="#FFFFFF"
        strokeOpacity="0.09"
        strokeWidth="5"
      />
      <g fill="url(#logo-voice)">
        <rect x="201" y="422" width="82" height="180" rx="41" />
        <rect x="336" y="322" width="82" height="380" rx="41" />
        <rect x="471" y="232" width="82" height="560" rx="41" />
        <rect x="606" y="342" width="82" height="340" rx="41" />
        <rect x="741" y="442" width="82" height="140" rx="41" />
      </g>
    </svg>
  );
}
