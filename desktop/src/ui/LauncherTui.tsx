import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from "react";
import {
  CommandBar,
  KeyValueList,
  LogView,
  MenuList,
  Pane,
  ProgressBar,
  StatusText,
  type KeyValueRow,
  type MenuItem,
} from "./primitives";
import "./tui.css";

export type TuiPage =
  | "home"
  | "installation"
  | "account"
  | "modules"
  | "utilities"
  | "settings"
  | "logs";

export type TuiUtilityId = "fps" | "cps" | "memory" | "coordinates" | "clock" | "keystrokes";
export type TuiUtilityAnchor = "top-left" | "top-right" | "bottom-left" | "bottom-right";

export type TuiUtilityDefinition = {
  id: TuiUtilityId;
  category: "performance" | "hud" | "input";
  name: string;
  description: string;
  purpose: string;
  icon: string;
};

export type TuiUtilityPreference = {
  enabled: boolean;
  anchor: TuiUtilityAnchor;
  offset: string;
  scale: number;
  opacity: number;
};

export type TuiAccount = {
  id: string;
  username: string;
  uuid: string | null;
  kind: "microsoft" | "offline";
  badge: string;
  ready: boolean;
  selected: boolean;
};

export type TuiInstance = {
  sessionId: string;
  accountId: string;
  username: string;
  badge: string;
  title: string;
  logDirectory: string;
};

export type TuiAccountSkin = {
  dataUrl: string;
  model: "classic" | "slim";
  isDefault: boolean;
};

export type TuiSnapshot = {
  platform: string;
  dataDirectory: string;
  minecraftReady: boolean;
  minecraftStatus: string;
  optifineReady: boolean;
  optifineStatus: string;
  javaStatus: string;
  accountStored: boolean;
  gameLaunchReady: boolean;
  buildEdition: "premium" | "qaOffline";
  offlineProfile: { username: string; valid: boolean } | null;
  accounts: TuiAccount[];
  selectedAccountId: string | null;
  activeLaunches: number;
  activeAccountIds: string[];
  developerTestProfile: {
    available: boolean;
    active: boolean;
    simulationActive: boolean;
  };
};

export type TuiSettings = {
  maxMemoryMib: number;
  closeLauncherOnGameStart: boolean;
};

export type TuiInstallProgress = {
  phase: string;
  completedFiles: number;
  totalFiles: number;
  downloadedFiles: number;
  cachedFiles: number;
};

type LauncherTuiProps = {
  page: TuiPage;
  snapshot: TuiSnapshot | null;
  settings: TuiSettings;
  utilities: readonly TuiUtilityDefinition[];
  utilityPreferences: Record<TuiUtilityId, TuiUtilityPreference>;
  utilitySaveStatus: "loading" | "saved" | "saving" | "error";
  busy: string | null;
  error: string | null;
  notice: string | null;
  installProgress: TuiInstallProgress | null;
  loginCancelling: boolean;
  runningSessions: Record<string, string>;
  instances: readonly TuiInstance[];
  skin: TuiAccountSkin | null;
  skinLoading: boolean;
  qaUsername: string;
  optifineSourcePath: string;
  onNavigate: (page: TuiPage) => void;
  onBack: () => void;
  onKillInstance: (sessionId: string) => void;
  onRefresh: () => void;
  onLaunch: (accountId: string) => void;
  onInstall: () => void;
  onImportOptiFine: () => void;
  onSaveSettings: () => void;
  onStartMicrosoftLogin: () => void;
  onCancelMicrosoftLogin: () => void;
  onSaveQaProfile: () => void;
  onSelectAccount: (accountId: string) => void;
  onRemoveAccount: (accountId: string) => void;
  onRunDeveloperSimulation: () => void;
  onQuit: () => void;
  onClearError: () => void;
  onClearNotice: () => void;
  onSettingsChange: (settings: TuiSettings) => void;
  onQaUsernameChange: (value: string) => void;
  onOptiFineSourcePathChange: (value: string) => void;
  onUtilityPreferenceChange: (id: TuiUtilityId, update: Partial<TuiUtilityPreference>) => void;
};

const utilityAnchorLabels: Record<TuiUtilityAnchor, string> = {
  "top-left": "Top left",
  "top-right": "Top right",
  "bottom-left": "Bottom left",
  "bottom-right": "Bottom right",
};

const logCategories = ["Launcher", "Authentication", "Installation", "Minecraft", "Opus"] as const;

