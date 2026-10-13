import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { AccountHeadAvatar } from "./AccountHeadAvatar";
import { UTILITY_SCALE_MIN, UTILITY_SCALE_MAX } from "../utilitySettings";
import "./liquid-glass.css";

export type TuiPage = "home" | "installation" | "account" | "modules" | "utilities" | "settings" | "logs";
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
  gameLaunchStatus: string;
  buildEdition: "unified";
  offlineProfile: { username: string; valid: boolean } | null;
  accounts: TuiAccount[];
  selectedAccountId: string | null;
  activeLaunches: number;
  activeAccountIds: string[];
  developerTestProfile: { available: boolean; active: boolean; simulationActive: boolean };
};

export type LaunchProfileType = "standalone" | "forge";
export type TuiSettings = {
  maxMemoryMib: number;
  closeLauncherOnGameStart: boolean;
  launchProfileType?: LaunchProfileType;
};
export type TuiInstallProgress = {
  phase: string;
  completedFiles: number;
  totalFiles: number;
  downloadedFiles: number;
  cachedFiles: number;
};

export type LauncherTheme = "azure" | "emerald" | "violet" | "amber" | "rose" | "monochrome";
export type LauncherBackdrop = "video" | "overworld" | "filament" | "void";
export type LauncherAppearance = {
  theme: LauncherTheme;
  backdrop: LauncherBackdrop;
  glassBlur: number;
  glassOpacity: number;
};

const DEFAULT_APPEARANCE: LauncherAppearance = { theme: "azure", backdrop: "video", glassBlur: 14, glassOpacity: 76 };
const navigation: readonly { id: TuiPage; label: string }[] = [
  { id: "home", label: "Home" },
  { id: "installation", label: "Install" },
  { id: "account", label: "Account" },
  { id: "modules", label: "Modules" },
  { id: "utilities", label: "Utilities" },
  { id: "settings", label: "Settings" },
  { id: "logs", label: "Logs" },
];
const themeOptions: readonly { id: LauncherTheme; label: string }[] = [
  { id: "azure", label: "Slate" },
  { id: "emerald", label: "Moss" },
  { id: "violet", label: "Dusk" },
  { id: "amber", label: "Sand" },
  { id: "rose", label: "Clay" },
  { id: "monochrome", label: "Mono" },
];
const backdropOptions: readonly { id: LauncherBackdrop; label: string; detail: string }[] = [
  { id: "video", label: "Night sky", detail: "Quiet motion" },
  { id: "overworld", label: "Overworld", detail: "Still image" },
  { id: "filament", label: "Filament", detail: "Abstract still" },
  { id: "void", label: "No media", detail: "Solid canvas" },
];
const logCategories = ["Launcher", "Authentication", "Installation", "Minecraft", "Opus"] as const;
type LogCategory = (typeof logCategories)[number];
type StatusTone = "normal" | "muted" | "success" | "warning" | "danger";

function loadStoredAppearance(): LauncherAppearance {
  try {
    const raw = localStorage.getItem("opus_launcher_appearance_v1");
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<LauncherAppearance>;
      return {
        theme: parsed.theme ?? DEFAULT_APPEARANCE.theme,
        backdrop: parsed.backdrop ?? DEFAULT_APPEARANCE.backdrop,
        glassBlur: typeof parsed.glassBlur === "number" ? parsed.glassBlur : DEFAULT_APPEARANCE.glassBlur,
        glassOpacity: typeof parsed.glassOpacity === "number" ? parsed.glassOpacity : DEFAULT_APPEARANCE.glassOpacity,
      };
    }
  } catch {
    // Storage is optional in the browser preview and hardened runtimes.
  }
  return DEFAULT_APPEARANCE;
}

export type LauncherTuiProps = {
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
  skinCache?: Record<string, TuiAccountSkin>;
  skinLoading: boolean;
  offlineUsername: string;
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
  onSaveOfflineProfile: () => void;
  onSelectAccount: (accountId: string) => void;
  onRemoveAccount: (accountId: string) => void;
  onRunDeveloperSimulation: () => void;
  onQuit: () => void;
  onClearError: () => void;
  onClearNotice: () => void;
  onSettingsChange: (settings: TuiSettings) => void;
  onOfflineUsernameChange: (value: string) => void;
  onOptiFineSourcePathChange: (value: string) => void;
  onUtilityPreferenceChange: (id: TuiUtilityId, update: Partial<TuiUtilityPreference>) => void;
};

