import React, { useEffect, useState } from "react";
import { Gamepad2, Gauge, HardDrive } from "lucide-react";
import { api, BandwidthProfile } from "../lib/ipc";
import {
  bpsToLogSlider,
  formatBytes,
  formatDualBandwidth,
  formatSpeedPrimary,
  logSliderToBps,
} from "../lib/format";
import { useT } from "../i18n";
import { useLimiterStore } from "../stores/limiterStore";
import { useSettingsStore } from "../stores/settingsStore";
import { Chip, Popover, Slider } from "../ui/Primitives";

const PROFILES: BandwidthProfile[] = ["unlimited", "balanced", "light", "custom"];

/**
 * Status-bar chip with the live bandwidth limit and the free space on the destination drive. The chip
 * is only an icon: a dot means a limit is on, and Game Mode fills it with the accent.
 */
export const ResourcesChip: React.FC = () => {
  const t = useT();
  const settings = useSettingsStore((s) => s.settings);
  const updateSettings = useSettingsStore((s) => s.updateSettings);
  const stats = useLimiterStore((s) => s.stats);
  const gameMode = useLimiterStore((s) => s.gameMode);
  const [open, setOpen] = useState(false);
  const [freeDisk, setFreeDisk] = useState<number | null>(null);

  // The free space only shows in the panel: read it up front, then every 15 s while it is open.
  useEffect(() => {
    const refresh = () => {
      api.getFreeDiskSpace(settings.video_root).then(setFreeDisk);
    };
    refresh();
    if (!open) return;
    const id = setInterval(refresh, 15_000);
    return () => clearInterval(id);
  }, [open, settings.video_root]);

  const limited = !gameMode.active && stats.profile !== "unlimited";

  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <button
          type="button"
          className="status-chip status-chip-icon"
          data-accent={gameMode.active}
          aria-label={t.app.resourcesAria}
        >
          {gameMode.active ? (
            <Gamepad2 size={13} aria-hidden="true" />
          ) : (
            <Gauge size={13} aria-hidden="true" />
          )}
          {limited && <span className="status-chip-mark" aria-hidden="true" />}
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content side="top" align="end" sideOffset={8} className="op-panel res-pop">
          {gameMode.active && (
            <div className="res-pop-game">
              <Gamepad2 size={13} aria-hidden="true" />
              {t.app.limiter.gameMode(formatSpeedPrimary(stats.limit_bps, settings.speed_unit))}
            </div>
          )}
          <div className="res-pop-title">{t.app.limiter.popoverTitle}</div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            {PROFILES.map((prof) => (
              <Chip
                key={prof}
                active={settings.bandwidth_profile === prof}
                onClick={async () => {
                  await api.setLimit(prof, settings.custom_limit_bps);
                  await updateSettings({ bandwidth_profile: prof });
                }}
              >
                {t.app.profiles[prof]}
              </Chip>
            ))}
          </div>
          <div className="font-mono-num" style={{ fontSize: 11.5, color: "var(--text-2)" }}>
            {formatDualBandwidth(settings.custom_limit_bps, settings.speed_unit)}
          </div>
          <Slider
            min={0}
            max={100}
            step={0.5}
            value={[bpsToLogSlider(settings.custom_limit_bps)]}
            onValueChange={async ([v]) => {
              const bps = logSliderToBps(v);
              await api.setLimit("custom", bps);
              await updateSettings({ bandwidth_profile: "custom", custom_limit_bps: bps });
            }}
          />
          {freeDisk !== null && (
            <div className="res-pop-disk">
              <HardDrive size={13} aria-hidden="true" />
              <span>{t.app.diskLabel}</span>
              <span className="font-mono-num res-pop-disk-value">
                {t.app.freeDisk(formatBytes(freeDisk))}
              </span>
            </div>
          )}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
};