export default function LauncherTui(props: LauncherTuiProps) {
  const [focusedPane, setFocusedPane] = useState(0);
  const [playIndex, setPlayIndex] = useState(0);
  const [installationIndex, setInstallationIndex] = useState(0);
  const [settingsIndex, setSettingsIndex] = useState(0);
  const [selectedUtilityId, setSelectedUtilityId] = useState<TuiUtilityId>(props.utilities[0]?.id ?? "fps");
  const [logCategory, setLogCategory] = useState<(typeof logCategories)[number]>("Launcher");
  const paneRefs = useRef<Array<HTMLElement | null>>([]);

  const snapshotPending = props.snapshot === null && props.error === null;
  const snapshotUnavailable = props.snapshot === null && props.error !== null;
  const isQaOfflineEdition = props.snapshot?.buildEdition === "qaOffline";
  const developerProfile = props.snapshot?.developerTestProfile;
  const developerTestProfileActive = !isQaOfflineEdition && (developerProfile?.active ?? false);
  const developerSimulationActive = !isQaOfflineEdition && (developerProfile?.simulationActive ?? false);
  const installed = Boolean(props.snapshot?.minecraftReady) && !developerTestProfileActive;
  const gameLaunchReady = Boolean(props.snapshot?.gameLaunchReady) && !developerTestProfileActive;
  const optifineReady = Boolean(props.snapshot?.optifineReady) && !developerTestProfileActive;
  const accounts = props.snapshot?.accounts ?? [];
  const selectedAccount = accounts.find((account) => account.id === props.snapshot?.selectedAccountId)
    ?? accounts.find((account) => account.selected)
    ?? null;
  const activeAccountIds = useMemo(
    () => new Set([...(props.snapshot?.activeAccountIds ?? []), ...Object.keys(props.runningSessions)]),
    [props.runningSessions, props.snapshot?.activeAccountIds],
  );
  const activeLaunchCount = activeAccountIds.size;
  const selectedAccountRunning = selectedAccount ? activeAccountIds.has(selectedAccount.id) : false;
  const profileReady = Boolean(selectedAccount?.ready) && !developerTestProfileActive;
  const readyToLaunch = installed && gameLaunchReady && optifineReady && profileReady;
  const canLaunch = readyToLaunch && !selectedAccountRunning && props.busy === null;
  const enabledUtilityCount = useMemo(
    () => Object.values(props.utilityPreferences).filter((preference) => preference.enabled).length,
    [props.utilityPreferences],
  );
  const selectedUtility = props.utilities.find((utility) => utility.id === selectedUtilityId) ?? props.utilities[0];
  const selectedUtilityPreference = selectedUtility ? props.utilityPreferences[selectedUtility.id] : undefined;
  const accountLabel = selectedAccount?.username ?? "NO IDENTITY";
  const editionLabel = isQaOfflineEdition ? "QA BUILD" : "UNIFIED CLIENT";
  const buildLabel = import.meta.env.DEV ? "v0.0.1-dev" : "v0.0.1";

  const runtimeState = snapshotPending
    ? "CHECKING"
    : snapshotUnavailable
      ? "UNAVAILABLE"
      : developerTestProfileActive
        ? developerSimulationActive ? "SIMULATING" : "DEVELOPER"
        : activeLaunchCount > 0
          ? `${activeLaunchCount} RUNNING`
          : readyToLaunch ? "READY" : "SETUP REQUIRED";

  const playItems = useMemo<readonly MenuItem[]>(() => [
    { id: "account", shortcut: "1", label: "Accounts & profiles", value: `${accounts.length}`, tone: accounts.length > 0 ? "success" : "warning" },
    { id: "installation", shortcut: "2", label: "Installation", value: installed ? "READY" : "MISSING", tone: installed ? "success" : "warning" },
    { id: "modules", shortcut: "3", label: "Modules", value: `${enabledUtilityCount} ON`, tone: enabledUtilityCount > 0 ? "success" : "muted" },
    { id: "settings", shortcut: "4", label: "Settings", value: `${props.settings.maxMemoryMib} MB`, tone: "normal" },
    { id: "logs", shortcut: "5", label: "Logs", value: "OPEN", tone: "muted" },
    { id: "quit", shortcut: "6", label: "Quit", disabled: props.busy !== null },
  ], [
    accounts.length,
    enabledUtilityCount,
    installed,
    props.busy,
    props.settings.maxMemoryMib,
  ]);

  const focusPane = useCallback((index: number) => {
    const next = (index + 4) % 4;
    setFocusedPane(next);
    window.requestAnimationFrame(() => paneRefs.current[next]?.focus());
  }, []);

  const focusPaneInDirection = useCallback((key: string) => {
    const panes = Array.from(document.querySelectorAll<HTMLElement>(".tui-screen-region .tui-pane"))
      .filter((pane) => pane.getClientRects().length > 0);
    if (panes.length === 0) {
      return;
    }
    const activeElement = document.activeElement;
    const current = panes.find((pane) => pane === activeElement || pane.contains(activeElement))
      ?? panes[0];
    const currentRect = current.getBoundingClientRect();
    const currentCenter = {
      x: currentRect.left + currentRect.width / 2,
      y: currentRect.top + currentRect.height / 2,
    };
    const direction = key.replace("Arrow", "").toLowerCase();
    const candidates = panes
      .filter((pane) => pane !== current)
      .map((pane) => {
        const rect = pane.getBoundingClientRect();
        const center = { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
        const deltaX = center.x - currentCenter.x;
        const deltaY = center.y - currentCenter.y;
        const inDirection = direction === "left" ? deltaX < -1
          : direction === "right" ? deltaX > 1
            : direction === "up" ? deltaY < -1
              : deltaY > 1;
        const primaryDistance = direction === "left" || direction === "right"
          ? Math.abs(deltaX)
          : Math.abs(deltaY);
        const crossDistance = direction === "left" || direction === "right"
          ? Math.abs(deltaY)
          : Math.abs(deltaX);
        return { pane, inDirection, score: primaryDistance + crossDistance * 2 };
      })
      .filter((candidate) => candidate.inDirection)
      .sort((left, right) => left.score - right.score);
    candidates[0]?.pane.focus();
  }, []);

  const activateLaunch = useCallback(() => {
    if (selectedAccountRunning || props.busy !== null) {
      return;
    }
    if (developerTestProfileActive) {
      props.onRunDeveloperSimulation();
      return;
    }
    if (canLaunch) {
      if (selectedAccount) props.onLaunch(selectedAccount.id);
      return;
    }
    if (!installed || !gameLaunchReady) {
      props.onNavigate("installation");
      return;
    }
    if (!optifineReady) {
      props.onNavigate("settings");
      setSettingsIndex(2);
      return;
    }
    if (!profileReady) {
      props.onNavigate("account");
    }
  }, [
    canLaunch,
    developerTestProfileActive,
    gameLaunchReady,
    installed,
    optifineReady,
    profileReady,
    props,
    selectedAccount,
    selectedAccountRunning,
  ]);

  const activatePlayItem = useCallback((index: number) => {
    const item = playItems[index];
    if (!item || item.disabled) {
      return;
    }
    switch (item.id) {
      case "installation":
      case "account":
      case "modules":
      case "settings":
      case "logs":
        props.onNavigate(item.id);
        break;
      case "quit":
        props.onQuit();
        break;
    }
  }, [activateLaunch, playItems, props]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target;
      const editing = target instanceof HTMLInputElement
        || target instanceof HTMLTextAreaElement
        || target instanceof HTMLSelectElement
        || (target instanceof HTMLElement && target.isContentEditable);

      if (event.shiftKey && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) {
        event.preventDefault();
        focusPaneInDirection(event.key);
        return;
      }

      if (editing) {
        if (event.key === "Escape" && target instanceof HTMLElement) {
          target.blur();
        }
        return;
      }

      if ((event.key === "l" || event.key === "L") && props.busy === null) {
        event.preventDefault();
        activateLaunch();
        return;
      }

      if ((event.key === "q" || event.key === "Q") && props.busy === null) {
        event.preventDefault();
        props.onQuit();
        return;
      }

      if (/^[1-6]$/.test(event.key)) {
        const index = Number(event.key) - 1;
        event.preventDefault();
        setPlayIndex(index);
        if (props.page !== "home") {
          props.onNavigate("home");
        }
        window.requestAnimationFrame(() => activatePlayItem(index));
        return;
      }

      if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) {
        const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        const pane = active?.closest<HTMLElement>(".tui-pane")
          ?? document.querySelector<HTMLElement>(".tui-pane.is-focused");
        const items = pane
          ? Array.from(pane.querySelectorAll<HTMLElement>("[data-tui-nav-item]:not(:disabled)"))
          : [];
        if (items.length > 0) {
          const currentIndex = active ? items.indexOf(active) : -1;
          const backwards = event.key === "ArrowLeft" || event.key === "ArrowUp";
          const nextIndex = currentIndex === -1
            ? backwards ? items.length - 1 : 0
            : (currentIndex + (backwards ? -1 : 1) + items.length) % items.length;
          event.preventDefault();
          items[nextIndex]?.focus();
          return;
        }
      }

      if (props.page !== "home") {
        if (event.key === "Escape") {
          event.preventDefault();
          props.onNavigate("home");
        }
        return;
      }

      if (event.key === "Tab" && event.shiftKey) {
        event.preventDefault();
        focusPane(focusedPane - 1);
        return;
      }
      if (event.key === "Tab") {
        event.preventDefault();
        focusPane(focusedPane + 1);
        return;
      }
      if (focusedPane === 0 && event.key === "Enter") {
        event.preventDefault();
        activatePlayItem(playIndex);
      }
    };

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [activateLaunch, activatePlayItem, focusPane, focusPaneInDirection, focusedPane, playIndex, props]);

  useEffect(() => {
    if (props.page === "home") {
      focusPane(0);
    }
  }, [focusPane, props.page]);

  const systemRows: readonly KeyValueRow[] = [
    { label: "Minecraft", value: installed ? "1.8.9" : snapshotPending ? "CHECKING" : "MISSING", tone: installed ? "success" : "warning" },
    { label: "Forge", value: installed ? "11.15.1.2318" : snapshotPending ? "CHECKING" : "MISSING", tone: installed ? "success" : "warning" },
    { label: "OptiFine", value: optifineReady ? "HD U M5" : snapshotPending ? "CHECKING" : "MISSING", tone: optifineReady ? "success" : "warning" },
    { label: "Java", value: installed ? "8" : snapshotPending ? "CHECKING" : "MISSING", tone: installed ? "success" : "warning" },
    { label: "Memory", value: `${props.settings.maxMemoryMib} MB` },
    { label: "Opus Build", value: gameLaunchReady ? buildLabel : "UNVERIFIED", tone: gameLaunchReady ? "success" : "warning" },
  ];

  const profileRows: readonly KeyValueRow[] = [
    { label: "Account", value: accountLabel, tone: profileReady ? "success" : "warning" },
    { label: "Badge", value: selectedAccount ? `[${selectedAccount.badge.toUpperCase()}]` : "NONE", tone: selectedAccount ? "warning" : "muted" },
    { label: "Auth", value: selectedAccount?.kind === "microsoft" ? "MICROSOFT" : selectedAccount ? "OFFLINE" : "NONE" },
    { label: "Status", value: profileReady ? "READY" : "REQUIRED", tone: profileReady ? "success" : "warning" },
    { label: "Instance", value: selectedAccountRunning ? "RUNNING" : "IDLE", tone: selectedAccountRunning ? "warning" : "muted" },
  ];

  const sessionRows: readonly KeyValueRow[] = [
    { label: "Mode", value: developerTestProfileActive ? "DEVELOPER" : "STANDARD", tone: developerTestProfileActive ? "warning" : "normal" },
    { label: "Instances", value: String(activeLaunchCount), tone: activeLaunchCount > 0 ? "warning" : "muted" },
    { label: "Augmentation", value: enabledUtilityCount > 0 ? "ON" : "OFF", tone: enabledUtilityCount > 0 ? "success" : "muted" },
    { label: "Isolation", value: "PER IDENTITY", tone: "success" },
    { label: "Launch", value: runtimeState, tone: readyToLaunch || activeLaunchCount > 0 ? "success" : "warning" },
  ];

  const logLines = useMemo(() => buildLogLines(props, runtimeState), [props, runtimeState]);
  const normalizedPage = props.page === "utilities" ? "modules" : props.page;

  return (
    <main className="tui-app">
      <header className="tui-header">
        <div className="tui-header-brand">
          {normalizedPage !== "home" ? (
            <button type="button" className="tui-header-back" onClick={props.onBack} aria-label="Back to menu">
              <span aria-hidden="true">&lt;</span>
              BACK
            </button>
          ) : null}
          <img className="tui-brand-mark" src="/brand/opus-mark-64.png" alt="" />
          <span className="tui-header-title">OPUS LAUNCHER</span>
          <span className="tui-header-state"><StatusText value={runtimeState} tone={readyToLaunch || activeLaunchCount > 0 ? "success" : "warning"} /></span>
        </div>
        <div className="tui-header-meta">
          <button type="button" onClick={() => props.onNavigate("account")}>{accountLabel}</button>
          <span aria-hidden="true">|</span>
          <span>{editionLabel}</span>
          <span aria-hidden="true">|</span>
          <span>{buildLabel}</span>
        </div>
      </header>

      <div className="tui-toast-stack" aria-live="polite">
        {props.error ? (
          <section className="tui-message tui-message--error" role="alert">
            <strong>ERROR</strong>
            <span>{friendlyError(props.error)}</span>
            <button type="button" onClick={props.onClearError}>DISMISS</button>
          </section>
        ) : null}
        {props.notice ? (
          <section className="tui-message tui-message--notice" role="status">
            <strong>EVENT</strong>
            <span>{props.notice}</span>
            <button type="button" onClick={props.onClearNotice}>DISMISS</button>
          </section>
        ) : null}
      </div>

      <section className="tui-screen-region" aria-busy={snapshotPending}>
        {normalizedPage === "home" ? (
          <div className="tui-home-grid">
            <Pane
              ref={(node) => { paneRefs.current[0] = node; }}
              title="MENU"
              className="tui-play-pane"
              focused={focusedPane === 0}
              onFocus={() => setFocusedPane(0)}
              onClick={() => focusPane(0)}
            >
              <MenuList items={playItems} selectedIndex={playIndex} onSelect={setPlayIndex} onActivate={activatePlayItem} />
            </Pane>

            <Pane
              ref={(node) => { paneRefs.current[1] = node; }}
              title="SYSTEM"
              focused={focusedPane === 1}
              onFocus={() => setFocusedPane(1)}
              onClick={() => focusPane(1)}
            >
              <KeyValueList rows={systemRows} />
            </Pane>

            <Pane
              ref={(node) => { paneRefs.current[2] = node; }}
              title="PROFILE"
              focused={focusedPane === 2}
              onFocus={() => setFocusedPane(2)}
              onClick={() => focusPane(2)}
            >
              <KeyValueList rows={profileRows} />
            </Pane>

            <Pane
              ref={(node) => { paneRefs.current[3] = node; }}
              title="SESSION"
              focused={focusedPane === 3}
              onFocus={() => setFocusedPane(3)}
              onClick={() => focusPane(3)}
            >
              <KeyValueList rows={sessionRows} />
              <InstanceManager instances={props.instances} busy={props.busy} onKill={props.onKillInstance} />
            </Pane>
          </div>
        ) : null}

        {normalizedPage === "installation" ? (
          <InstallationScreen
            index={installationIndex}
            setIndex={setInstallationIndex}
            snapshot={props.snapshot}
            installed={installed}
            optifineReady={optifineReady}
            busy={props.busy}
            onInstall={props.onInstall}
            onRefresh={props.onRefresh}
            onOpenSettings={() => {
              props.onNavigate("settings");
              setSettingsIndex(2);
            }}
          />
        ) : null}

        {normalizedPage === "account" ? (
          <AccountScreen
            isQaOfflineEdition={isQaOfflineEdition}
            snapshotPending={snapshotPending}
            snapshotUnavailable={snapshotUnavailable}
            accounts={accounts}
            selectedAccountId={selectedAccount?.id ?? null}
            activeAccountIds={activeAccountIds}
            skin={props.skin}
            skinLoading={props.skinLoading}
            qaUsername={props.qaUsername}
            busy={props.busy}
            loginCancelling={props.loginCancelling}
            developerTestProfileActive={developerTestProfileActive}
            developerSimulationActive={developerSimulationActive}
            onQaUsernameChange={props.onQaUsernameChange}
            onSaveQaProfile={props.onSaveQaProfile}
            onStartMicrosoftLogin={props.onStartMicrosoftLogin}
            onCancelMicrosoftLogin={props.onCancelMicrosoftLogin}
            onSelectAccount={props.onSelectAccount}
            onRemoveAccount={props.onRemoveAccount}
            onRunDeveloperSimulation={props.onRunDeveloperSimulation}
            onRefresh={props.onRefresh}
          />
        ) : null}

        {normalizedPage === "modules" ? (
          <ModulesScreen
            utilities={props.utilities}
            preferences={props.utilityPreferences}
            saveStatus={props.utilitySaveStatus}
            selectedId={selectedUtility?.id ?? "fps"}
            onSelect={setSelectedUtilityId}
            onChange={props.onUtilityPreferenceChange}
          />
        ) : null}

        {normalizedPage === "settings" ? (
          <SettingsScreen
            index={settingsIndex}
            setIndex={setSettingsIndex}
            settings={props.settings}
            snapshot={props.snapshot}
            optifineReady={optifineReady}
            optifineSourcePath={props.optifineSourcePath}
            busy={props.busy}
            onSettingsChange={props.onSettingsChange}
            onOptiFineSourcePathChange={props.onOptiFineSourcePathChange}
            onImportOptiFine={props.onImportOptiFine}
            onSave={props.onSaveSettings}
          />
        ) : null}

        {normalizedPage === "logs" ? (
          <LogsScreen
            category={logCategory}
            onCategoryChange={setLogCategory}
            lines={logLines}
            dataDirectory={props.snapshot?.dataDirectory ?? "Unavailable"}
            onRefresh={props.onRefresh}
          />
        ) : null}
      </section>

      <LaunchDock
        account={selectedAccount}
        accountRunning={selectedAccountRunning}
        activeLaunchCount={activeLaunchCount}
        ready={readyToLaunch}
        busy={props.busy}
        developerTestProfileActive={developerTestProfileActive}
        developerSimulationActive={developerSimulationActive}
        onLaunch={activateLaunch}
        onOpenAccounts={() => props.onNavigate("account")}
      />

      <CommandBar commands={normalizedPage === "home"
        ? ["ARROW navigate", "SHIFT+ARROW pane", "ENTER select", "L launch", "1-6 quick", "Q quit"]
        : ["ARROW navigate", "SHIFT+ARROW pane", "ESC back", "L launch", "1-6 quick", "Q quit"]}
      />

      {props.busy ? (
        <TaskOverlay
          busy={props.busy}
          progress={props.installProgress}
          account={selectedAccount}
          loginCancelling={props.loginCancelling}
          onCancelLogin={props.onCancelMicrosoftLogin}
        />
      ) : null}
    </main>
  );
}