export default function LauncherTui(props: LauncherTuiProps) {
  const [appearance, setAppearance] = useState<LauncherAppearance>(loadStoredAppearance);
  const [selectedUtilityId, setSelectedUtilityId] = useState<TuiUtilityId>(props.utilities[0]?.id ?? "fps");
  const [logCategory, setLogCategory] = useState<LogCategory>("Launcher");
  const [copyLogStatus, setCopyLogStatus] = useState<"idle" | "copied" | "failed">("idle");
  const videoRef = useRef<HTMLVideoElement | null>(null);

  useEffect(() => {
    try { localStorage.setItem("opus_launcher_appearance_v1", JSON.stringify(appearance)); } catch { /* optional */ }
  }, [appearance]);
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const motionPreference = window.matchMedia("(prefers-reduced-motion: reduce)");
    const syncPlayback = () => {
      if (!document.hidden && !motionPreference.matches) void video.play().catch(() => undefined);
      else video.pause();
    };
    syncPlayback();
    document.addEventListener("visibilitychange", syncPlayback);
    motionPreference.addEventListener("change", syncPlayback);
    return () => {
      document.removeEventListener("visibilitychange", syncPlayback);
      motionPreference.removeEventListener("change", syncPlayback);
      video.pause();
    };
  }, [appearance.backdrop, props.page]);
  useEffect(() => {
    if (!props.utilities.some((utility) => utility.id === selectedUtilityId) && props.utilities[0]) setSelectedUtilityId(props.utilities[0].id);
  }, [props.utilities, selectedUtilityId]);

  const snapshotPending = props.snapshot === null && props.error === null;
  const snapshotUnavailable = props.snapshot === null && props.error !== null;
  const developerProfile = props.snapshot?.developerTestProfile;
  const developerTestProfileActive = developerProfile?.active ?? false;
  const developerSimulationActive = developerProfile?.simulationActive ?? false;
  const minecraftReady = Boolean(props.snapshot?.minecraftReady) && !developerTestProfileActive;
  const optifineReady = Boolean(props.snapshot?.optifineReady) && !developerTestProfileActive;
  const gameLaunchReady = Boolean(props.snapshot?.gameLaunchReady) && !developerTestProfileActive;
  const opusClientVerified = minecraftReady && optifineReady && gameLaunchReady;
  const accounts = props.snapshot?.accounts ?? [];
  const selectedAccount = accounts.find((account) => account.id === props.snapshot?.selectedAccountId)
    ?? accounts.find((account) => account.selected) ?? accounts[0] ?? null;
  const selectedAccountSkin = selectedAccount ? (props.skinCache?.[selectedAccount.id] ?? props.skin) : null;
  const activeAccountIds = useMemo(
    () => new Set([...(props.snapshot?.activeAccountIds ?? []), ...Object.keys(props.runningSessions)]),
    [props.runningSessions, props.snapshot?.activeAccountIds],
  );
  const activeLaunchCount = activeAccountIds.size;
  const selectedAccountRunning = selectedAccount ? activeAccountIds.has(selectedAccount.id) : false;
  const profileReady = Boolean(selectedAccount?.ready) && !developerTestProfileActive;
  const readyToLaunch = opusClientVerified && profileReady;
  const canLaunch = readyToLaunch && !selectedAccountRunning && props.busy === null;
  const enabledUtilityCount = useMemo(
    () => Object.values(props.utilityPreferences).filter((preference) => preference.enabled).length,
    [props.utilityPreferences],
  );
  const selectedUtility = props.utilities.find((utility) => utility.id === selectedUtilityId) ?? props.utilities[0] ?? null;
  const selectedUtilityPreference = selectedUtility ? props.utilityPreferences[selectedUtility.id] : null;
  const runtimeState = snapshotPending ? "Checking" : snapshotUnavailable ? "Unavailable"
    : developerTestProfileActive ? (developerSimulationActive ? "Simulating" : "Developer mode")
      : activeLaunchCount > 0 ? `${activeLaunchCount} running` : readyToLaunch ? "Ready" : "Setup required";
  const runtimeTone: StatusTone = snapshotUnavailable ? "danger" : snapshotPending ? "muted"
    : readyToLaunch || activeLaunchCount > 0 ? "success" : "warning";
  const primaryLabel = props.busy ? `${sentenceCase(props.busy)}…` : selectedAccountRunning ? "In game"
    : developerTestProfileActive ? "Review developer mode" : canLaunch ? "Play"
      : !opusClientVerified ? "Review installation" : !profileReady ? "Choose account" : "Play";

  const handlePrimaryAction = useCallback(() => {
    if (developerTestProfileActive) props.onNavigate("settings");
    else if (canLaunch && selectedAccount) props.onLaunch(selectedAccount.id);
    else if (!opusClientVerified) props.onNavigate("installation");
    else if (!profileReady) props.onNavigate("account");
  }, [canLaunch, developerTestProfileActive, opusClientVerified, profileReady, props, selectedAccount]);

  const logLines = useMemo(
    () => buildLogLines(props, runtimeState, opusClientVerified, enabledUtilityCount),
    [enabledUtilityCount, opusClientVerified, props, runtimeState],
  );
  const handleCopyLogs = useCallback(async () => {
    try { await navigator.clipboard.writeText(logLines.join("\n")); setCopyLogStatus("copied"); }
    catch { setCopyLogStatus("failed"); }
    window.setTimeout(() => setCopyLogStatus("idle"), 2500);
  }, [logLines]);
  const dynamicStyles = {
    "--media-softness": `${Math.max(0, Math.min(1.25, (appearance.glassBlur - 4) * 0.0625))}px`,
    "--media-opacity": String(Math.max(0.58, Math.min(0.95, appearance.glassOpacity / 100))),
  } as CSSProperties;

  return (
    <div className="liquid-launcher" data-theme={appearance.theme} style={dynamicStyles}>
      <header className="liquid-header">
        <div className="liquid-header__inner">
          <button type="button" className="launcher-brand" onClick={() => props.onNavigate("home")} aria-label="Open Home">
            <img src="/brand/opus-woven-o-c3.svg" alt="" draggable={false} />
            <span><strong>Opus</strong><small>Launcher</small></span>
          </button>
          <nav className="liquid-nav" aria-label="Launcher navigation">
            {navigation.map((item) => (
              <button key={item.id} type="button" className={`liquid-nav__tab ${props.page === item.id ? "is-active" : ""}`}
                aria-label={item.label} aria-current={props.page === item.id ? "page" : undefined} onClick={() => props.onNavigate(item.id)}>
                <span>{item.label}</span>
                {item.id === "account" && accounts.length > 0 ? <b aria-hidden="true">{accounts.length}</b> : null}
                {item.id === "modules" && enabledUtilityCount > 0 ? <b aria-hidden="true">{enabledUtilityCount}</b> : null}
              </button>
            ))}
          </nav>
          <div className="liquid-header__meta">
            <StatusBadge tone={runtimeTone}>{runtimeState}</StatusBadge>
            <button type="button" className="account-pill" onClick={() => props.onNavigate("account")} aria-label="Open account">
              <AccountHeadAvatar skinDataUrl={selectedAccountSkin?.dataUrl} username={selectedAccount?.username} size={24} />
              <span className="account-username">{selectedAccount?.username ?? "No account"}</span>
            </button>
            <button type="button" className="icon-button" onClick={props.onRefresh} aria-label="Refresh launcher state"><RefreshIcon /></button>
          </div>
        </div>
      </header>

      <main className="liquid-body" aria-busy={snapshotPending || props.busy !== null}>
        <StateMessages snapshotPending={snapshotPending} snapshotUnavailable={snapshotUnavailable} error={props.error}
          notice={props.notice} busy={props.busy} loginCancelling={props.loginCancelling} onRefresh={props.onRefresh}
          onClearError={props.onClearError} onClearNotice={props.onClearNotice} onCancelMicrosoftLogin={props.onCancelMicrosoftLogin} />

        <div key={props.page} className="page-viewport" data-page={props.page}>
        {props.page === "home" ? <HomePage appearance={appearance} videoRef={videoRef} runtimeState={runtimeState}
          runtimeTone={runtimeTone} snapshotPending={snapshotPending} snapshotUnavailable={snapshotUnavailable} snapshot={props.snapshot}
          settings={props.settings} selectedAccount={selectedAccount} selectedAccountSkin={selectedAccountSkin} accounts={accounts}
          activeLaunchCount={activeLaunchCount} instances={props.instances} opusClientVerified={opusClientVerified} profileReady={profileReady}
          primaryLabel={primaryLabel} primaryDisabled={props.busy !== null || selectedAccountRunning || snapshotPending || snapshotUnavailable}
          busy={props.busy} onPrimaryAction={handlePrimaryAction} onNavigate={props.onNavigate} onSelectAccount={props.onSelectAccount}
          onKillInstance={props.onKillInstance} onSettingsChange={props.onSettingsChange} /> : null}

        {props.page === "installation" ? <InstallationPage snapshot={props.snapshot} minecraftReady={minecraftReady}
          optifineReady={optifineReady} gameLaunchReady={gameLaunchReady} busy={props.busy} installProgress={props.installProgress}
          optifineSourcePath={props.optifineSourcePath} onBack={props.onBack} onRefresh={props.onRefresh} onInstall={props.onInstall}
          onImportOptiFine={props.onImportOptiFine} onOptiFineSourcePathChange={props.onOptiFineSourcePathChange} /> : null}

        {props.page === "account" ? <AccountPage accounts={accounts} selectedAccount={selectedAccount} activeAccountIds={activeAccountIds}
          skin={props.skin} skinCache={props.skinCache} skinLoading={props.skinLoading} snapshotUnavailable={snapshotUnavailable}
          offlineUsername={props.offlineUsername} busy={props.busy} loginCancelling={props.loginCancelling} developerProfile={developerProfile}
          onBack={props.onBack} onOfflineUsernameChange={props.onOfflineUsernameChange} onSaveOfflineProfile={props.onSaveOfflineProfile}
          onStartMicrosoftLogin={props.onStartMicrosoftLogin} onCancelMicrosoftLogin={props.onCancelMicrosoftLogin}
          onSelectAccount={props.onSelectAccount} onRemoveAccount={props.onRemoveAccount} onRunDeveloperSimulation={props.onRunDeveloperSimulation} /> : null}

        {props.page === "modules" ? <ModulesPage utilities={props.utilities} preferences={props.utilityPreferences}
          saveStatus={props.utilitySaveStatus} enabledCount={enabledUtilityCount} onBack={props.onBack}
          onChange={props.onUtilityPreferenceChange} onOpenUtilities={(id) => { setSelectedUtilityId(id); props.onNavigate("utilities"); }} /> : null}

        {props.page === "utilities" ? <UtilitiesPage utilities={props.utilities} preferences={props.utilityPreferences}
          saveStatus={props.utilitySaveStatus} selected={selectedUtility} preference={selectedUtilityPreference} onBack={props.onBack}
          onSelect={setSelectedUtilityId} onChange={props.onUtilityPreferenceChange} /> : null}

        {props.page === "settings" ? <SettingsPage settings={props.settings} snapshot={props.snapshot} appearance={appearance}
          busy={props.busy} developerProfile={developerProfile} onBack={props.onBack} onAppearanceChange={setAppearance}
          onSettingsChange={props.onSettingsChange} onSaveSettings={props.onSaveSettings}
          onRunDeveloperSimulation={props.onRunDeveloperSimulation} onQuit={props.onQuit} /> : null}

        {props.page === "logs" ? <LogsPage category={logCategory} lines={logLines}
          dataDirectory={props.snapshot?.dataDirectory ?? "Unavailable"} copyStatus={copyLogStatus} onBack={props.onBack}
          onCategoryChange={setLogCategory} onCopy={handleCopyLogs} onRefresh={props.onRefresh} /> : null}
        </div>
      </main>
    </div>
  );
}

