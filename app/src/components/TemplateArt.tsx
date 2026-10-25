import type { CSSProperties } from "react";
import type { TemplateId } from "../scene/templates";
import styles from "./TemplateArt.module.css";

/**
 * Schematic of each gallery template (axial cross-section, or end-on for the quadrupole), drawn
 * with the viewport's electrode and trajectory colours so the cards match the 3D view.
 */
export function TemplateArt({ id }: { id: TemplateId }) {
  return (
    <svg className={styles.art} viewBox="0 0 240 112" aria-hidden preserveAspectRatio="xMidYMid meet">
      {id === "einzel" && <Einzel />}
      {id === "quadrupole" && <Quadrupole />}
      {id === "reflectron" && <Reflectron />}
      {id === "trap" && <Trap />}
    </svg>
  );
}

const Axis = () => <line className={styles.axis} x1="6" y1="56" x2="234" y2="56" />;

/** Pair of rectangles mirrored about the axis (a ring / tube in cross-section). */
function Ring({ x, w, r0, r1, className, style }: { x: number; w: number; r0: number; r1: number; className?: string; style?: CSSProperties }) {
  return (
    <>
      <rect className={className} style={style} x={x} y={56 - r1} width={w} height={r1 - r0} rx="1.5" />
      <rect className={className} style={style} x={x} y={56 + r0} width={w} height={r1 - r0} rx="1.5" />
    </>
  );
}

function Einzel() {
  return (
    <>
      <Axis />
      <path className={styles.field} d="M96 30 Q120 56 96 82 M144 30 Q120 56 144 82 M104 30 Q120 56 104 82 M136 30 Q120 56 136 82" />
      <Ring x={30} w={60} r0={26} r1={34} className={styles.neutral} />
      <Ring x={100} w={40} r0={26} r1={34} className={styles.positive} />
      <Ring x={150} w={60} r0={26} r1={34} className={styles.neutral} />
      {[-12, -6, 0, 6, 12].map((y) => (
        <path key={y} className={styles.beam} d={`M8 ${56 + y} L80 ${56 + y} C118 ${56 + y * 1.05} 150 ${56 + y * 0.45} 222 ${56 - y * 0.15}`} />
      ))}
    </>
  );
}

function Quadrupole() {
  return (
    <>
      <path className={styles.field} d="M86 22 Q120 56 154 22 M86 90 Q120 56 154 90 M98 30 Q120 56 142 30 M98 82 Q120 56 142 82" />
      <circle className={styles.positive} cx="74" cy="56" r="19" />
      <circle className={styles.positive} cx="166" cy="56" r="19" />
      <circle className={styles.negative} cx="120" cy="17" r="15" />
      <circle className={styles.negative} cx="120" cy="95" r="15" />
      <ellipse className={styles.beamFill} cx="120" cy="56" rx="17" ry="5" />
      <circle className={styles.dot} cx="120" cy="56" r="2" />
    </>
  );
}

function Reflectron() {
  const rings = [0, 1, 2, 3, 4, 5];
  return (
    <>
      <Axis />
      {rings.map((k) => (
        <Ring key={k} x={62 + k * 22} w={7} r0={30} r1={38} className={styles.ramp} style={{ "--amt": `${k * 18}%` } as CSSProperties} />
      ))}
      <rect className={styles.positive} x="200" y="16" width="8" height="80" rx="1.5" />
      <path className={styles.beam} d="M8 64 L120 60 Q176 56 120 52 L8 48" />
      <path className={styles.beamFaint} d="M8 66 L124 61.5 Q186 56 124 50.5 L8 46" />
    </>
  );
}

function Trap() {
  return (
    <>
      <Axis />
      <Ring x={92} w={56} r0={30} r1={42} className={styles.positive} />
      <rect className={styles.neutral} x="72" y="14" width="7" height="36" rx="1.5" />
      <rect className={styles.neutral} x="72" y="62" width="7" height="36" rx="1.5" />
      <rect className={styles.neutral} x="161" y="14" width="7" height="36" rx="1.5" />
      <rect className={styles.neutral} x="161" y="62" width="7" height="36" rx="1.5" />
      <path className={styles.beam} d="M8 56 L76 56 C96 56 100 44 110 44 S124 68 134 68 S148 44 158 50 L164 56 L232 56" />
    </>
  );
}