function InstallationScreen(props: {
  index: number;
  setIndex: (index: number) => void;
  snapshot: TuiSnapshot | null;
  installed: boolean;
  optifineReady: boolean;
  busy: string | null;
  onInstall: () => void;
  onRefresh: () => void;
  onOpenSettings: () => void;
}) {
  const items: readonly MenuItem[] = [
    { id: "verify", shortcut: "V", label: "Verify installation" },
    { id: "repair", shortcut: "R", label: "Repair installation" },
    { id: "optifine", shortcut: "O", label: "Import OptiFine" },
    { id: "refresh", shortcut: "F", label: "Refresh state" },
  ];
  const activate = (index: number) => {
    if (props.busy !== null) return;
    if (index === 0 || index === 1) props.onInstall();
    if (index === 2) props.onOpenSettings();
    if (index === 3) props.onRefresh();
  };
  const rows: readonly KeyValueRow[] = [
    { label: "Minecraft", value: props.installed ? "READY / 1.8.9" : "MISSING", tone: props.installed ? "success" : "warning" },
    { label: "Forge", value: props.installed ? "READY / 11.15.1.2318" : "MISSING", tone: props.installed ? "success" : "warning" },
    { label: "OptiFine", value: props.optifineReady ? "READY / HD U M5" : "MISSING", tone: props.optifineReady ? "success" : "warning" },
    { label: "Opus Client", value: props.snapshot?.gameLaunchReady ? "READY" : "MISSING", tone: props.snapshot?.gameLaunchReady ? "success" : "warning" },
    { label: "Java", value: props.installed ? "READY / 8" : "MISSING", tone: props.installed ? "success" : "warning" },
  ];
  return (
    <div className="tui-page-grid">
      <Pane title="INSTALLATION">
        <MenuList items={items} selectedIndex={props.index} onSelect={props.setIndex} onActivate={activate} />
      </Pane>
      <Pane title="COMPONENTS">
        <KeyValueList rows={rows} />
        <div className="tui-detail-copy">
          <p>{props.snapshot?.minecraftStatus ?? "The local installation has not been checked."}</p>
          <p>{props.snapshot?.optifineStatus ?? "OptiFine status unavailable."}</p>
          <p>{props.snapshot?.javaStatus ?? "Java runtime status unavailable."}</p>
        </div>
      </Pane>
    </div>
  );
}