function StateMessages(props: {
  snapshotPending: boolean; snapshotUnavailable: boolean; error: string | null; notice: string | null;
  busy: string | null; loginCancelling: boolean; onRefresh: () => void; onClearError: () => void;
  onClearNotice: () => void; onCancelMicrosoftLogin: () => void;
}) {
  return <div className="state-stack" aria-live="polite">
    {props.snapshotPending ? <div className="state-banner" role="status"><span className="state-banner__marker" aria-hidden="true" />
      <div><strong>Checking launcher readiness</strong><span>Reading installation, account, and runtime state.</span></div></div> : null}
    {props.snapshotUnavailable ? <div className="state-banner is-danger" role="alert"><span className="state-banner__marker" aria-hidden="true" />
      <div><strong>Launcher state unavailable</strong><span>{friendlyError(props.error ?? "The local service did not respond.")}</span></div>
      <button type="button" className="button-secondary button-compact" onClick={props.onRefresh}>Try again</button>
      <button type="button" className="button-quiet button-compact" onClick={props.onClearError}>Dismiss</button></div> : null}
    {props.error && !props.snapshotUnavailable ? <div className="state-banner is-danger" role="alert"><span className="state-banner__marker" aria-hidden="true" />
      <div><strong>Action failed</strong><span>{friendlyError(props.error)}</span></div>
      <button type="button" className="button-quiet button-compact" onClick={props.onClearError}>Dismiss</button></div> : null}
    {props.notice ? <div className="state-banner is-success" role="status"><span className="state-banner__marker" aria-hidden="true" />
      <div><strong>Launcher updated</strong><span>{props.notice}</span></div>
      <button type="button" className="button-quiet button-compact" onClick={props.onClearNotice}>Dismiss</button></div> : null}
    {props.busy ? <div className="state-banner is-busy" role="status"><span className="state-banner__marker" aria-hidden="true" />
      <div><strong>{sentenceCase(props.busy)}</strong><span>The current operation must finish before another starts.</span></div>
      {props.busy === "logging in" ? <button type="button" className="button-danger button-compact" disabled={props.loginCancelling} onClick={props.onCancelMicrosoftLogin}>
        {props.loginCancelling ? "Cancelling…" : "Cancel sign-in"}</button> : null}</div> : null}
  </div>;
}

