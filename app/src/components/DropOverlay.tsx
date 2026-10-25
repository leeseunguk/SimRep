import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Upload } from "lucide-react";
import { useI18n } from "../i18n";
import { COMING_SOON_EXTENSIONS, READY_EXTENSIONS } from "../scene/formats";
import { importFiles } from "../scene/importFiles";
import { openPaths } from "../state/project";
import { useApp } from "../state/store";
import styles from "./DropOverlay.module.css";

const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes("Files");

/**
 * Window-wide file drop target.
 * - Desktop: Tauri's own drag-drop event (dragDropEnabled: true) gives native paths, so the core
 *   reads files directly; nothing is uploaded over IPC.
 * - Browser preview: standard HTML5 drag events with File objects.
 */
export function DropOverlay() {
  const { t } = useI18n();
  const [active, setActive] = useState(false);
  const depth = useRef(0);

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type === "enter") setActive(payload.paths.length > 0);
        else if (payload.type === "leave") setActive(false);
        else if (payload.type === "drop") {
          setActive(false);
          if (payload.paths.length) void openPaths(payload.paths);
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (isTauri()) return;
    const onEnter = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth.current++;
      setActive(true);
    };
    const onOver = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    const onLeave = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      depth.current = Math.max(0, depth.current - 1);
      if (depth.current === 0) setActive(false);
    };
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth.current = 0;
      setActive(false);
      const files = Array.from(e.dataTransfer?.files ?? []);
      if (files.length) {
        useApp.getState().setHome(false);
        void importFiles(files);
      }
    };
    window.addEventListener("dragenter", onEnter);
    window.addEventListener("dragover", onOver);
    window.addEventListener("dragleave", onLeave);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("dragenter", onEnter);
      window.removeEventListener("dragover", onOver);
      window.removeEventListener("dragleave", onLeave);
      window.removeEventListener("drop", onDrop);
    };
  }, []);

  if (!active) return null;
  return (
    <div className={styles.overlay}>
      <div className={styles.card}>
        <div className={styles.iconRing}>
          <Upload className={styles.icon} aria-hidden />
        </div>
        <p className={styles.title}>{t("drop.title")}</p>
        <p className={styles.body}>{t("drop.body")}</p>
        <div className={styles.chips}>
          {[...READY_EXTENSIONS, ".simrep"].map((ext) => (
            <span key={ext} className={styles.ready}>
              {ext}
            </span>
          ))}
        </div>
        <p className={styles.soonLabel}>{t("drop.soon")}</p>
        <div className={styles.chips}>
          {COMING_SOON_EXTENSIONS.map((ext) => (
            <span key={ext} className={styles.chip}>
              {ext}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}
