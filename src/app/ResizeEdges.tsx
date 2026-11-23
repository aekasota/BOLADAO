import React from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauriRuntime } from "../lib/ipc";
import { PLATFORM } from "../lib/platform";

type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>["startResizeDragging"]>[0];

const EDGES: [ResizeDirection, string][] = [
  ["North", "n"],
  ["South", "s"],
  ["East", "e"],
  ["West", "w"],
  ["NorthEast", "ne"],
  ["NorthWest", "nw"],
  ["SouthEast", "se"],
  ["SouthWest", "sw"],
];

/**
 * Invisible borders that resize the window on Linux. Windows resizes a frameless window by its
 * own edges and macOS keeps its native frame; GTK offers no edge without a title bar.
 */
export const ResizeEdges: React.FC = () => {
  if (PLATFORM !== "linux" || !isTauriRuntime()) return null;
  const start = (direction: ResizeDirection) => (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    void getCurrentWindow().startResizeDragging(direction);
  };
  return (
    <>
      {EDGES.map(([direction, edge]) => (
        <div key={edge} className={`resize-edge resize-${edge}`} onMouseDown={start(direction)} aria-hidden="true" />
      ))}
    </>
  );
};