function HomePage(props: {
  appearance: LauncherAppearance; videoRef: RefObject<HTMLVideoElement | null>; runtimeState: string; runtimeTone: StatusTone;
  snapshotPending: boolean; snapshotUnavailable: boolean; snapshot: TuiSnapshot | null; settings: TuiSettings;
  selectedAccount: TuiAccount | null; selectedAccountSkin: TuiAccountSkin | null; accounts: readonly TuiAccount[];
  activeLaunchCount: number; instances: readonly TuiInstance[]; opusClientVerified: boolean; profileReady: boolean;
  primaryLabel: string; primaryDisabled: boolean; busy: string | null; onPrimaryAction: () => void;
  onNavigate: (page: TuiPage) => void; onSelectAccount: (accountId: string) => void; onKillInstance: (sessionId: string) => void;
  onSettingsChange?: (settings: TuiSettings) => void;
}) {
  const isForge = props.settings.launchProfileType === "forge";
  return <section className="launcher-page home-page" aria-labelledby="home-title">
    <PageHeader id="home-title" eyebrow="Opus Client" title="Ready to play"
      aside={<StatusBadge tone={props.runtimeTone}>{props.runtimeState}</StatusBadge>} />
    <div className="home-play-card surface-panel">
      <div className={`home-media is-${props.appearance.backdrop}`}>
        {props.appearance.backdrop === "video" ? <video ref={props.videoRef} loop muted playsInline preload="metadata"
          poster="/media/opus-night-sky-poster-v1.jpg" aria-hidden="true"><source src="/media/opus-night-sky-v1.mp4" type="video/mp4" /></video> : null}
        <div className="home-media__identity"><img src="/brand/opus-woven-o-c3.svg" alt="" draggable={false} />
          <span><strong>Opus</strong><small>Competitive intelligence client</small></span></div>
      </div>
      <div className="home-play-panel">
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <div className="section-kicker">Launch profile</div>
          <div className="preset-row" style={{ margin: 0, gap: "4px" }}>
            <button
              type="button"
              className={!isForge ? "is-active" : ""}
              disabled={props.busy !== null}
              style={{ padding: "3px 8px", fontSize: "0.75rem", borderRadius: "4px" }}
              title="Standard: Clean standalone loader (Lunar style)"
              onClick={() => props.onSettingsChange?.({ ...props.settings, launchProfileType: "standalone" })}
            >
              Standard
            </button>
            <button
              type="button"
              className={isForge ? "is-active" : ""}
              disabled={props.busy !== null}
              style={{ padding: "3px 8px", fontSize: "0.75rem", borderRadius: "4px" }}
              title="Forge: Supports Forge coremods and custom mods"
              onClick={() => props.onSettingsChange?.({ ...props.settings, launchProfileType: "forge" })}
            >
              Forge
            </button>
          </div>
        </div>
        <h2>{isForge ? "Opus 1.8.9 (Forge)" : "Opus 1.8.9 (Standard)"}</h2>
        <p className="home-profile-status" title={props.snapshot?.gameLaunchStatus}>
          {props.opusClientVerified
            ? (isForge ? "Verified Opus + Forge profile. Ready for launch." : "Verified Opus Standard profile. Ready for launch.")
            : props.snapshot?.gameLaunchStatus ?? "Waiting for launcher state."}</p>
        <dl className="launch-facts">
          <div><dt>Profile</dt><dd>{isForge ? "Forge" : "Standard"}</dd></div>
          <div><dt>Account</dt><dd className="account-username">{props.selectedAccount?.username ?? "Not selected"}</dd></div>
          <div><dt>Memory</dt><dd>{props.settings.maxMemoryMib} MB</dd></div><div><dt>Instances</dt><dd>{props.activeLaunchCount}</dd></div></dl>
        <div className="launch-account-row"><AccountHeadAvatar skinDataUrl={props.selectedAccountSkin?.dataUrl} username={props.selectedAccount?.username} size={36} />
          {props.accounts.length > 1 ? <label className="inline-select"><span>Active account</span>
            <select className="account-username" value={props.selectedAccount?.id ?? ""} onChange={(event) => props.onSelectAccount(event.target.value)}>
              {props.accounts.map((account) => <option className="account-username" key={account.id} value={account.id}>{account.username}</option>)}</select></label>
            : <button type="button" className="account-summary" onClick={() => props.onNavigate("account")}>
              <strong className="account-username">{props.selectedAccount?.username ?? "Choose an account"}</strong>
              <span>{props.selectedAccount?.kind === "microsoft" ? "Microsoft account" : props.selectedAccount ? "Offline profile" : "Required to launch"}</span></button>}
        </div>
        <button type="button" className="launcher-primary" disabled={props.primaryDisabled} onClick={props.onPrimaryAction}>{props.primaryLabel}</button>
      </div>
    </div>
    <div className="home-status-grid">
      <section className="surface-panel home-readiness" aria-labelledby="home-readiness-title">
        <header className="home-status-heading"><h2 id="home-readiness-title">Readiness</h2></header>
        <div className="home-readiness-items">
        <OperationalRow label="Installation" value={props.snapshotPending ? "Checking" : props.opusClientVerified ? "Ready" : "Action required"}
          tone={props.opusClientVerified ? "success" : props.snapshotUnavailable ? "danger" : "warning"} onClick={() => props.onNavigate("installation")} />
        <OperationalRow label="Account" value={props.profileReady ? "Ready" : "Required"} tone={props.profileReady ? "success" : "warning"} onClick={() => props.onNavigate("account")} />
        <OperationalRow label="Java" value={props.snapshot?.javaStatus ?? "Unavailable"} tone={props.snapshot?.minecraftReady ? "success" : "muted"} />
        </div>
      </section>
      <section className="surface-panel home-sessions" aria-labelledby="home-sessions-title">
        <header className="home-status-heading"><h2 id="home-sessions-title">Active sessions</h2>
          <StatusBadge tone={props.instances.length ? "success" : "muted"}>{props.instances.length} running</StatusBadge></header>
        {props.instances.length === 0 ? <p className="home-session-empty">No running instances</p>
          : <div className="session-list" tabIndex={0} role="region" aria-label="Running instances">{props.instances.map((instance) => <div className="session-row" key={instance.sessionId}>
            <div><strong className="account-username">{instance.username}</strong><span>{instance.title || instance.sessionId}</span></div><StatusBadge tone="success">Running</StatusBadge>
            <button type="button" className="button-danger button-compact" disabled={props.busy !== null} onClick={() => props.onKillInstance(instance.sessionId)}>Stop</button>
          </div>)}</div>}
      </section>
    </div>
  </section>;
}