const SKIN_PX = 7;

type FaceRects = {
  front: [number, number, number, number];
  back: [number, number, number, number];
  right: [number, number, number, number];
  left: [number, number, number, number];
  top: [number, number, number, number];
  bottom: [number, number, number, number];
};

/// One cuboid body part: width/height/depth in skin pixels, its offset from the
/// model centre in skin pixels, and the UV rectangles for its six faces.
type BodyPart = {
  key: string;
  w: number;
  h: number;
  d: number;
  x: number;
  y: number;
  base: FaceRects;
  overlay?: FaceRects;
};

/// Classic Minecraft 1.8 64x64 UV layout. `armW` is 4 for the classic model and
/// 3 for the slim model; the left arm/leg use the dedicated 64x64 regions.
function bodyParts(slim: boolean): BodyPart[] {
  const a = slim ? 3 : 4;
  return [
    {
      key: "head",
      w: 8,
      h: 8,
      d: 8,
      x: 0,
      y: -12,
      base: {
        front: [8, 8, 8, 8],
        back: [24, 8, 8, 8],
        right: [0, 8, 8, 8],
        left: [16, 8, 8, 8],
        top: [8, 0, 8, 8],
        bottom: [16, 0, 8, 8],
      },
      overlay: {
        front: [40, 8, 8, 8],
        back: [56, 8, 8, 8],
        right: [32, 8, 8, 8],
        left: [48, 8, 8, 8],
        top: [40, 0, 8, 8],
        bottom: [48, 0, 8, 8],
      },
    },
    {
      key: "body",
      w: 8,
      h: 12,
      d: 4,
      x: 0,
      y: -2,
      base: {
        front: [20, 20, 8, 12],
        back: [32, 20, 8, 12],
        right: [16, 20, 4, 12],
        left: [28, 20, 4, 12],
        top: [20, 16, 8, 4],
        bottom: [28, 16, 8, 4],
      },
    },
    {
      key: "arm-right",
      w: a,
      h: 12,
      d: 4,
      x: -(4 + a / 2),
      y: -2,
      base: {
        right: [40, 20, 4, 12],
        front: [44, 20, a, 12],
        left: [44 + a, 20, 4, 12],
        back: [44 + a + 4, 20, a, 12],
        top: [44, 16, a, 4],
        bottom: [44 + a, 16, a, 4],
      },
    },
    {
      key: "arm-left",
      w: a,
      h: 12,
      d: 4,
      x: 4 + a / 2,
      y: -2,
      base: {
        right: [32, 52, 4, 12],
        front: [36, 52, a, 12],
        left: [36 + a, 52, 4, 12],
        back: [40 + a, 52, a, 12],
        top: [36, 48, a, 4],
        bottom: [36 + a, 48, a, 4],
      },
    },
    {
      key: "leg-right",
      w: 4,
      h: 12,
      d: 4,
      x: -2,
      y: 10,
      base: {
        right: [0, 20, 4, 12],
        front: [4, 20, 4, 12],
        left: [8, 20, 4, 12],
        back: [12, 20, 4, 12],
        top: [4, 16, 4, 4],
        bottom: [8, 16, 4, 4],
      },
    },
    {
      key: "leg-left",
      w: 4,
      h: 12,
      d: 4,
      x: 2,
      y: 10,
      base: {
        right: [16, 52, 4, 12],
        front: [20, 52, 4, 12],
        left: [24, 52, 4, 12],
        back: [28, 52, 4, 12],
        top: [20, 48, 4, 4],
        bottom: [24, 48, 4, 4],
      },
    },
  ];
}

/// Background style that samples one UV rectangle from the 64x64 skin sheet.
function faceTexture(url: string, rect: [number, number, number, number]): CSSProperties {
  const [x, y, w, h] = rect;
  return {
    backgroundImage: `url("${url}")`,
    backgroundRepeat: "no-repeat",
    backgroundSize: `${64 * SKIN_PX}px ${64 * SKIN_PX}px`,
    backgroundPosition: `-${x * SKIN_PX}px -${y * SKIN_PX}px`,
    width: `${w * SKIN_PX}px`,
    height: `${h * SKIN_PX}px`,
    imageRendering: "pixelated",
  };
}

