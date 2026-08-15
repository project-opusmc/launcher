export const tuiTokens = {
  colors: {
    background: "var(--tui-background)",
    surface: "var(--tui-surface)",
    surfaceRaised: "var(--tui-surface-raised)",
    foreground: "var(--tui-foreground)",
    muted: "var(--tui-muted)",
    quiet: "var(--tui-quiet)",
    border: "var(--tui-border)",
    borderStrong: "var(--tui-border-strong)",
    focus: "var(--tui-focus)",
    selection: "var(--tui-selection)",
    success: "var(--tui-success)",
    warning: "var(--tui-warning)",
    error: "var(--tui-error)",
  },
  spacing: {
    xs: "var(--tui-space-xs)",
    sm: "var(--tui-space-sm)",
    md: "var(--tui-space-md)",
    lg: "var(--tui-space-lg)",
    xl: "var(--tui-space-xl)",
  },
} as const;

export type TuiStatusTone = "normal" | "muted" | "success" | "warning" | "error";