function InstallationPage(props: {
  snapshot: TuiSnapshot | null; minecraftReady: boolean; optifineReady: boolean; gameLaunchReady: boolean;
  busy: string | null; installProgress: TuiInstallProgress | null; optifineSourcePath: string;
  onBack: () => void; onRefresh: () => void; onInstall: () => void; onImportOptiFine: () => void;
  onOptiFineSourcePathChange: (value: string) => void;
}) {
  const percent = props.installProgress ? Math.round((props.installProgress.completedFiles / Math.max(1, props.installProgress.totalFiles)) * 100) : 0;
  const packageReady = props.minecraftReady && props.optifineReady && props.gameLaunchReady;
  return <section className="launcher-page" aria-labelledby="installation-title">
    <PageHeader id="installation-title" eyebrow="Local distribution" title="Installation"
      description="Verify each required component without changing the normal .minecraft directory." onBack={props.onBack}
      aside={<StatusBadge tone={packageReady ? "success" : "warning"}>{packageReady ? "Ready" : "Action required"}</StatusBadge>} />
    <Surface title="Opus Client bundle" detail="Unified local release contract"><div className="operational-list">
      <OperationalRow label="Minecraft" value={props.snapshot?.minecraftStatus ?? "Not checked"} tone={props.minecraftReady ? "success" : "warning"} />
      <OperationalRow label="OptiFine" value={props.snapshot?.optifineStatus ?? "Not checked"} tone={props.optifineReady ? "success" : "warning"} />
      <OperationalRow label="Java runtime" value={props.snapshot?.javaStatus ?? "Not checked"} tone={props.minecraftReady ? "success" : "warning"} />
      <OperationalRow label="Launch host" value={props.snapshot?.gameLaunchStatus ?? "Not checked"} tone={props.gameLaunchReady ? "success" : "warning"} />
    </div>
      {props.installProgress ? <div className="progress-block"><div><span>{props.installProgress.phase}</span><strong>{props.installProgress.completedFiles} / {props.installProgress.totalFiles}</strong></div>
        <div className="progress-track" role="progressbar" aria-label="Installation progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}><span style={{ width: `${percent}%` }} /></div>
        <small>{props.installProgress.downloadedFiles} downloaded · {props.installProgress.cachedFiles} reused</small></div> : null}
      <div className="panel-actions"><button type="button" className="launcher-primary" onClick={props.onInstall} disabled={props.busy !== null}>
        {props.busy === "installing" ? "Verifying…" : packageReady ? "Verify installation" : "Install and verify"}</button>
        <button type="button" className="button-secondary" onClick={props.onRefresh} disabled={props.busy !== null}>Refresh state</button></div>
    </Surface>
    {!props.optifineReady ? <Surface title="OptiFine source" detail="Use your verified local 1.8.9 HD U M5 JAR">
      <div className="form-row form-row--action"><label htmlFor="optifine-source-path">Local JAR path</label>
        <input id="optifine-source-path" type="text" value={props.optifineSourcePath} placeholder="/path/to/OptiFine_1.8.9_HD_U_M5.jar"
          disabled={props.busy !== null} onChange={(event) => props.onOptiFineSourcePathChange(event.target.value)} />
        <button type="button" className="button-secondary" onClick={props.onImportOptiFine} disabled={!props.optifineSourcePath.trim() || props.busy !== null}>
          {props.busy === "importing" ? "Verifying…" : "Import OptiFine"}</button></div></Surface> : null}
  </section>;
}

function AccountPage(props: {
  accounts: readonly TuiAccount[]; selectedAccount: TuiAccount | null; activeAccountIds: ReadonlySet<string>;
  skin: TuiAccountSkin | null; skinCache?: Record<string, TuiAccountSkin>; skinLoading: boolean; snapshotUnavailable: boolean;
  offlineUsername: string; busy: string | null; loginCancelling: boolean; developerProfile: TuiSnapshot["developerTestProfile"] | undefined;
  onBack: () => void; onOfflineUsernameChange: (value: string) => void; onSaveOfflineProfile: () => void;
  onStartMicrosoftLogin: () => void; onCancelMicrosoftLogin: () => void; onSelectAccount: (id: string) => void;
  onRemoveAccount: (id: string) => void; onRunDeveloperSimulation: () => void;
}) {
  return <section className="launcher-page" aria-labelledby="account-title">
    <PageHeader id="account-title" focusable eyebrow="Identity" title="Account" description="Choose the profile used for the next launch. Stored identities stay local to Opus."
      onBack={props.onBack} aside={<StatusBadge tone={props.selectedAccount?.ready ? "success" : "warning"}>{props.accounts.length} stored</StatusBadge>} />
    <div className="account-grid">
      <Surface title="Add profile" detail="Official or offline"><div className="form-stack">
        <div className="form-copy"><strong>Microsoft account</strong><span>Sign in and verify Java Edition ownership.</span></div>
        <div className="panel-actions"><button type="button" className="launcher-primary" disabled={props.busy !== null || props.snapshotUnavailable} onClick={props.onStartMicrosoftLogin}>
          {props.busy === "logging in" ? "Waiting for sign-in…" : "Sign in with Microsoft"}</button>
          {props.busy === "logging in" ? <button type="button" className="button-danger" disabled={props.loginCancelling} onClick={props.onCancelMicrosoftLogin}>
            {props.loginCancelling ? "Cancelling…" : "Cancel"}</button> : null}</div>
        <div className="section-divider" /><label htmlFor="offline-username">Offline profile</label>
        <input id="offline-username" className="account-username" type="text" value={props.offlineUsername} maxLength={16}
          autoCapitalize="none" autoCorrect="off" autoComplete="off" spellCheck={false}
          placeholder="Minecraft username" disabled={props.busy !== null}
          onChange={(event) => props.onOfflineUsernameChange(event.target.value)} onKeyDown={(event) => {
            if (event.key === "Enter" && props.offlineUsername.trim() && props.busy === null) props.onSaveOfflineProfile();
          }} />
        <button type="button" className="button-secondary" disabled={!props.offlineUsername.trim() || props.busy !== null} onClick={props.onSaveOfflineProfile}>Add offline profile</button>
        {props.developerProfile?.available ? <div className="developer-row"><div><strong>Developer simulation</strong><span>No game process is launched.</span></div>
          <button type="button" className="button-secondary button-compact" disabled={props.developerProfile.simulationActive || props.busy !== null} onClick={props.onRunDeveloperSimulation}>
            {props.developerProfile.simulationActive ? "Running" : props.developerProfile.active ? "Disable" : "Run"}</button></div> : null}
      </div></Surface>
      <Surface title="Registered profiles" detail={`${props.accounts.length} available`}>
        {props.accounts.length === 0 ? <EmptyState title="No accounts yet" detail="Sign in with Microsoft or add an offline profile to continue." />
          : <div className="account-list">{props.accounts.map((account) => {
            const selected = props.selectedAccount?.id === account.id;
            const accountSkin = props.skinCache?.[account.id] ?? (selected ? props.skin : null);
            const running = props.activeAccountIds.has(account.id);
            return <div className={`account-row ${selected ? "is-selected" : ""}`} key={account.id}>
              <button type="button" className="account-row__select" aria-label={`Select ${account.username}`} aria-pressed={selected} onClick={() => props.onSelectAccount(account.id)}>
                <AccountHeadAvatar skinDataUrl={accountSkin?.dataUrl} username={account.username} size={36} />
                <span><strong className="account-username">{account.username}</strong><small>{account.uuid ? compactUuid(account.uuid) : "Offline profile"}</small></span></button>
              <div className="account-row__meta">{selected && props.skinLoading ? <StatusBadge tone="muted">Loading skin</StatusBadge> : null}
                <StatusBadge tone={running ? "success" : account.ready ? "normal" : "warning"}>{running ? "Running" : account.kind}</StatusBadge>
                <button type="button" className="button-danger button-compact" disabled={props.busy !== null || running} onClick={() => props.onRemoveAccount(account.id)}>Remove</button></div>
            </div>;
          })}</div>}
      </Surface>
    </div>
  </section>;
}