/// Place a single cube face: centre it on the part origin, orient it, then push
/// it out by half of the perpendicular dimension so six faces form a solid box.
function faceTransform(face: keyof FaceRects, w: number, h: number, d: number): string {
  const halfW = (w * SKIN_PX) / 2;
  const halfH = (h * SKIN_PX) / 2;
  const halfD = (d * SKIN_PX) / 2;
  switch (face) {
    case "front":
      return `translate(-50%, -50%) translateZ(${halfD}px)`;
    case "back":
      return `translate(-50%, -50%) rotateY(180deg) translateZ(${halfD}px)`;
    case "right":
      return `translate(-50%, -50%) rotateY(-90deg) translateZ(${halfW}px)`;
    case "left":
      return `translate(-50%, -50%) rotateY(90deg) translateZ(${halfW}px)`;
    case "top":
      return `translate(-50%, -50%) rotateX(90deg) translateZ(${halfH}px)`;
    case "bottom":
      return `translate(-50%, -50%) rotateX(-90deg) translateZ(${halfH}px)`;
  }
}

function SkinBox(props: { url: string; part: BodyPart }) {
  const { part } = props;
  const faces: (keyof FaceRects)[] = ["front", "back", "right", "left", "top", "bottom"];
  const partTransform = `translate(-50%, -50%) translate3d(${part.x * SKIN_PX}px, ${part.y * SKIN_PX}px, 0)`;
  return (
    <div className="tui-skin-part" style={{ transform: partTransform }}>
      {faces.map((face) => (
        <span
          key={`base-${face}`}
          className="tui-skin-face"
          style={{ ...faceTexture(props.url, part.base[face]), transform: faceTransform(face, part.w, part.h, part.d) }}
        />
      ))}
      {part.overlay
        ? faces.map((face) => (
            <span
              key={`overlay-${face}`}
              className="tui-skin-face tui-skin-face-overlay"
              style={{ ...faceTexture(props.url, part.overlay![face]), transform: faceTransform(face, part.w + 0.5, part.h + 0.5, part.d + 0.5) }}
            />
          ))
        : null}
    </div>
  );
}

function SkinViewer(props: { skin: TuiAccountSkin | null; loading: boolean; username: string }) {
  const [rotX, setRotX] = useState(-12);
  const [rotY, setRotY] = useState(-24);
  const [spinning, setSpinning] = useState(true);
  const dragRef = useRef<{ pointerId: number; x: number; y: number; rotX: number; rotY: number } | null>(null);

  useEffect(() => {
    if (!spinning) return;
    let frame = 0;
    let previous = performance.now();
    const step = (now: number) => {
      const delta = now - previous;
      previous = now;
      setRotY((current) => current + delta * 0.03);
      frame = window.requestAnimationFrame(step);
    };
    frame = window.requestAnimationFrame(step);
    return () => window.cancelAnimationFrame(frame);
  }, [spinning]);

  const skinUrl = props.skin?.dataUrl ?? null;
  const slim = props.skin?.model === "slim";
  const parts = useMemo(() => bodyParts(slim), [slim]);
  const normalizedY = ((rotY % 360) + 360) % 360;
  const clampedX = Math.max(-90, Math.min(90, rotX));

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    dragRef.current = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, rotX, rotY };
    setSpinning(false);
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    setRotY(drag.rotY + (event.clientX - drag.x) * 0.6);
    setRotX(Math.max(-90, Math.min(90, drag.rotX - (event.clientY - drag.y) * 0.6)));
  };
  const endDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId === event.pointerId) dragRef.current = null;
  };

  return (
    <div className="tui-skin-viewer">
      <div
        className="tui-skin-stage"
        role="img"
        aria-label={`3D skin preview for ${props.username}`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      >
        {skinUrl ? (
          <div className="tui-skin-scene">
            <div
              className="tui-skin-root"
              style={{ transform: `rotateX(${clampedX}deg) rotateY(${normalizedY}deg)` }}
            >
              {parts.map((part) => (
                <SkinBox key={part.key} url={skinUrl} part={part} />
              ))}
            </div>
          </div>
        ) : (
          <p className="tui-empty-state">{props.loading ? "LOADING SKIN..." : "NO SKIN"}</p>
        )}
      </div>
      <div className="tui-skin-controls">
        <button type="button" className="tui-action-button" data-tui-nav-item onClick={() => setSpinning((value) => !value)}>
          {spinning ? "PAUSE SPIN" : "AUTO SPIN"}
        </button>
        <button
          type="button"
          className="tui-action-button"
          data-tui-nav-item
          onClick={() => { setRotX(-12); setRotY(-24); }}
        >
          RESET VIEW
        </button>
        <span className="tui-skin-hint">Drag to rotate{props.skin?.isDefault ? " · default skin" : ""}</span>
      </div>
    </div>
  );
}

