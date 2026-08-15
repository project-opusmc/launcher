import { forwardRef, type ReactNode } from "react";
import type { TuiStatusTone } from "./theme/tokens";

export type MenuItem = {
  id: string;
  label: string;
  shortcut?: string;
  value?: string;
  tone?: TuiStatusTone;
  disabled?: boolean;
};

export type KeyValueRow = {
  label: string;
  value: string;
  tone?: TuiStatusTone;
};

export const Pane = forwardRef<HTMLElement, {
  title: string;
  children: ReactNode;
  focused?: boolean;
  className?: string;
  onFocus?: () => void;
  onClick?: () => void;
  footer?: ReactNode;
}>(function Pane(props, ref) {
  const className = ["tui-pane", props.focused ? "is-focused" : "", props.className ?? ""]
    .filter(Boolean)
    .join(" ");
  return (
    <section
      ref={ref}
      className={className}
      tabIndex={-1}
      onFocus={props.onFocus}
      onClick={props.onClick}
      onMouseDown={(event) => {
        const target = event.target;
        if (!(target instanceof Element)) {
          return;
        }
        if (!target.closest("button, input, select, textarea, a, [contenteditable='true']")) {
          event.currentTarget.focus();
        }
      }}
      aria-label={props.title}
    >
      <header className="tui-pane-header">
        <span className="tui-pane-rule" aria-hidden="true" />
        <h2>{props.title}</h2>
        <span className="tui-pane-rule tui-pane-rule--right" aria-hidden="true" />
      </header>
      <div className="tui-pane-content">{props.children}</div>
      {props.footer ? <footer className="tui-pane-footer">{props.footer}</footer> : null}
    </section>
  );
});

export function MenuList(props: {
  items: readonly MenuItem[];
  selectedIndex: number;
  onSelect: (index: number) => void;
  onActivate?: (index: number) => void;
}) {
  return (
    <div className="tui-menu-list" role="listbox" aria-label="Menu">
      {props.items.map((item, index) => {
        const selected = index === props.selectedIndex;
        return (
          <button
            key={item.id}
            type="button"
            className={`tui-menu-item ${selected ? "is-selected" : ""}`}
            disabled={item.disabled}
            tabIndex={-1}
            data-tui-nav-item
            role="option"
            aria-selected={selected}
            onMouseEnter={() => props.onSelect(index)}
            onFocus={() => props.onSelect(index)}
            onClick={() => {
              props.onSelect(index);
              props.onActivate?.(index);
            }}
          >
            <span className="tui-menu-caret" aria-hidden="true">{selected ? ">" : " "}</span>
            {item.shortcut ? <kbd>[{item.shortcut}]</kbd> : <span className="tui-menu-key-spacer" />}
            <span className="tui-menu-label">{item.label}</span>
            {item.value ? <StatusText value={item.value} tone={item.tone ?? "muted"} /> : null}
          </button>
        );
      })}
    </div>
  );
}

export function KeyValueList(props: { rows: readonly KeyValueRow[] }) {
  return (
    <dl className="tui-key-value-list">
      {props.rows.map((row) => (
        <div className="tui-key-value-row" key={row.label}>
          <dt>{row.label}</dt>
          <dd><StatusText value={row.value} tone={row.tone ?? "normal"} /></dd>
        </div>
      ))}
    </dl>
  );
}

export function StatusText(props: { value: string; tone?: TuiStatusTone }) {
  return <span className={`tui-status tui-status--${props.tone ?? "normal"}`}>{props.value}</span>;
}

export function ProgressBar(props: { value: number; label?: string }) {
  const value = Math.min(100, Math.max(0, props.value));
  return (
    <div className="tui-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={value} aria-label={props.label ?? "Progress"}>
      <span style={{ width: `${value}%` }} />
      <strong>{value}%</strong>
    </div>
  );
}

export function LogView(props: { lines: readonly string[]; emptyLabel?: string }) {
  return (
    <pre className="tui-log-view" aria-live="polite">
      {(props.lines.length > 0 ? props.lines : [props.emptyLabel ?? "No log entries."]).join("\n")}
    </pre>
  );
}

export function CommandBar(props: { commands: readonly string[] }) {
  return (
    <footer className="tui-command-bar" aria-label="Keyboard commands">
      {props.commands.map((command) => {
        const separator = command.indexOf(" ");
        const key = separator === -1 ? command : command.slice(0, separator);
        const label = separator === -1 ? "" : command.slice(separator + 1);
        return (
          <span className="tui-command" key={command}>
            <kbd>{key}</kbd>
            {label ? <span>{label}</span> : null}
          </span>
        );
      })}
    </footer>
  );
}