function ModulesPage(props: {
  utilities: readonly TuiUtilityDefinition[]; preferences: Record<TuiUtilityId, TuiUtilityPreference>;
  saveStatus: "loading" | "saved" | "saving" | "error"; enabledCount: number; onBack: () => void;
  onChange: (id: TuiUtilityId, update: Partial<TuiUtilityPreference>) => void; onOpenUtilities: (id: TuiUtilityId) => void;
}) {
  return <section className="launcher-page" aria-labelledby="modules-title">
    <PageHeader id="modules-title" eyebrow="Client overlays" title="Modules" description="Enable only the information overlays you want available in game."
      onBack={props.onBack} aside={<SaveStatus status={props.saveStatus} label={`${props.enabledCount} enabled`} />} />
    <Surface title="Available modules" detail="Changes save automatically">
      {props.utilities.length === 0 ? <EmptyState title="No modules available" detail="The launcher did not report any configurable overlays." />
        : <div className="module-list">{props.utilities.map((utility) => {
          const preference = props.preferences[utility.id];
          return <div className="module-row" key={utility.id}><span className="module-row__icon" aria-hidden="true">{utility.icon}</span>
            <div className="module-row__copy"><strong>{utility.name}</strong><span>{utility.description}</span></div>
            <span className="module-row__category">{utility.category}</span>
            <button type="button" className="button-quiet button-compact" onClick={() => props.onOpenUtilities(utility.id)}>Configure</button>
            <SwitchControl label={`${preference.enabled ? "Disable" : "Enable"} ${utility.name}`} checked={preference.enabled}
              onChange={() => props.onChange(utility.id, { enabled: !preference.enabled })} />
          </div>;
        })}</div>}
    </Surface>
  </section>;
}

function UtilitiesPage(props: {
  utilities: readonly TuiUtilityDefinition[]; preferences: Record<TuiUtilityId, TuiUtilityPreference>;
  saveStatus: "loading" | "saved" | "saving" | "error"; selected: TuiUtilityDefinition | null;
  preference: TuiUtilityPreference | null; onBack: () => void; onSelect: (id: TuiUtilityId) => void;
  onChange: (id: TuiUtilityId, update: Partial<TuiUtilityPreference>) => void;
}) {
  return <section className="launcher-page" aria-labelledby="utilities-title">
    <PageHeader id="utilities-title" eyebrow="HUD utilities" title="Utilities" description="Manage overlay visibility and appearance. Arrange overlays in the in-game HUD editor with Right Shift."
      onBack={props.onBack} aside={<SaveStatus status={props.saveStatus} />} />
    <div className="utility-grid">
      <Surface title="Utility" detail={`${props.utilities.length} available`}>
        {props.utilities.length === 0 ? <EmptyState title="No utilities available" detail="There is nothing to configure in this snapshot." />
          : <div className="utility-menu">{props.utilities.map((utility) => <button key={utility.id} type="button"
            className={props.selected?.id === utility.id ? "is-active" : ""} aria-pressed={props.selected?.id === utility.id} onClick={() => props.onSelect(utility.id)}>
            <span>{utility.icon}</span><strong>{utility.name}</strong><small>{props.preferences[utility.id].enabled ? "On" : "Off"}</small></button>)}</div>}
      </Surface>
      <Surface title={props.selected?.name ?? "Configuration"} detail={props.selected?.purpose ?? "Select a utility"}>
        {props.selected && props.preference ? <div className="utility-editor"><div className="utility-editor__status">
          <div><strong>{props.selected.name}</strong><span>{props.selected.description}</span></div>
          <SwitchControl label={`${props.preference.enabled ? "Disable" : "Enable"} ${props.selected.name}`} checked={props.preference.enabled}
            onChange={() => props.onChange(props.selected!.id, { enabled: !props.preference!.enabled })} /></div>
          <div className="form-grid">
            <div className="field-control field-control--range"><div><label htmlFor="utility-scale">Scale</label><output htmlFor="utility-scale">{props.preference.scale}%</output></div>
              <input id="utility-scale" type="range" min={UTILITY_SCALE_MIN} max={UTILITY_SCALE_MAX} step={1} value={props.preference.scale}
                onChange={(event) => props.onChange(props.selected!.id, { scale: Number(event.target.value) })} /></div>
            <div className="field-control field-control--range"><div><label htmlFor="utility-opacity">Opacity</label><output htmlFor="utility-opacity">{props.preference.opacity}%</output></div>
              <input id="utility-opacity" type="range" min={25} max={100} step={5} value={props.preference.opacity}
                onChange={(event) => props.onChange(props.selected!.id, { opacity: Number(event.target.value) })} /></div></div>
        </div> : <EmptyState title="No utility selected" detail="Choose an available utility to edit its appearance." />}
      </Surface>
    </div>
  </section>;
}