function AccountScreen(props: {
  isQaOfflineEdition: boolean;
  snapshotPending: boolean;
  snapshotUnavailable: boolean;
  accounts: readonly TuiAccount[];
  selectedAccountId: string | null;
  activeAccountIds: ReadonlySet<string>;
  skin: TuiAccountSkin | null;
  skinLoading: boolean;
  qaUsername: string;
  busy: string | null;
  loginCancelling: boolean;
  developerTestProfileActive: boolean;
  developerSimulationActive: boolean;
  onQaUsernameChange: (value: string) => void;
  onSaveQaProfile: () => void;
  onStartMicrosoftLogin: () => void;
  onCancelMicrosoftLogin: () => void;
  onSelectAccount: (accountId: string) => void;
  onRemoveAccount: (accountId: string) => void;
  onRunDeveloperSimulation: () => void;
  onRefresh: () => void;
}) {
  const [pendingRemoveId, setPendingRemoveId] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [addMode, setAddMode] = useState<"choose" | "unofficial">("choose");
  const qaUsernameValid = /^[A-Za-z0-9_]{3,16}$/.test(props.qaUsername);
  const selectedAccount = props.accounts.find((account) => account.id === props.selectedAccountId) ?? null;
  const selectedRunning = selectedAccount ? props.activeAccountIds.has(selectedAccount.id) : false;
  useEffect(() => setPendingRemoveId(null), [props.selectedAccountId]);

  const closeAdd = useCallback(() => {
    setAddOpen(false);
    setAddMode("choose");
  }, []);

  useEffect(() => {
    if (props.busy === "offline-profile" && props.qaUsername === "") closeAdd();
  }, [closeAdd, props.busy, props.qaUsername]);

  const accountRows: readonly KeyValueRow[] = [
    { label: "Identity", value: selectedAccount?.username ?? "NONE", tone: selectedAccount?.ready ? "success" : "warning" },
    { label: "Prefix", value: selectedAccount ? `[${selectedAccount.badge.toUpperCase()}]` : "NONE", tone: selectedAccount ? "warning" : "muted" },
    { label: "Provider", value: selectedAccount?.kind === "microsoft" ? "MICROSOFT" : selectedAccount ? "OFFLINE" : "NONE" },
    { label: "Credential", value: selectedAccount?.ready ? "READY" : "UNAVAILABLE", tone: selectedAccount?.ready ? "success" : "warning" },
    { label: "Instance", value: selectedRunning ? "RUNNING" : "IDLE", tone: selectedRunning ? "warning" : "muted" },
    { label: "UUID", value: selectedAccount?.uuid ? compactUuid(selectedAccount.uuid) : "MIGRATES ON LAUNCH", tone: "muted" },
  ];

  return (
    <div className="tui-page-grid">
      <Pane title={`IDENTITIES / ${props.accounts.length}`}>
        <div className="tui-account-list" role="listbox" aria-label="Launch identities">
          {props.accounts.length === 0 ? <p className="tui-empty-state">NO IDENTITIES SAVED</p> : null}
          {props.accounts.map((account) => {
            const selected = account.id === props.selectedAccountId;
            const running = props.activeAccountIds.has(account.id);
            return (
              <button
                key={account.id}
                type="button"
                className={`tui-account-row ${selected ? "is-selected" : ""}`}
                role="option"
                aria-selected={selected}
                data-tui-nav-item
                disabled={props.busy !== null}
                onClick={() => props.onSelectAccount(account.id)}
              >
                <span className="tui-menu-caret" aria-hidden="true">{selected ? ">" : " "}</span>
                <span className={`tui-account-badge ${badgeToneClass(account.badge)}`}>[{account.badge.toUpperCase()}]</span>
                <span className="tui-account-name">{account.username}</span>
                <StatusText value={running ? "RUNNING" : account.ready ? "READY" : "RECONNECT"} tone={running ? "warning" : account.ready ? "success" : "warning"} />
              </button>
            );
          })}
        </div>
        <div className="tui-pane-actions tui-pane-actions--stacked">
          <button
            type="button"
            className="tui-action-button is-primary"
            data-tui-nav-item
            disabled={props.snapshotPending || props.busy !== null}
            onClick={props.snapshotUnavailable ? props.onRefresh : () => { setAddMode("choose"); setAddOpen(true); }}
          >
            {props.snapshotPending ? "CHECKING..." : props.snapshotUnavailable ? "REFRESH" : "ADD ACCOUNT"}
          </button>
          <button type="button" className="tui-action-button" data-tui-nav-item disabled={props.busy !== null} onClick={props.onRefresh}>REFRESH LIST</button>
        </div>
      </Pane>

      <Pane title="IDENTITY MANAGER">
        <div className="tui-identity-detail">
          <SkinViewer skin={props.skin} loading={props.skinLoading} username={selectedAccount?.username ?? "NONE"} />
          <div className="tui-identity-info">
            <KeyValueList rows={accountRows} />
            <div className="tui-account-manager">
              {selectedAccount ? (
                <button
                  type="button"
                  className="tui-action-button is-danger"
                  data-tui-nav-item
                  disabled={props.busy !== null || selectedRunning}
                  onClick={() => {
                    if (pendingRemoveId === selectedAccount.id) props.onRemoveAccount(selectedAccount.id);
                    else setPendingRemoveId(selectedAccount.id);
                  }}
                >
                  {selectedRunning ? "INSTANCE IS RUNNING" : pendingRemoveId === selectedAccount.id ? "CONFIRM REMOVE" : "REMOVE SELECTED"}
                </button>
              ) : <p className="tui-empty-state">SELECT AN IDENTITY</p>}
              {props.busy === "login" ? (
                <button type="button" className="tui-action-button" disabled={props.loginCancelling} onClick={props.onCancelMicrosoftLogin}>
                  {props.loginCancelling ? "CANCELLING..." : "CANCEL SIGN-IN"}
                </button>
              ) : null}
              {props.developerTestProfileActive ? (
                <button type="button" className="tui-action-button" data-tui-nav-item disabled={props.developerSimulationActive || props.busy !== null} onClick={props.onRunDeveloperSimulation}>
                  {props.developerSimulationActive ? "SIMULATION RUNNING" : "RUN DEVELOPER SIMULATION"}
                </button>
              ) : null}
            </div>
          </div>
        </div>
      </Pane>

      {addOpen ? (
        <div className="tui-modal-layer" role="dialog" aria-modal="true" aria-labelledby="tui-add-account-title" onClick={closeAdd}>
          <section className="tui-modal tui-add-account" onClick={(event) => event.stopPropagation()}>
            <header>
              <span id="tui-add-account-title">ADD ACCOUNT</span>
              <button type="button" className="tui-modal-action" onClick={closeAdd}>CLOSE</button>
            </header>
            {addMode === "choose" ? (
              <div className="tui-add-choice">
                <button
                  type="button"
                  className="tui-choice-card"
                  disabled={props.busy !== null || props.snapshotUnavailable}
                  onClick={() => { closeAdd(); props.onStartMicrosoftLogin(); }}
                >
                  <strong>OFFICIAL</strong>
                  <span>Microsoft account</span>
                  <span className="tui-choice-note">Java Edition ownership verified · real skin</span>
                </button>
                <button
                  type="button"
                  className="tui-choice-card"
                  disabled={props.busy !== null}
                  onClick={() => setAddMode("unofficial")}
                >
                  <strong>UNOFFICIAL</strong>
                  <span>Offline profile</span>
                  <span className="tui-choice-note">Username only · no online play</span>
                </button>
              </div>
            ) : (
              <div className="tui-form-stack">
                <label htmlFor="tui-qa-username">Unofficial username</label>
                <input
                  id="tui-qa-username"
                  type="text"
                  value={props.qaUsername}
                  minLength={3}
                  maxLength={16}
                  spellCheck={false}
                  autoFocus
                  placeholder="Minecraft username"
                  onChange={(event) => props.onQaUsernameChange(event.target.value)}
                  disabled={props.busy !== null}
                  onKeyDown={(event) => { if (event.key === "Enter" && qaUsernameValid) props.onSaveQaProfile(); }}
                />
                <p className={qaUsernameValid ? "tui-help is-valid" : "tui-help"}>3-16 letters, numbers, or underscores.</p>
                <div className="tui-modal-buttons">
                  <button type="button" className="tui-action-button" onClick={() => setAddMode("choose")} disabled={props.busy !== null}>BACK</button>
                  <button type="button" className="tui-action-button is-primary" disabled={!qaUsernameValid || props.busy !== null} onClick={props.onSaveQaProfile}>
                    ADD UNOFFICIAL PROFILE
                  </button>
                </div>
              </div>
            )}
          </section>
        </div>
      ) : null}
    </div>
  );
}

