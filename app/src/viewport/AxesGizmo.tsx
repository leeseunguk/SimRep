import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import type { Vec3 } from "../scene/mesh";
import type { ViewPreset } from "../state/store";
import type { CameraBasis } from "./camera";
import type { Renderer } from "./renderer";
import styles from "./AxesGizmo.module.css";

const AXES: { label: string; dir: Vec3; cls: string; pos: ViewPreset; neg: ViewPreset }[] = [
  { label: "X", dir: [1, 0, 0], cls: "x", pos: "right", neg: "left" },
  { label: "Y", dir: [0, 1, 0], cls: "y", pos: "top", neg: "bottom" },
  { label: "Z", dir: [0, 0, 1], cls: "z", pos: "front", neg: "back" },
];

const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

/** Orientation gizmo; click an axis end to look along it (Blender-style). */
export function AxesGizmo({ renderer, onPreset }: { renderer: Renderer; onPreset(view: ViewPreset): void }) {
  const { t } = useI18n();
  const [basis, setBasis] = useState<CameraBasis>(() => renderer.camera.basis());

  useEffect(() => renderer.addFrameListener(() => setBasis(renderer.camera.basis())), [renderer]);

  const R = 30;
  const ends = AXES.flatMap((a) => {
    const x = dot(a.dir, basis.right) * R;
    const y = -dot(a.dir, basis.up) * R;
    const z = dot(a.dir, basis.back);
    return [
      { ...a, sign: 1, x, y, z, view: a.pos },
      { ...a, sign: -1, x: -x, y: -y, z: -z, view: a.neg },
    ];
  }).sort((p, q) => p.z - q.z);

  return (
    <svg className={styles.gizmo} viewBox="-44 -44 88 88" role="group" aria-label={t("viewport.gizmo")}>
      <title>{t("viewport.gizmo")}</title>
      <circle className={styles.backdrop} r={42} />
      {ends.map((e) =>
        e.sign > 0 ? (
          <g key={e.label + e.sign} className={`${styles.axis} ${styles[e.cls]}`} onClick={() => onPreset(e.view)}>
            <line x1={0} y1={0} x2={e.x} y2={e.y} />
            <circle cx={e.x} cy={e.y} r={8} />
            <text x={e.x} y={e.y} dy="0.35em">
              {e.label}
            </text>
          </g>
        ) : (
          <circle
            key={e.label + e.sign}
            className={`${styles.negative} ${styles[e.cls]}`}
            cx={e.x}
            cy={e.y}
            r={5}
            onClick={() => onPreset(e.view)}
          />
        ),
      )}
    </svg>
  );
}
