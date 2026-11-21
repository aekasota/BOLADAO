import React, { useEffect, useRef } from "react";
import { motion } from "motion/react";
import * as PopoverPrimitive from "@radix-ui/react-popover";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import * as SliderPrimitive from "@radix-ui/react-slider";
import { Globe, FolderOpen } from "lucide-react";
import { getBrandSvgPath } from "../lib/format";
import { JobState } from "../lib/ipc";
import { useSettingsStore } from "../stores/settingsStore";

export const SPRING_UI = { type: "spring" as const, stiffness: 420, damping: 34 };
export const SPRING_PANEL = { type: "spring" as const, stiffness: 300, damping: 30 };

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost" | "warn";
  size?: "sm" | "md" | "lg";
}

export const Button: React.FC<ButtonProps> = ({
  variant = "secondary",
  size = "md",
  style,
  children,
  disabled,
  ...rest
}) => {
  const heights = { sm: 28, md: 36, lg: 44 };
  const paddings = { sm: "0 10px", md: "0 14px", lg: "0 20px" };
  const fontSizes = { sm: 12, md: 13, lg: 14 };

  const palette: Record<string, React.CSSProperties> = {
    primary: {
      background: "var(--accent)",
      color: "var(--on-accent)",
      border: "1px solid var(--accent)",
      fontVariationSettings: '"wdth" 112, "wght" 760',
    },
    secondary: {
      background: "var(--surface-2)",
      color: "var(--text)",
      border: "1px solid var(--line)",
      fontVariationSettings: '"wdth" 100, "wght" 560',
    },
    ghost: {
      background: "transparent",
      color: "var(--text-2)",
      border: "1px solid transparent",
      fontVariationSettings: '"wdth" 100, "wght" 500',
    },
    warn: {
      background: "transparent",
      color: "var(--warn)",
      border: "1px solid var(--warn)",
      fontVariationSettings: '"wdth" 105, "wght" 650',
    },
  };

  return (
    <button
      disabled={disabled}
      style={{
        height: heights[size],
        padding: paddings[size],
        fontSize: fontSizes[size],
        borderRadius: 8,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        gap: 6,
        cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.45 : 1,
        fontFamily: "var(--font-sans)",
        whiteSpace: "nowrap",
        transition: "transform var(--dur-micro) var(--ease-out), border-color var(--dur-micro) var(--ease-out)",
        ...palette[variant],
        ...style,
      }}
      {...rest}
    >
      {children}
    </button>
  );
};

interface ChipProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  active?: boolean;
  accentDot?: string;
}

export const Chip: React.FC<ChipProps> = ({ active, accentDot, children, style, ...rest }) => (
  <button
    type="button"
    style={{
      height: 30,
      padding: "0 12px",
      borderRadius: "var(--radius-pill)",
      border: active ? "1px solid var(--accent)" : "1px solid var(--line)",
      background: active ? "var(--accent)" : "var(--surface-2)",
      color: active ? "var(--on-accent)" : "var(--text)",
      fontSize: 12,
      fontFamily: "var(--font-sans)",
      fontVariationSettings: active ? '"wdth" 108, "wght" 720' : '"wdth" 100, "wght" 520',
      display: "inline-flex",
      alignItems: "center",
      gap: 6,
      cursor: rest.disabled ? "not-allowed" : "pointer",
      opacity: rest.disabled ? 0.45 : 1,
      whiteSpace: "nowrap",
      ...style,
    }}
    {...rest}
  >
    {accentDot && (
      <span
        style={{
          width: 7,
          height: 7,
          borderRadius: "50%",
          background: active ? "var(--on-accent)" : accentDot,
          flexShrink: 0,
        }}
      />
    )}
    {children}
  </button>
);

