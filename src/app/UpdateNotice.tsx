import React, { useState } from "react";
import { motion } from "motion/react";
import { Button, Popover, SPRING_UI } from "../ui/Primitives";
import { useSettingsStore } from "../stores/settingsStore";

/**
 * Status-bar chip for an update that is ready. Its details open by themselves once, without taking
 * focus from what the user is doing, and the chip stays until the update is applied.
 */
export const UpdateChip: React.FC<{
  label: string;
  ariaLabel: string;
  title: string;
  body: string;
  actionLabel: string;
  busyLabel: string;
  laterLabel: string;
  busy: boolean;
  error?: string | null;
  autoOpen?: boolean;
  onAction: () => void;
}> = ({
  label,
  ariaLabel,
  title,
  body,
  actionLabel,
  busyLabel,
  laterLabel,
  busy,
  error,
  autoOpen = false,
  onAction,
}) => {
  const [open, setOpen] = useState(autoOpen);
  const reduceMotion = useSettingsStore((s) => s.settings.reduce_motion);
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <motion.button
          type="button"
          className="font-mono-num update-chip"
          initial={reduceMotion ? false : { opacity: 0, scale: 0.92 }}
          animate={{ opacity: 1, scale: 1 }}
          transition={SPRING_UI}
          aria-label={ariaLabel}
        >
          <span className="update-chip-dot" aria-hidden="true" />
          {label}
        </motion.button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="end"
          sideOffset={8}
          className="op-panel update-pop"
          onOpenAutoFocus={(e) => e.preventDefault()}
        >
          <div className="update-pop-title">{title}</div>
          <p className="update-pop-text">{body}</p>
          {error && (
            <p className="update-pop-text" role="alert" style={{ color: "var(--warn)" }}>
              {error}
            </p>
          )}
          <div className="update-pop-actions">
            <Button size="sm" variant="primary" disabled={busy} onClick={onAction}>
              {busy ? busyLabel : actionLabel}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setOpen(false)}>
              {laterLabel}
            </Button>
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
};