function SettingsPage(props: {
  settings: TuiSettings; snapshot: TuiSnapshot | null; appearance: LauncherAppearance; busy: string | null;
  developerProfile: TuiSnapshot["developerTestProfile"] | undefined; onBack: () => void;
  onAppearanceChange: (appearance: LauncherAppearance) => void; onSettingsChange: (settings: TuiSettings) => void;
  onSaveSettings: () => void; onRunDeveloperSimulation: () => void; onQuit: () => void;
}) {
  return <section className="launcher-page" aria-labelledby="settings-title">
    <PageHeader id="settings-title" eyebrow="Launcher preferences" title="Settings"
      description="Control launch memory, lifecycle behavior, and the bounded Home media treatment." onBack={props.onBack} />
    <div className="settings-grid">
      <Surface title="System" detail={props.snapshot?.platform ?? "Platform unavailable"}><div className="form-stack">
        <div className="setting-row"><div><strong>Client Edition</strong><span>Standard runs standalone OpusLoader; Forge supports Forge coremods and custom mods.</span></div>
          <div className="preset-row">
            <button type="button" className={props.settings.launchProfileType !== "forge" ? "is-active" : ""} disabled={props.busy !== null}
              onClick={() => props.onSettingsChange({ ...props.settings, launchProfileType: "standalone" })}>Standard (OpusLoader)</button>
            <button type="button" className={props.settings.launchProfileType === "forge" ? "is-active" : ""} disabled={props.busy !== null}
              onClick={() => props.onSettingsChange({ ...props.settings, launchProfileType: "forge" })}>Opus + Forge</button>
          </div></div>
        <div className="field-control field-control--range"><div><label htmlFor="allocated-memory">Allocated memory</label><output htmlFor="allocated-memory">{props.settings.maxMemoryMib} MB</output></div>
          <input id="allocated-memory" type="range" min={512} max={8192} step={256} value={props.settings.maxMemoryMib} disabled={props.busy !== null}
            onChange={(event) => props.onSettingsChange({ ...props.settings, maxMemoryMib: Number(event.target.value) })} />
          <div className="preset-row">{[2048, 4096, 6144, 8192].map((preset) => <button key={preset} type="button"
            className={props.settings.maxMemoryMib === preset ? "is-active" : ""} disabled={props.busy !== null}
            onClick={() => props.onSettingsChange({ ...props.settings, maxMemoryMib: preset })}>{preset / 1024} GB</button>)}</div></div>
        <div className="setting-row"><div><strong>Close on game start</strong><span>Exit the launcher after Minecraft starts successfully.</span></div>
          <SwitchControl label="Close launcher on game start" checked={props.settings.closeLauncherOnGameStart} disabled={props.busy !== null}
            onChange={() => props.onSettingsChange({ ...props.settings, closeLauncherOnGameStart: !props.settings.closeLauncherOnGameStart })} /></div>
        <div className="directory-row"><span>Data directory</span><code>{props.snapshot?.dataDirectory ?? "Unavailable"}</code></div>
      </div></Surface>
      <Surface title="Appearance" detail="Home media only"><div className="form-stack">
        <div className="field-control"><label>Accent</label><div className="option-grid option-grid--themes">{themeOptions.map((theme) => <button key={theme.id} type="button"
          className={props.appearance.theme === theme.id ? "is-active" : ""} data-tone={theme.id} aria-pressed={props.appearance.theme === theme.id}
          onClick={() => props.onAppearanceChange({ ...props.appearance, theme: theme.id })}><span aria-hidden="true" />{theme.label}</button>)}</div></div>
        <div className="field-control"><label>Home media</label><div className="option-grid">{backdropOptions.map((backdrop) => <button key={backdrop.id} type="button"
          className={props.appearance.backdrop === backdrop.id ? "is-active" : ""} aria-pressed={props.appearance.backdrop === backdrop.id}
          onClick={() => props.onAppearanceChange({ ...props.appearance, backdrop: backdrop.id })}><strong>{backdrop.label}</strong><small>{backdrop.detail}</small></button>)}</div></div>
        <div className="field-control field-control--range"><div><label htmlFor="media-softness">Media softness</label><output htmlFor="media-softness">{props.appearance.glassBlur}</output></div>
          <input id="media-softness" type="range" min={4} max={24} value={props.appearance.glassBlur}
            onChange={(event) => props.onAppearanceChange({ ...props.appearance, glassBlur: Number(event.target.value) })} /></div>
        <div className="field-control field-control--range"><div><label htmlFor="media-opacity">Media opacity</label><output htmlFor="media-opacity">{props.appearance.glassOpacity}%</output></div>
          <input id="media-opacity" type="range" min={40} max={95} value={props.appearance.glassOpacity}
            onChange={(event) => props.onAppearanceChange({ ...props.appearance, glassOpacity: Number(event.target.value) })} /></div>
      </div></Surface>
    </div>
    {props.developerProfile?.available ? <Surface title="Developer profile" detail="Visible only in development-enabled builds"><div className="setting-row">
      <div><strong>{props.developerProfile.active ? "Developer profile active" : "Developer simulation"}</strong><span>Exercise the launch lifecycle without starting Minecraft.</span></div>
      <button type="button" className="button-secondary" disabled={props.developerProfile.simulationActive || props.busy !== null} onClick={props.onRunDeveloperSimulation}>
        {props.developerProfile.simulationActive ? "Simulation running" : props.developerProfile.active ? "Disable profile" : "Run simulation"}</button></div></Surface> : null}
    <div className="settings-actions"><button type="button" className="launcher-primary" disabled={props.busy !== null} onClick={props.onSaveSettings}>
      {props.busy === "saving" ? "Saving…" : "Save settings"}</button><button type="button" className="button-secondary" disabled={props.busy !== null} onClick={props.onQuit}>Quit launcher</button></div>
  </section>;
}