export function Segmented<T extends string>({
  id,
  options,
  value,
  onChange,
}: {
  id: string;
  options: { value: T; label: React.ReactNode }[];
  value: T;
  onChange: (v: T) => void;
}) {
  const reduceMotion = useSettingsStore((s) => s.settings.reduce_motion);
  return (
    <div
      role="tablist"
      style={{
        display: "inline-flex",
        background: "var(--surface-2)",
        border: "1px solid var(--line)",
        borderRadius: 8,
        padding: 3,
        gap: 2,
      }}
    >
      {options.map((opt) => {
        const selected = opt.value === value;
        return (
          <button
            key={opt.value}
            role="tab"
            aria-selected={selected}
            type="button"
            onClick={() => onChange(opt.value)}
            style={{
              position: "relative",
              height: 28,
              padding: "0 12px",
              border: "none",
              borderRadius: 6,
              background: "transparent",
              color: selected ? "var(--on-accent)" : "var(--text-2)",
              fontFamily: "var(--font-sans)",
              fontSize: 13,
              fontVariationSettings: selected ? '"wdth" 110, "wght" 740' : '"wdth" 100, "wght" 520',
              cursor: "pointer",
              zIndex: 1,
              display: "inline-flex",
              alignItems: "center",
              gap: 6,
            }}
          >
            {selected && (
              <motion.span
                layoutId={`seg-${id}`}
                transition={reduceMotion ? { duration: 0 } : SPRING_UI}
                style={{
                  position: "absolute",
                  inset: 0,
                  borderRadius: 6,
                  background: "var(--accent)",
                  zIndex: -1,
                }}
              />
            )}
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}

export const Toggle: React.FC<{
  checked: boolean;
  onChange: (next: boolean) => void;
  label?: string;
  disabled?: boolean;
}> = ({ checked, onChange, label, disabled }) => (
  <label
    style={{
      display: "inline-flex",
      alignItems: "center",
      gap: 8,
      cursor: disabled ? "not-allowed" : "pointer",
      opacity: disabled ? 0.5 : 1,
      fontSize: 13,
      color: "var(--text)",
    }}
  >
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => !disabled && onChange(!checked)}
      style={{
        width: 36,
        height: 20,
        borderRadius: 999,
        padding: 2,
        border: checked ? "1px solid var(--accent)" : "1px solid var(--line)",
        background: checked ? "var(--accent)" : "var(--surface-2)",
        display: "flex",
        alignItems: "center",
        justifyContent: checked ? "flex-end" : "flex-start",
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >
      <span
        style={{
          width: 14,
          height: 14,
          borderRadius: "50%",
          background: checked ? "var(--on-accent)" : "var(--text-2)",
        }}
      />
    </button>
    {label && <span>{label}</span>}
  </label>
);

export const Slider: React.FC<{
  value: number[];
  min: number;
  max: number;
  step?: number;
  onValueChange: (val: number[]) => void;
  ariaLabel?: string;
}> = ({ value, min, max, step = 1, onValueChange, ariaLabel }) => (
  <SliderPrimitive.Root
    value={value}
    min={min}
    max={max}
    step={step}
    onValueChange={onValueChange}
    aria-label={ariaLabel}
    style={{
      position: "relative",
      display: "flex",
      alignItems: "center",
      userSelect: "none",
      touchAction: "none",
      width: "100%",
      height: 20,
    }}
  >
    <SliderPrimitive.Track
      style={{
        background: "var(--surface-2)",
        border: "1px solid var(--line)",
        position: "relative",
        flexGrow: 1,
        borderRadius: 999,
        height: 6,
      }}
    >
      <SliderPrimitive.Range
        style={{
          position: "absolute",
          background: "var(--accent)",
          borderRadius: 999,
          height: "100%",
        }}
      />
    </SliderPrimitive.Track>
    {value.map((_, i) => (
      <SliderPrimitive.Thumb
        key={i}
        style={{
          display: "block",
          width: 16,
          height: 16,
          background: "var(--text)",
          border: "2px solid var(--bg)",
          borderRadius: "50%",
          cursor: "grab",
        }}
      />
    ))}
  </SliderPrimitive.Root>
);

/**
 * Op-Art Diagonal Striped Progress Bar (Section 14.3):
 * Stripes shift smoothly with velocity proportional to `speedBps`, freeze when paused, and turn amber (`--warn`) on failure!
 */
export const StripeProgress: React.FC<{
  pct: number;
  state: JobState;
  speedBps: number;
  height?: number;
}> = ({ pct, state, speedBps, height = 10 }) => {
  const fillRef = useRef<HTMLDivElement | null>(null);
  const reduceMotion = useSettingsStore((s) => s.settings.reduce_motion);

  const isFailed = state === "failed";
  const isPaused = state === "paused" || state === "canceled";
  const isMoving =
    !reduceMotion &&
    (state === "downloading" || state === "postprocessing" || state === "workshop");

  useEffect(() => {
    if (!isMoving) return;
    let raf = 0;
    let offset = 0;
    let prev = performance.now();

    const tick = (now: number) => {
      const dt = Math.min(0.05, (now - prev) / 1000);
      prev = now;
      const mbs = speedBps / (1024 * 1024);
      const pxPerSec = state === "downloading" ? 18 + Math.log1p(mbs) * 28 : 24;
      offset = (offset + pxPerSec * dt) % 28;
      if (fillRef.current) {
        fillRef.current.style.backgroundPosition = `${offset.toFixed(1)}px 0px`;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [isMoving, speedBps, state]);

  const clampedPct = Math.max(0, Math.min(100, (isFailed ? Math.max(pct, 0.12) : pct) * 100));

  const fillBg = isFailed
    ? "var(--warn)"
    : state === "done"
    ? "var(--ok)"
    : isPaused
    ? "var(--text-3)"
    : `repeating-linear-gradient(
        -45deg,
        var(--stripe-fg) 0px,
        var(--stripe-fg) 9px,
        var(--stripe-bg) 9px,
        var(--stripe-bg) 16px
      )`;

  return (
    <div
      style={{
        width: "100%",
        height,
        borderRadius: 999,
        background: "var(--surface-2)",
        border: "1px solid var(--line)",
        overflow: "hidden",
        position: "relative",
      }}
    >
      <div
        ref={fillRef}
        style={{
          width: `${clampedPct.toFixed(1)}%`,
          height: "100%",
          background: fillBg,
          backgroundSize: "22.6px 22.6px",
          transition: "width 160ms linear",
        }}
      />
    </div>
  );
};

export const PlatformIcon: React.FC<{
  slug: string;
  color?: string;
  size?: number;
}> = ({ slug, color = "currentColor", size = 15 }) => {
  const path = getBrandSvgPath(slug);
  if (!path) {
    return <Globe size={size} color={color} />;
  }
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={color}
      aria-hidden="true"
      style={{ flexShrink: 0 }}
    >
      <path d={path} />
    </svg>
  );
};

export const PathPreview: React.FC<{
  path: string;
  onClick?: () => void;
}> = ({ path, onClick }) => (
  <button
    type="button"
    onClick={onClick}
    title="Clique para escolher outra pasta de destino"
    style={{
      width: "100%",
      display: "flex",
      alignItems: "center",
      gap: 8,
      padding: "7px 10px",
      background: "var(--surface-2)",
      border: "1px solid var(--line)",
      borderRadius: 8,
      color: "var(--text-2)",
      fontSize: 12,
      cursor: onClick ? "pointer" : "default",
      textAlign: "left",
      overflow: "hidden",
    }}
  >
    <FolderOpen size={14} style={{ flexShrink: 0, color: "var(--accent-text)" }} />
    <span
      className="font-mono-num"
      style={{
        overflow: "hidden",
        textOverflow: "ellipsis",
        whiteSpace: "nowrap",
        flex: 1,
      }}
    >
      {path}
    </span>
    {onClick && (
      <span style={{ color: "var(--text)", fontSize: 11, fontWeight: 600, flexShrink: 0 }}>
        Trocar
      </span>
    )}
  </button>
);

export const Kbd: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <kbd
    className="font-mono-num"
    style={{
      display: "inline-flex",
      alignItems: "center",
      padding: "1px 5px",
      fontSize: 11,
      borderRadius: 4,
      background: "var(--surface-2)",
      border: "1px solid var(--line)",
      color: "var(--text-2)",
    }}
  >
    {children}
  </kbd>
);

export const Popover = PopoverPrimitive;
export const Tooltip = TooltipPrimitive;
