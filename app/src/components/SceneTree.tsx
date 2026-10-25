import { useState, type KeyboardEvent, type ReactNode } from "react";
import { Box, Boxes, ChevronDown, Eye, EyeOff, FileQuestion, FileUp, Loader2, X, Zap } from "lucide-react";
import { useI18n } from "../i18n";
import { electrodeName, electrodeScale, paElectrodes, type PaElectrode } from "../scene/electrodes";
import { objectName } from "../scene/names";
import { potentialAmount } from "../scene/potential";
import { useApp, type PaObject, type SceneObject } from "../state/store";
import { Badge } from "./ui/Badge";
import panel from "./Panel.module.css";
import styles from "./SceneTree.module.css";

/** Tree row key: an object id, or `<array id>#<electrode label>`. */
const rowKey = (id: string, label: number | null) => (label === null ? id : `${id}#${label}`);

export function SceneTree() {
  const { t, num } = useI18n();
  const objects = useApp((s) => s.objects);
  const selectedId = useApp((s) => s.selectedId);
  const selectedElectrode = useApp((s) => s.selectedElectrode);
  const { select, toggleVisible, toggleElectrode, removeObject, rename, renameElectrode, requestView, log } = useApp.getState();
  const [editing, setEditing] = useState<string | null>(null);

  const arrays = objects.filter((o): o is PaObject => o.kind === "pa");
  const electrodeGroups = arrays.map((o) => ({ o, list: paElectrodes(o) })).filter((g) => g.list.length > 0);
  const electrodeCount = electrodeGroups.reduce((n, g) => n + g.list.length, 0);
  const rows: { id: string; label: number | null }[] = [
    ...electrodeGroups.flatMap((g) => g.list.map((e) => ({ id: g.o.id, label: e.label }))),
    ...objects.map((o) => ({ id: o.id, label: null })),
  ];
  const selectedKey = selectedId ? rowKey(selectedId, selectedElectrode) : null;
  const firstKey = rows[0] ? rowKey(rows[0].id, rows[0].label) : null;

  const remove = (o: SceneObject) => {
    removeObject(o.id);
    log("info", { key: "log.removed", params: { name: objectName(o) } });
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (editing) return;
    if (e.key === "F2" && selectedKey) {
      e.preventDefault();
      setEditing(selectedKey);
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const i = rows.findIndex((r) => rowKey(r.id, r.label) === selectedKey);
    const next = rows[e.key === "ArrowDown" ? Math.min(rows.length - 1, i + 1) : Math.max(0, i - 1)];
    if (next) {
      select(next.id, next.label);
      document.getElementById(`tree-${rowKey(next.id, next.label)}`)?.focus();
    }
  };

  const nameCell = (key: string, name: string, commit: (name: string) => void) =>
    editing === key ? (
      <RenameInput
        value={name}
        label={t("tree.rename")}
        onDone={(value) => {
          setEditing(null);
          if (value !== null && value !== name) commit(value);
        }}
      />
    ) : (
      <span className={styles.name} title={name}>
        {name}
      </span>
    );

  const row = (
    key: string,
    selected: boolean,
    hidden: boolean,
    kind: string,
    icon: ReactNode,
    name: ReactNode,
    meta: ReactNode,
    actions: ReactNode,
    handlers: { onClick(): void; onDoubleClick(): void },
  ) => (
    <div
      key={key}
      id={`tree-${key}`}
      role="treeitem"
      aria-selected={selected}
      tabIndex={selected || (!selectedKey && key === firstKey) ? 0 : -1}
      className={styles.row}
      data-hidden={hidden}
      data-kind={kind}
      onClick={handlers.onClick}
      onDoubleClick={handlers.onDoubleClick}
    >
      <span className={styles.icon}>{icon}</span>
      {name}
      <span className={styles.meta}>{meta}</span>
      <span className={styles.actions}>{actions}</span>
    </div>
  );

  const eye = (visible: boolean, onToggle: () => void) => (
    <button
      type="button"
      className={styles.action}
      data-persistent={!visible}
      aria-label={t(visible ? "tree.hide" : "tree.show")}
      title={t(visible ? "tree.hide" : "tree.show")}
      onClick={(e) => {
        e.stopPropagation();
        onToggle();
      }}
    >
      {visible ? <Eye /> : <EyeOff />}
    </button>
  );

  const electrodeRow = (o: PaObject, e: PaElectrode, scale: number) => {
    const key = rowKey(o.id, e.label);
    const name = electrodeName(o, e, t, num);
    const hidden = o.hiddenElectrodes.includes(e.label);
    const v = e.voltage ?? 0;
    const amount = potentialAmount(v, scale) * 100;
    const pole = v >= 0 ? "var(--vp-electrode-positive)" : "var(--vp-electrode-negative)";
    const meta =
      e.kind === "numbered" ? (
        <span title={t("tree.electrode.unsolved")}>—</span>
      ) : e.voltage !== null ? (
        <span className="num">{num(e.voltage)} V</span>
      ) : null;
    return row(
      key,
      key === selectedKey,
      hidden || !o.visible,
      "electrode",
      <span className={styles.chip} title={e.points !== null ? t("tree.electrode.points", { count: e.points }) : undefined}>
        <span className={styles.chipDot} style={{ background: `color-mix(in srgb, ${pole} ${amount}%, var(--vp-electrode-neutral))` }} />
        <span className="num">{e.number !== null ? e.number : "·"}</span>
      </span>,
      nameCell(key, name, (value) => renameElectrode(o.id, e.label, value)),
      meta,
      eye(!hidden, () => toggleElectrode(o.id, e.label)),
      { onClick: () => select(o.id, e.label), onDoubleClick: () => setEditing(key) },
    );
  };

  const objectRow = (o: SceneObject, icon: ReactNode, meta: ReactNode) => {
    const canToggle = o.kind === "mesh" || o.kind === "pa";
    return row(
      o.id,
      o.id === selectedKey,
      !o.visible,
      o.kind,
      icon,
      nameCell(o.id, objectName(o), (value) => rename(o.id, value)),
      meta,
      <>
        {canToggle && eye(o.visible, () => toggleVisible(o.id))}
        <button
          type="button"
          className={styles.action}
          aria-label={t("tree.remove")}
          title={t("tree.remove")}
          onClick={(e) => {
            e.stopPropagation();
            remove(o);
          }}
        >
          <X />
        </button>
      </>,
      { onClick: () => select(o.id), onDoubleClick: () => canToggle && requestView({ type: "fit", ids: [o.id] }) },
    );
  };

  return (
    <aside className={panel.panel}>
      <div className={panel.header}>
        <h2 className={panel.title}>{t("tree.title")}</h2>
        <span className={panel.count}>{objects.length}</span>
      </div>
      <div className={panel.body} role="tree" aria-label={t("tree.title")} onKeyDown={onKeyDown}>
        <Group label={t("tree.electrodes")} count={electrodeCount}>
          {electrodeGroups.length === 0 && (
            <p className={styles.empty}>
              <Zap aria-hidden />
              {t("tree.electrodesEmpty")}
            </p>
          )}
          {electrodeGroups.map(({ o, list }) => {
            const scale = electrodeScale(list);
            return (
              <div key={o.id} role="group" aria-label={objectName(o)} className={styles.subgroup}>
                <p className={styles.subgroupLabel} title={objectName(o)}>
                  {objectName(o)}
                </p>
                {list.map((e) => electrodeRow(o, e, scale))}
              </div>
            );
          })}
        </Group>
        <Group label={t("tree.files")} count={objects.length}>
          {objects.length === 0 && (
            <p className={styles.empty}>
              <FileUp aria-hidden />
              {t("tree.importedEmpty")}
            </p>
          )}
          {objects.map((o) => {
            if (o.kind === "mesh")
              return objectRow(
                o,
                <Box />,
                <span className="num" title={t("tree.triangles", { count: o.mesh.triangleCount })}>
                  {num(o.mesh.triangleCount)}
                </span>,
              );
            if (o.kind === "pa") {
              const { nx, ny, nz } = o.info;
              const size = nz > 1 ? `${nx}×${ny}×${nz}` : `${nx}×${ny}`;
              return objectRow(o, <Boxes />, <span className="num" title={t("props.pa.points", { count: o.info.points })}>{size}</span>);
            }
            if (o.kind === "loading") return objectRow(o, <Loader2 className={styles.spin} />, t("tree.reading"));
            return objectRow(o, <FileQuestion />, <Badge>{t("tree.comingSoon")}</Badge>);
          })}
        </Group>
      </div>
    </aside>
  );
}

function RenameInput({ value, label, onDone }: { value: string; label: string; onDone(value: string | null): void }) {
  const [text, setText] = useState(value);
  return (
    <input
      className={styles.renameInput}
      value={text}
      aria-label={label}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onChange={(e) => setText(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onBlur={() => onDone(text)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") onDone(null);
      }}
    />
  );
}

function Group({ label, count, children }: { label: string; count: number; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <div role="group" className={styles.group} data-open={open}>
      <button type="button" className={styles.groupHeader} aria-expanded={open} onClick={() => setOpen(!open)}>
        <ChevronDown className={styles.chevron} aria-hidden />
        <span className={styles.groupLabel}>{label}</span>
        <span className={styles.groupCount}>{count}</span>
      </button>
      {open && <div className={styles.groupBody}>{children}</div>}
    </div>
  );
}