function LogsPage(props: {
  category: LogCategory; lines: readonly string[]; dataDirectory: string; copyStatus: "idle" | "copied" | "failed";
  onBack: () => void; onCategoryChange: (category: LogCategory) => void; onCopy: () => void; onRefresh: () => void;
}) {
  return <section className="launcher-page" aria-labelledby="logs-title">
    <PageHeader id="logs-title" eyebrow="Diagnostics" title="Logs"
      description="Read the current launcher snapshot without presenting preview data as a live game session." onBack={props.onBack}
      aside={<StatusBadge tone="muted">{props.category}</StatusBadge>} />
    <Surface title="Diagnostic log" detail={`Data: ${props.dataDirectory}`}><div className="log-toolbar">
      <div className="log-categories" aria-label="Log category">{logCategories.map((category) => <button key={category} type="button"
        className={props.category === category ? "is-active" : ""} aria-pressed={props.category === category} onClick={() => props.onCategoryChange(category)}>{category}</button>)}</div>
      <div className="panel-actions"><button type="button" className="button-secondary button-compact" onClick={props.onRefresh}>Refresh</button>
        <button type="button" className="button-secondary button-compact" aria-label="Copy log" onClick={props.onCopy}>
          {props.copyStatus === "copied" ? "Copied" : props.copyStatus === "failed" ? "Copy failed" : "Copy log"}</button></div></div>
      <pre className="log-terminal" aria-live="polite">{props.lines.length > 0 ? props.lines.join("\n") : "No log entries."}</pre>
    </Surface>
  </section>;
}

function PageHeader(props: { id: string; eyebrow: string; title: string; description?: string; onBack?: () => void; aside?: ReactNode; focusable?: boolean }) {
  return <header className="page-header"><div className="page-header__copy">
    {props.onBack ? <button type="button" className="page-back" onClick={props.onBack}><ArrowLeftIcon />Home</button> : null}
    <span className="page-eyebrow">{props.eyebrow}</span><h1 id={props.id} tabIndex={props.focusable ? -1 : undefined}>{props.title}</h1>{props.description ? <p>{props.description}</p> : null}</div>
    {props.aside ? <div className="page-header__aside">{props.aside}</div> : null}</header>;
}

function Surface(props: { title: string; detail?: string; children: ReactNode }) {
  return <section className="surface-panel"><header className="surface-header"><div><h2>{props.title}</h2>{props.detail ? <p>{props.detail}</p> : null}</div></header>
    <div className="surface-content">{props.children}</div></section>;
}

function OperationalRow(props: { label: string; value: string; tone?: StatusTone; onClick?: () => void }) {
  const content = <><span>{props.label}</span><StatusBadge tone={props.tone ?? "normal"}>{props.value}</StatusBadge>{props.onClick ? <ArrowRightIcon /> : null}</>;
  return props.onClick ? <button type="button" className="operational-row" title={props.value} onClick={props.onClick}>{content}</button> : <div className="operational-row" title={props.value}>{content}</div>;
}

function StatusBadge(props: { tone?: StatusTone; children: ReactNode }) {
  return <span className={`status-badge is-${props.tone ?? "normal"}`}><i aria-hidden="true" />{props.children}</span>;
}
function SaveStatus(props: { status: "loading" | "saved" | "saving" | "error"; label?: string }) {
  const tone: StatusTone = props.status === "error" ? "danger" : props.status === "saved" ? "success" : "muted";
  return <StatusBadge tone={tone}>{props.label ? `${props.label} · ` : ""}{sentenceCase(props.status)}</StatusBadge>;
}
function SwitchControl(props: { label: string; checked: boolean; disabled?: boolean; onChange: () => void }) {
  return <button type="button" role="switch" className={`switch-control ${props.checked ? "is-checked" : ""}`} aria-label={props.label}
    aria-checked={props.checked} disabled={props.disabled} onClick={props.onChange}><span /></button>;
}
function EmptyState(props: { title: string; detail: string }) {
  return <div className="empty-state"><strong>{props.title}</strong><span>{props.detail}</span></div>;
}
function RefreshIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 6v5h-5M4 18v-5h5M6.2 8.1A7 7 0 0 1 18.8 9M17.8 15.9A7 7 0 0 1 5.2 15" /></svg>; }
function ArrowLeftIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m15 18-6-6 6-6" /></svg>; }
function ArrowRightIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9 18 6-6-6-6" /></svg>; }
function compactUuid(uuid: string) {
  const compact = uuid.replaceAll("-", "");
  return compact.length > 12 ? `${compact.slice(0, 6)}…${compact.slice(-6)}` : compact;
}
function sentenceCase(value: string) { return value ? `${value.charAt(0).toUpperCase()}${value.slice(1)}` : value; }

function buildLogLines(props: LauncherTuiProps, runtimeState: string, opusClientVerified: boolean, enabledUtilityCount: number): string[] {
  const selectedAccount = props.snapshot?.accounts.find((account) => account.id === props.snapshot?.selectedAccountId);
  const lines = [
    `[INFO] Launcher snapshot ........... ${props.snapshot ? "OK" : "UNAVAILABLE"}`,
    `[INFO] Runtime state ............... ${runtimeState.toUpperCase()}`,
    `[INFO] Opus Client bundle ......... ${opusClientVerified ? "VERIFIED & READY" : "SETUP REQUIRED"}`,
    `[INFO] Java runtime ............... ${props.snapshot?.javaStatus ?? "UNKNOWN"}`,
    `[INFO] Identities count ........... ${props.snapshot?.accounts.length ?? 0}`,
    `[INFO] Selected identity .......... ${selectedAccount?.username ?? "NONE"}`,
    `[INFO] Active game instances ...... ${props.snapshot?.activeLaunches ?? 0}`,
    `[INFO] Enabled modules ............ ${enabledUtilityCount}`,
  ];
  if (props.busy) lines.push(`[BUSY] Active operation ........... ${props.busy.toUpperCase()}`);
  if (props.notice) lines.push(`[NOTICE] Event .................... ${props.notice}`);
  if (props.error) lines.push(`[ERROR] Failure ................... ${friendlyError(props.error)}`);
  return lines;
}

function friendlyError(error: string) {
  const registrationPrefix = "Minecraft Services rejected Opus's application registration";
  if (error.includes(registrationPrefix)) return "Microsoft sign-in completed, but Opus's application ID is still awaiting Minecraft Services approval.";
  return error;
}