function ModulesScreen(props: {
  utilities: readonly TuiUtilityDefinition[];
  preferences: Record<TuiUtilityId, TuiUtilityPreference>;
  saveStatus: "loading" | "saved" | "saving" | "error";
  selectedId: TuiUtilityId;
  onSelect: (id: TuiUtilityId) => void;
  onChange: (id: TuiUtilityId, update: Partial<TuiUtilityPreference>) => void;
}) {
  const selectedIndex = Math.max(0, props.utilities.findIndex((utility) => utility.id === props.selectedId));
  const selected = props.utilities[selectedIndex];
  const preference = selected ? props.preferences[selected.id] : undefined;
  const items = props.utilities.map<MenuItem>((utility) => ({
    id: utility.id,
    label: utility.name,
    value: props.preferences[utility.id].enabled ? "ON" : "OFF",
    tone: props.preferences[utility.id].enabled ? "success" : "muted",
  }));
  return (
    <div className="tui-page-grid">
      <Pane title="MODULES" footer={<span>AUTOSAVE: {props.saveStatus.toUpperCase()}</span>}>
        <MenuList
          items={items}
          selectedIndex={selectedIndex}
          onSelect={(index) => {
            const utility = props.utilities[index];
            if (utility) props.onSelect(utility.id);
          }}
          onActivate={(index) => {
            const utility = props.utilities[index];
            if (utility) props.onChange(utility.id, { enabled: !props.preferences[utility.id].enabled });
          }}
        />
      </Pane>
      <Pane title="DETAILS">
        {selected && preference ? (
          <div className="tui-module-detail">
            <div className="tui-detail-heading">
              <div><span>{selected.category.toUpperCase()}</span><h3>{selected.name}</h3></div>
              <button type="button" className={`tui-toggle-button ${preference.enabled ? "is-on" : ""}`} onClick={() => props.onChange(selected.id, { enabled: !preference.enabled })}>
                {preference.enabled ? "ON" : "OFF"}
              </button>
            </div>
            <p>{selected.purpose}</p>
            <div className="tui-form-grid">
              <label>Anchor
                <select value={preference.anchor} onChange={(event) => props.onChange(selected.id, { anchor: event.target.value as TuiUtilityAnchor })}>
                  {(Object.entries(utilityAnchorLabels) as [TuiUtilityAnchor, string][]).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
                </select>
              </label>
              <label>Offset
                <input type="text" value={preference.offset} maxLength={24} onChange={(event) => props.onChange(selected.id, { offset: event.target.value })} />
              </label>
              <label>Scale <output>{preference.scale}%</output>
                <input type="range" min={50} max={150} step={5} value={preference.scale} onChange={(event) => props.onChange(selected.id, { scale: Number(event.target.value) })} />
              </label>
              <label>Opacity <output>{preference.opacity}%</output>
                <input type="range" min={25} max={100} step={5} value={preference.opacity} onChange={(event) => props.onChange(selected.id, { opacity: Number(event.target.value) })} />
              </label>
            </div>
          </div>
        ) : <p>No module selected.</p>}
      </Pane>
    </div>
  );
}

function SettingsScreen(props: {
  index: number;
  setIndex: (index: number) => void;
  settings: TuiSettings;
  snapshot: TuiSnapshot | null;
  optifineReady: boolean;
  optifineSourcePath: string;
  busy: string | null;
  onSettingsChange: (settings: TuiSettings) => void;
  onOptiFineSourcePathChange: (value: string) => void;
  onImportOptiFine: () => void;
  onSave: () => void;
}) {
  const items: readonly MenuItem[] = [
    { id: "memory", label: "Memory", value: `${props.settings.maxMemoryMib} MB` },
    { id: "launcher", label: "Launcher behavior", value: props.settings.closeLauncherOnGameStart ? "HIDE" : "STAY OPEN" },
    { id: "optifine", label: "OptiFine", value: props.optifineReady ? "READY" : "MISSING", tone: props.optifineReady ? "success" : "warning" },
    { id: "directory", label: "Game Directory", value: "VIEW" },
  ];
  return (
    <div className="tui-page-grid">
      <Pane title="SETTINGS">
        <MenuList items={items} selectedIndex={props.index} onSelect={props.setIndex} />
      </Pane>
      <Pane title="VALUE">
        {props.index === 0 ? (
          <div className="tui-setting-editor">
            <label htmlFor="tui-memory">Memory allocation</label>
            <output>{props.settings.maxMemoryMib} MB</output>
            <input
              id="tui-memory"
              type="range"
              min={512}
              max={8192}
              step={256}
              value={props.settings.maxMemoryMib}
              disabled={props.busy !== null}
              onChange={(event) => props.onSettingsChange({ ...props.settings, maxMemoryMib: Number(event.target.value) })}
            />
            <div className="tui-range-labels"><span>512 MB</span><span>8192 MB</span></div>
          </div>
        ) : null}
        {props.index === 1 ? (
          <div className="tui-setting-editor">
            <p>Keep launcher visibility predictable after Minecraft starts.</p>
            <label className="tui-checkbox-row">
              <input
                type="checkbox"
                checked={props.settings.closeLauncherOnGameStart}
                disabled={props.busy !== null}
                onChange={(event) => props.onSettingsChange({ ...props.settings, closeLauncherOnGameStart: event.target.checked })}
              />
              <span>Hide Opus Launcher while Minecraft is open</span>
            </label>
          </div>
        ) : null}
        {props.index === 2 ? (
          <div className="tui-setting-editor">
            <KeyValueList rows={[{ label: "Required", value: "OptiFine 1.8.9 HD U M5" }, { label: "Status", value: props.optifineReady ? "READY" : "MISSING", tone: props.optifineReady ? "success" : "warning" }]} />
            <label htmlFor="tui-optifine-path">Local JAR path</label>
            <input
              id="tui-optifine-path"
              type="text"
              value={props.optifineSourcePath}
              disabled={props.optifineReady || props.busy !== null}
              placeholder="/path/to/OptiFine_1.8.9_HD_U_M5.jar"
              onChange={(event) => props.onOptiFineSourcePathChange(event.target.value)}
            />
            <button type="button" className="tui-action-button" disabled={props.optifineReady || props.busy !== null || props.optifineSourcePath.trim().length === 0} onClick={props.onImportOptiFine}>
              {props.busy === "optifine" ? "VERIFYING..." : "IMPORT AND VERIFY"}
            </button>
          </div>
        ) : null}
        {props.index === 3 ? (
          <div className="tui-setting-editor">
            <label>Opus data directory</label>
            <code>{props.snapshot?.dataDirectory ?? "Unavailable"}</code>
            <p>This isolated directory does not modify the normal .minecraft installation.</p>
          </div>
        ) : null}
        <div className="tui-pane-actions">
          <button type="button" className="tui-action-button is-primary" disabled={props.busy !== null} onClick={props.onSave}>
            {props.busy === "settings" ? "SAVING..." : "SAVE SETTINGS"}
          </button>
        </div>
      </Pane>
    </div>
  );
}

function LogsScreen(props: {
  category: (typeof logCategories)[number];
  onCategoryChange: (category: (typeof logCategories)[number]) => void;
  lines: readonly string[];
  dataDirectory: string;
  onRefresh: () => void;
}) {
  const items = logCategories.map<MenuItem>((category) => ({ id: category, label: category, value: category === props.category ? "ACTIVE" : undefined }));
  const selectedIndex = logCategories.indexOf(props.category);
  return (
    <div className="tui-page-grid">
      <Pane title="LOGS">
        <MenuList
          items={items}
          selectedIndex={selectedIndex}
          onSelect={(index) => props.onCategoryChange(logCategories[index] ?? "Launcher")}
          onActivate={(index) => props.onCategoryChange(logCategories[index] ?? "Launcher")}
        />
        <button type="button" className="tui-action-button" onClick={props.onRefresh}>REFRESH</button>
      </Pane>
      <Pane title={props.category.toUpperCase()} footer={<span>DATA: {props.dataDirectory}</span>}>
        <LogView lines={props.lines} />
      </Pane>
    </div>
  );
}

function InstanceManager(props: {
  instances: readonly TuiInstance[];
  busy: string | null;
  onKill: (sessionId: string) => void;
}) {
  return (
    <div className="tui-instance-manager">
      <div className="tui-instance-manager-head">
        <span>ACTIVE INSTANCES</span>
        <strong>{String(props.instances.length).padStart(2, "0")}</strong>
      </div>
      {props.instances.length === 0 ? (
        <p className="tui-instance-empty">No running instances.</p>
      ) : (
        <ul className="tui-instance-list">
          {props.instances.map((instance) => (
            <li key={instance.sessionId} className="tui-instance-row">
              <span className={`tui-account-badge ${badgeToneClass(instance.badge)}`}>[{instance.badge.toUpperCase()}]</span>
              <span className="tui-instance-name" title={instance.title}>{instance.username}</span>
              <StatusText value="RUNNING" tone="warning" />
              <button
                type="button"
                className="tui-instance-kill"
                data-tui-nav-item
                disabled={props.busy !== null}
                onClick={() => props.onKill(instance.sessionId)}
              >
                KILL
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function LaunchDock(props: {
  account: TuiAccount | null;
  accountRunning: boolean;
  activeLaunchCount: number;
  ready: boolean;
  busy: string | null;
  developerTestProfileActive: boolean;
  developerSimulationActive: boolean;
  onLaunch: () => void;
  onOpenAccounts: () => void;
}) {
  const launchLabel = props.developerTestProfileActive
    ? props.developerSimulationActive ? "SIMULATION RUNNING" : "RUN SIMULATION"
    : props.accountRunning ? "INSTANCE RUNNING"
      : props.busy === "launch" ? "LAUNCHING..."
        : props.ready ? "LAUNCH MINECRAFT" : "OPEN SETUP";
  return (
    <section className="tui-launch-dock" aria-label="Launch Minecraft">
      <button type="button" className="tui-launch-identity" onClick={props.onOpenAccounts}>
        {props.account ? <span className={`tui-account-badge ${badgeToneClass(props.account.badge)}`}>[{props.account.badge.toUpperCase()}]</span> : <span className="tui-account-badge">[NONE]</span>}
        <strong>{props.account?.username ?? "SELECT AN IDENTITY"}</strong>
        <StatusText value={props.accountRunning ? "RUNNING" : props.account?.ready ? "READY" : "REQUIRED"} tone={props.accountRunning || props.account?.ready ? "warning" : "muted"} />
      </button>
      <div className="tui-launch-summary">
        <span>ACTIVE INSTANCES</span>
        <strong>{String(props.activeLaunchCount).padStart(2, "0")}</strong>
      </div>
      <button
        type="button"
        className="tui-launch-button"
        disabled={props.busy !== null || props.accountRunning || props.developerSimulationActive}
        onClick={props.onLaunch}
      >
        <span aria-hidden="true">&gt;</span>
        {launchLabel}
      </button>
    </section>
  );
}

function TaskOverlay(props: {
  busy: string;
  progress: TuiInstallProgress | null;
  account: TuiAccount | null;
  loginCancelling: boolean;
  onCancelLogin: () => void;
}) {
  const task = taskPresentation(props.busy, props.account);
  const installing = props.busy === "install";
  const progress = props.progress ?? {
    phase: "Starting installer",
    completedFiles: 0,
    totalFiles: 0,
    downloadedFiles: 0,
    cachedFiles: 0,
  };
  const percent = progress.totalFiles > 0
    ? Math.min(100, Math.round((progress.completedFiles / progress.totalFiles) * 100))
    : 0;
  const lines = installing ? [
    `phase.............................. ${progress.phase}`,
    `verified........................... ${progress.cachedFiles}`,
    `downloaded......................... ${progress.downloadedFiles}`,
    progress.totalFiles > 0
      ? `files.............................. ${progress.completedFiles}/${progress.totalFiles}`
      : "files.............................. PREPARING",
  ] : [task.detail];
  return (
    <div className="tui-modal-layer" role="dialog" aria-modal="true" aria-labelledby="tui-task-title">
      <section className="tui-modal">
        <header><span id="tui-task-title">{task.title}</span><StatusText value="RUNNING" tone="warning" /></header>
        <LogView lines={lines} />
        {installing ? <ProgressBar value={percent} label="Installation progress" /> : (
          <div className="tui-activity" role="progressbar" aria-label={task.title}><span /></div>
        )}
        <footer>
          <span>{task.footer}</span>
          {props.busy === "login" ? (
            <button type="button" className="tui-modal-action" disabled={props.loginCancelling} onClick={props.onCancelLogin}>
              {props.loginCancelling ? "CANCELLING..." : "CANCEL"}
            </button>
          ) : null}
        </footer>
      </section>
    </div>
  );
}

function taskPresentation(busy: string, account: TuiAccount | null) {
  const identity = account ? `[${account.badge.toUpperCase()}] ${account.username}` : "selected identity";
  switch (busy) {
    case "install":
      return { title: "INSTALL / VERIFY", detail: "Preparing managed runtime", footer: "Keep Opus Launcher open until verification completes." };
    case "launch":
      return { title: "LAUNCHING MINECRAFT", detail: `Preparing isolated instance for ${identity}`, footer: "Authenticating, verifying mods and handing off to Java." };
    case "login":
      return { title: "MICROSOFT SIGN-IN", detail: "Waiting for the secure browser sign-in to complete", footer: "Finish sign-in in your browser." };
    case "optifine":
      return { title: "VERIFYING OPTIFINE", detail: "Checking the selected JAR and importing it into Opus", footer: "The source file will not be modified." };
    case "settings":
      return { title: "SAVING SETTINGS", detail: "Writing launcher settings atomically", footer: "Settings are being committed." };
    case "offline-profile":
      return { title: "ADDING UNOFFICIAL PROFILE", detail: "Creating an isolated offline identity", footer: "The profile will appear in the shared launch list." };
    case "account-select":
      return { title: "SELECTING IDENTITY", detail: `Selecting ${identity}`, footer: "The next launch will use this identity." };
    case "account-remove":
      return { title: "REMOVING IDENTITY", detail: `Removing ${identity}`, footer: "Saved credentials for this identity are being deleted." };
    default:
      return { title: "OPUS LAUNCHER", detail: `Running ${busy}`, footer: "Please wait for the operation to complete." };
  }
}

function badgeToneClass(badge: string) {
  if (badge.toLowerCase() === "premium") return "is-premium";
  if (badge.toLowerCase() === "official") return "is-official";
  if (badge.toLowerCase() === "unofficial") return "is-unofficial";
  return "is-generic";
}

function compactUuid(uuid: string) {
  const compact = uuid.replaceAll("-", "");
  return compact.length > 12 ? `${compact.slice(0, 6)}...${compact.slice(-6)}` : compact;
}

function buildLogLines(props: LauncherTuiProps, runtimeState: string): string[] {
  const lines = [
    `launcher snapshot.................. ${props.snapshot ? "OK" : "UNAVAILABLE"}`,
    `runtime state...................... ${runtimeState}`,
    `minecraft 1.8.9.................... ${props.snapshot?.minecraftReady ? "READY" : "MISSING"}`,
    `forge 11.15.1.2318................. ${props.snapshot?.minecraftReady ? "READY" : "MISSING"}`,
    `optifine HD U M5................... ${props.snapshot?.optifineReady ? "READY" : "MISSING"}`,
    `java runtime....................... ${props.snapshot?.javaStatus ?? "UNKNOWN"}`,
    `identities......................... ${props.snapshot?.accounts.length ?? 0}`,
    `selected identity.................. ${props.snapshot?.accounts.find((account) => account.id === props.snapshot?.selectedAccountId)?.username ?? "NONE"}`,
    `active instances................... ${props.snapshot?.activeLaunches ?? 0}`,
  ];
  if (props.busy) lines.push(`active operation................... ${props.busy.toUpperCase()}`);
  if (props.notice) lines.push(`event.............................. ${props.notice}`);
  if (props.error) lines.push(`error.............................. ${friendlyError(props.error)}`);
  return lines;
}

function friendlyError(error: string) {
  const registrationPrefix = "Minecraft Services rejected Opus's application registration";
  if (error.includes(registrationPrefix)) {
    return "Microsoft sign-in completed, but Opus's application ID is still awaiting Minecraft Services approval.";
  }
  return error;
}
