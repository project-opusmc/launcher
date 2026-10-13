import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { RemoveAccountDialog } from "./ui/RemoveAccountDialog";
import {
  applyUtilityPreferencePatches,
  mergeUtilitySettings,
  type UtilityPreferencePatches,
  type UtilitySettings,
} from "./utilitySettings";
import LauncherTui, {
  type TuiAccount,
  type TuiAccountSkin,
  type TuiInstallProgress,
  type TuiInstance,
  type TuiPage,
  type TuiSettings,
  type TuiSnapshot,
  type TuiUtilityDefinition,
  type TuiUtilityId,
  type TuiUtilityPreference,
} from "./ui/LauncherTui";

type DeveloperTestProfile = {
  available: boolean;
  active: boolean;
  simulationActive: boolean;
};

type AccountResult = {
  profile: string;
  account: TuiAccount;
};

type GameLaunchStarted = {
  sessionId: string;
  logDirectory: string | null;
  accountId: string;
  simulated: boolean;
};

type GameLaunchFinished = GameLaunchStarted & {
  outcome: "exited" | "failed";
  message: string;
};

type InstallResult = {
  minecraftVersion: string;
  javaVersion: string;
  optifineReady: boolean;
  downloadedFiles: number;
  cachedFiles: number;
};

type OptiFineImportResult = { fileName: string };

type RunningInstance = {
  sessionId: string;
  accountId: string;
  username: string;
  badge: string;
  title: string;
  logDirectory: string;
};

type AccountSkin = TuiAccountSkin;

const defaultSettings: TuiSettings = {
  maxMemoryMib: 2048,
  closeLauncherOnGameStart: false,
  launchProfileType: "standalone",
};

const utilityDefinitions: readonly TuiUtilityDefinition[] = [
  {
    id: "fps",
    category: "performance",
    name: "FPS Monitor",
    description: "Live Minecraft frame-rate counter.",
    purpose: "Keep the current frame rate visible while tuning settings.",
    icon: "FPS",
  },
  {
    id: "cps",
    category: "input",
    name: "CPS Tracker",
    description: "Left and right mouse click rate counters with trailing peak hold.",
    purpose: "Track input pacing and debounce thresholds during combat and bridging.",
    icon: "CPS",
  },
  {
    id: "memory",
    category: "performance",
    name: "Memory HUD",
    description: "Heap allocation, GC pressure, and committed memory monitor.",
    purpose: "Monitor allocated JVM heap without opening the heavy debug overlay.",
    icon: "MEM",
  },
  {
    id: "coordinates",
    category: "hud",
    name: "Coordinates",
    description: "Compass heading, biome identity, and XYZ spatial position.",
    purpose: "Maintain spatial awareness during navigation without cluttering view.",
    icon: "XYZ",
  },
  {
    id: "clock",
    category: "hud",
    name: "Session Clock",
    description: "Real-world time and active gaming session duration meter.",
    purpose: "Keep track of active session time and system time inside full-screen play.",
    icon: "CLK",
  },
  {
    id: "keystrokes",
    category: "input",
    name: "Keystrokes HUD",
    description: "Movement, jump, and mouse-button states using your actual key bindings.",
    purpose: "See physical press and release states, including rebound controls.",
    icon: "KEYS",
  },
];

function defaultUtilityPreference(id: TuiUtilityId): TuiUtilityPreference {
  switch (id) {
    case "fps":
      return { enabled: false, anchor: "top-left", offset: "12 · 12", scale: 100, opacity: 100 };
    case "cps":
      return { enabled: false, anchor: "top-left", offset: "12 · 26", scale: 100, opacity: 100 };
    case "memory":
      return { enabled: false, anchor: "top-right", offset: "12 · 12", scale: 100, opacity: 100 };
    case "coordinates":
      return { enabled: false, anchor: "bottom-left", offset: "12 · 28", scale: 100, opacity: 100 };
    case "clock":
      return { enabled: false, anchor: "top-right", offset: "12 · 26", scale: 100, opacity: 100 };
    case "keystrokes":
      return { enabled: false, anchor: "bottom-right", offset: "12 · 28", scale: 100, opacity: 100 };
  }
}

function createDefaultUtilitySettings(): UtilitySettings {
  return {
    schemaVersion: 1,
    utilities: {
      fps: defaultUtilityPreference("fps"),
      cps: defaultUtilityPreference("cps"),
      memory: defaultUtilityPreference("memory"),
      coordinates: defaultUtilityPreference("coordinates"),
      clock: defaultUtilityPreference("clock"),
      keystrokes: defaultUtilityPreference("keystrokes"),
    },
  };
}

function normalizeUtilitySettings(settings: UtilitySettings | null | undefined): UtilitySettings {
  return mergeUtilitySettings(settings, createDefaultUtilitySettings());
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function previewSnapshot(): TuiSnapshot {
  return {
    platform: "macOS • Apple Silicon (ARM64)",
    dataDirectory: "/Users/opus/Library/Application Support/opus",
    minecraftReady: true,
    minecraftStatus: "Minecraft 1.8.9 verified",
    optifineReady: true,
    optifineStatus: "OptiFine 1.8.9 HD U M5 ready",
    javaStatus: "Java 8 Zulu verified",
    accountStored: true,
    gameLaunchReady: true,
    gameLaunchStatus: "Ready for launch",
    buildEdition: "unified",
    offlineProfile: null,
    accounts: [
      {
        id: "preview-ms",
        username: "OpusUser",
        uuid: "00000000-0000-0000-0000-000000000001",
        kind: "microsoft",
        badge: "MICROSOFT",
        ready: true,
        selected: true,
      },
    ],
    selectedAccountId: "preview-ms",
    activeLaunches: 0,
    activeAccountIds: [],
    developerTestProfile: {
      available: false,
      active: false,
      simulationActive: false,
    },
  };
}

export default function App() {
  const [page, setPage] = useState<TuiPage>("home");
  const [snapshot, setSnapshot] = useState<TuiSnapshot | null>(null);
  const [settings, setSettings] = useState<TuiSettings>(defaultSettings);
  const [utilitySettings, setUtilitySettings] = useState<UtilitySettings>(createDefaultUtilitySettings);
  const [utilitySaveStatus, setUtilitySaveStatus] = useState<"loading" | "saved" | "saving" | "error">("loading");
  const [busy, setBusy] = useState<string | null>(null);
  const [installProgress, setInstallProgress] = useState<TuiInstallProgress | null>(null);
  const [loginCancelling, setLoginCancelling] = useState(false);
  const [runningSessions, setRunningSessions] = useState<Record<string, string>>({});
  const [instances, setInstances] = useState<RunningInstance[]>([]);
  const [skin, setSkin] = useState<AccountSkin | null>(null);
  const [skinCache, setSkinCache] = useState<Record<string, AccountSkin>>({});
  const [skinLoading, setSkinLoading] = useState(false);
  const skinRequestRef = useRef(0);
  const [offlineUsername, setOfflineUsername] = useState("");
  const [optifineSourcePath, setOptifineSourcePath] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [removalTarget, setRemovalTarget] = useState<TuiAccount | null>(null);
  const [removalPending, setRemovalPending] = useState(false);
  const [removalError, setRemovalError] = useState<string | null>(null);
  const removalInFlight = useRef(false);
  const utilityPatches = useRef<UtilityPreferencePatches>({});
  const utilitySaveTimer = useRef<number | null>(null);
  const utilitySaveInFlight = useRef(false);
  const finishedSessions = useRef(new Set<string>());
  const knownSessions = useRef(new Set<string>());

  const refresh = useCallback(async () => {
    setError(null);
    if (!isTauriRuntime()) {
      setSnapshot(previewSnapshot());
      setSettings(defaultSettings);
      setUtilitySettings(createDefaultUtilitySettings());
      setUtilitySaveStatus("saved");
      return true;
    }
    try {
      const [nextSnapshot, nextSettings, nextUtilitySettings] = await Promise.all([
        invoke<TuiSnapshot>("launcher_snapshot"),
        invoke<TuiSettings>("get_settings"),
        invoke<UtilitySettings>("get_utility_settings"),
      ]);
      setSnapshot(nextSnapshot);
      setSettings(nextSettings);
      setUtilitySettings(normalizeUtilitySettings(nextUtilitySettings));
      setUtilitySaveStatus("saved");
      return true;
    } catch (reason) {
      setSnapshot(null);
      setError(String(reason));
      setUtilitySaveStatus("error");
      return false;
    }
  }, []);

  const refreshInstances = useCallback(async () => {
    if (!isTauriRuntime()) return;
    try {
      const next = await invoke<RunningInstance[]>("list_instances");
      setInstances(next);
    } catch {
      // ignore transient list error
    }
  }, []);

  useEffect(() => {
    void refresh();
    void refreshInstances();
  }, [refresh, refreshInstances]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const timer = window.setInterval(() => void refreshInstances(), 2000);
    return () => window.clearInterval(timer);
  }, [refreshInstances]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<GameLaunchFinished>("opus://game-finished", (event) => {
      if (disposed) return;
      const result = event.payload;
      if (!result.simulated && !knownSessions.current.delete(result.sessionId)) {
        finishedSessions.current.add(result.sessionId);
      }
      setRunningSessions((current) => {
        if (!(result.accountId in current)) return current;
        const next = { ...current };
        delete next[result.accountId];
        return next;
      });
      setSnapshot((current) => current ? {
        ...current,
        activeLaunches: current.activeAccountIds.includes(result.accountId)
          ? Math.max(0, current.activeLaunches - 1)
          : current.activeLaunches,
        activeAccountIds: current.activeAccountIds.filter((id) => id !== result.accountId),
        developerTestProfile: result.simulated
          ? { ...current.developerTestProfile, simulationActive: false }
          : current.developerTestProfile,
      } : current);
      const detail = `${result.simulated ? "Developer test" : "Minecraft"}: ${result.message}`;
      if (result.outcome === "failed") setError(detail);
      else setNotice(detail);
      void refreshInstances();
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch((reason) => setError(String(reason)));
    return () => { disposed = true; unlisten?.(); };
  }, [refreshInstances]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<TuiInstallProgress>("opus://install-progress", (event) => {
      if (!disposed) setInstallProgress(event.payload);
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch((reason) => setError(String(reason)));
    return () => { disposed = true; unlisten?.(); };
  }, []);

  const selectedAccountId = snapshot?.selectedAccountId ?? null;
  const selectedAccountUuid = snapshot?.accounts.find((account) => account.id === selectedAccountId)?.uuid ?? null;

  // Pre-fetch & cache skins for all accounts in the background
  useEffect(() => {
    if (!isTauriRuntime() || !snapshot?.accounts) return;
    for (const acc of snapshot.accounts) {
      if (!skinCache[acc.id]) {
        void invoke<AccountSkin>("account_skin", { uuid: acc.uuid ?? "" })
          .then((loaded) => {
            setSkinCache((prev) => ({ ...prev, [acc.id]: loaded }));
          })
          .catch(() => {});
      }
    }
  }, [snapshot?.accounts, skinCache]);

  // Update selected skin smoothly using cache whenever available
  useEffect(() => {
    if (!isTauriRuntime() || !selectedAccountId) {
      setSkin(null);
      return;
    }

    if (skinCache[selectedAccountId]) {
      setSkin(skinCache[selectedAccountId]);
      setSkinLoading(false);
      return;
    }

    const requestId = skinRequestRef.current + 1;
    skinRequestRef.current = requestId;
    setSkinLoading(true);

    void invoke<AccountSkin>("account_skin", { uuid: selectedAccountUuid ?? "" })
      .then((next) => {
        if (skinRequestRef.current === requestId) {
          setSkin(next);
          setSkinCache((prev) => ({ ...prev, [selectedAccountId]: next }));
        }
      })
      .catch(() => {
        if (skinRequestRef.current === requestId) setSkin(null);
      })
      .finally(() => {
        if (skinRequestRef.current === requestId) setSkinLoading(false);
      });
  }, [selectedAccountId, selectedAccountUuid, skinCache]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 6500);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const flushUtilityPatches = useCallback(async () => {
    if (utilitySaveInFlight.current) return;
    utilitySaveInFlight.current = true;
    try {
      while (Object.keys(utilityPatches.current).length > 0) {
        const patches = utilityPatches.current;
        utilityPatches.current = {};
        if (!isTauriRuntime()) continue;
        try {
          const saved = normalizeUtilitySettings(await invoke<UtilitySettings>("save_utility_settings", { patches }));
          setUtilitySettings(applyUtilityPreferencePatches(saved, utilityPatches.current));
        } catch {
          const newer = utilityPatches.current;
          const queued: UtilityPreferencePatches = { ...patches };
          for (const [id, patch] of Object.entries(newer) as [TuiUtilityId, Partial<TuiUtilityPreference>][]) {
            queued[id] = { ...queued[id], ...patch };
          }
          utilityPatches.current = queued;
          setUtilitySaveStatus("error");
          return;
        }
      }
      setUtilitySaveStatus("saved");
    } finally {
      utilitySaveInFlight.current = false;
    }
  }, []);

  useEffect(() => () => {
    if (utilitySaveTimer.current !== null) window.clearTimeout(utilitySaveTimer.current);
  }, []);

  const onUtilityPreferenceChange = useCallback((id: TuiUtilityId, update: Partial<TuiUtilityPreference>) => {
    utilityPatches.current = {
      ...utilityPatches.current,
      [id]: { ...utilityPatches.current[id], ...update },
    };
    setUtilitySettings((current) => applyUtilityPreferencePatches(current, { [id]: update }));
    setUtilitySaveStatus("saving");
    if (utilitySaveTimer.current !== null) window.clearTimeout(utilitySaveTimer.current);
    utilitySaveTimer.current = window.setTimeout(() => {
      utilitySaveTimer.current = null;
      void flushUtilityPatches();
    }, 350);
  }, [flushUtilityPatches]);

  const launchMinecraft = useCallback(async (accountId: string) => {
    setBusy("launching");
    setError(null);
    try {
      const started = await invoke<GameLaunchStarted>("launch_game", { settings, accountId });
      if (finishedSessions.current.delete(started.sessionId)) {
        await refreshInstances();
        return;
      }
      knownSessions.current.add(started.sessionId);
      setRunningSessions((prev) => ({ ...prev, [started.accountId]: started.sessionId }));
      setNotice("Minecraft instance is starting. Its status will update when it exits.");
      await refresh();
      await refreshInstances();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }, [settings, refresh, refreshInstances]);

  const onKillInstance = useCallback(async (sessionId: string) => {
    if (!window.confirm("Stop this Minecraft instance? Unsaved in-game progress may be lost.")) return;
    try {
      await invoke("kill_instance", { sessionId });
      setNotice(`Stop signal sent to session ${sessionId}.`);
      await refresh();
      await refreshInstances();
    } catch (reason) {
      setError(String(reason));
    }
  }, [refresh, refreshInstances]);

  const installMinecraft = useCallback(async () => {
    setBusy("installing");
    setError(null);
    setInstallProgress(null);
    try {
      const result = await invoke<InstallResult>("install_minecraft");
      setNotice(result.optifineReady
        ? `Minecraft ${result.minecraftVersion} verified; ${result.downloadedFiles} downloaded, ${result.cachedFiles} reused.`
        : `Minecraft ${result.minecraftVersion} installed. Import your own OptiFine 1.8.9 HD U M5 JAR to enable launch.`);
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
      setInstallProgress(null);
    }
  }, [refresh]);

  const importOptiFine = useCallback(async () => {
    if (!optifineSourcePath.trim()) {
      setError("Enter the local OptiFine 1.8.9 HD U M5 JAR path first.");
      return;
    }
    setBusy("importing");
    setError(null);
    try {
      const result = await invoke<OptiFineImportResult>("import_optifine", { sourcePath: optifineSourcePath.trim() });
      setNotice(`${result.fileName} verified and added to the Opus runtime.`);
      setOptifineSourcePath("");
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }, [optifineSourcePath, refresh]);

  const saveSettings = useCallback(async () => {
    setBusy("saving");
    setError(null);
    try {
      await invoke("save_settings", { settings });
      setNotice("Settings saved.");
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }, [settings, refresh]);

  const handleSettingsChange = useCallback(
    (nextSettings: TuiSettings) => {
      setSettings(nextSettings);
      if (nextSettings.launchProfileType !== settings.launchProfileType && isTauriRuntime()) {
        invoke("save_settings", { settings: nextSettings }).catch(console.error);
      }
    },
    [settings.launchProfileType],
  );

  const startMicrosoftLogin = useCallback(async () => {
    setBusy("logging in");
    setError(null);
    try {
      const result = await invoke<AccountResult>("login_with_microsoft");
      setNotice(`Signed in as ${result.account.username}. Java Edition ownership verified.`);
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }, [refresh]);

  const cancelMicrosoftLogin = useCallback(async () => {
    setLoginCancelling(true);
    try {
      const signalled = await invoke<boolean>("cancel_microsoft_login");
      if (signalled) setNotice("Cancelling Microsoft sign-in...");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoginCancelling(false);
    }
  }, []);

  const saveOfflineProfile = useCallback(async () => {
    if (!offlineUsername.trim()) return;
    setBusy("saving profile");
    setError(null);
    try {
      await invoke("save_offline_profile", { username: offlineUsername.trim() });
      setNotice(`Offline profile '${offlineUsername.trim()}' created.`);
      setOfflineUsername("");
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }, [offlineUsername, refresh]);

  // OPTIMISTIC ZERO-LATENCY ACCOUNT SWITCHING
  const selectAccount = useCallback(async (accountId: string) => {
    setSnapshot((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        selectedAccountId: accountId,
        accounts: prev.accounts.map((acc) => ({
          ...acc,
          selected: acc.id === accountId,
        })),
      };
    });
    try {
      await invoke("select_account", { accountId });
    } catch (reason) {
      setError(String(reason));
      void refresh();
    }
  }, [refresh]);

  const removeAccount = useCallback((accountId: string) => {
    const account = snapshot?.accounts.find(candidate => candidate.id === accountId);
    if (!account || busy !== null || removalInFlight.current
        || snapshot?.activeAccountIds.includes(accountId)) return;
    setRemovalError(null);
    setRemovalTarget(account);
  }, [busy, snapshot]);

  const confirmAccountRemoval = useCallback(async () => {
    if (!removalTarget || removalInFlight.current) return;
    const { id: accountId, username } = removalTarget;
    removalInFlight.current = true;
    setRemovalPending(true);
    setRemovalError(null);
    setBusy("removing profile");
    try {
      const removed = await invoke<boolean>("remove_account", { accountId });
      if (!removed) throw new Error("This profile no longer exists in the native catalog. Refresh the account list after cancelling.");
      // Only change the visible catalog after the native operation succeeds.
      // Keep this acknowledged state even if the subsequent refresh fails.
      setSnapshot(prev => {
        if (!prev) return prev;
        const remaining = prev.accounts.filter(account => account.id !== accountId);
        const selectedAccountId = prev.selectedAccountId === accountId ? remaining[0]?.id ?? null : prev.selectedAccountId;
        return { ...prev, selectedAccountId,
          accounts: remaining.map(account => ({ ...account, selected: account.id === selectedAccountId })) };
      });
      setRemovalTarget(null);
      setNotice(`Profile '${username}' removed from this Launcher.`);
      await refresh();
    } catch (reason) {
      setRemovalError(`${String(reason)} Removal was not confirmed. You can try again or cancel.`);
    } finally {
      removalInFlight.current = false;
      setRemovalPending(false);
      setBusy(null);
    }
  }, [refresh, removalTarget]);

  const runDeveloperSimulation = useCallback(async () => {
    const profile = snapshot?.developerTestProfile;
    if (!profile?.available || profile.simulationActive) return;
    setBusy("simulation");
    try {
      if (profile.active) {
        await invoke("set_developer_test_profile", { active: false });
        setNotice("Developer test profile disabled.");
      } else {
        await invoke("set_developer_test_profile", { active: true });
        await invoke<GameLaunchStarted>("simulate_developer_game");
        setNotice("Developer simulation started; Minecraft was not launched.");
      }
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }, [refresh, snapshot?.developerTestProfile]);

  const onQuit = useCallback(() => {
    if (isTauriRuntime()) {
      void getCurrentWindow().close().catch((reason) => setError(String(reason)));
    }
  }, []);

  const onBack = useCallback(() => {
    setPage("home");
  }, []);

  return (
    <>
    <LauncherTui
      page={page}
      snapshot={snapshot}
      settings={settings}
      utilities={utilityDefinitions}
      utilityPreferences={utilitySettings.utilities}
      utilitySaveStatus={utilitySaveStatus}
      busy={busy}
      error={error}
      notice={notice}
      installProgress={installProgress}
      loginCancelling={loginCancelling}
      runningSessions={runningSessions}
      instances={instances}
      skin={skin}
      skinCache={skinCache}
      skinLoading={skinLoading}
      offlineUsername={offlineUsername}
      optifineSourcePath={optifineSourcePath}
      onNavigate={setPage}
      onBack={onBack}
      onKillInstance={onKillInstance}
      onRefresh={() => { void refresh(); void refreshInstances(); }}
      onLaunch={launchMinecraft}
      onInstall={installMinecraft}
      onImportOptiFine={importOptiFine}
      onSaveSettings={saveSettings}
      onStartMicrosoftLogin={startMicrosoftLogin}
      onCancelMicrosoftLogin={cancelMicrosoftLogin}
      onSaveOfflineProfile={saveOfflineProfile}
      onSelectAccount={selectAccount}
      onRemoveAccount={removeAccount}
      onRunDeveloperSimulation={runDeveloperSimulation}
      onQuit={onQuit}
      onClearError={() => setError(null)}
      onClearNotice={() => setNotice(null)}
      onSettingsChange={handleSettingsChange}
      onOfflineUsernameChange={setOfflineUsername}
      onOptiFineSourcePathChange={setOptifineSourcePath}
      onUtilityPreferenceChange={onUtilityPreferenceChange}
    />
    {removalTarget ? <RemoveAccountDialog username={removalTarget.username}
      pending={removalPending} error={removalError}
      onCancel={() => { if (!removalInFlight.current) setRemovalTarget(null); }}
      onConfirm={() => { void confirmAccountRemoval(); }} /> : null}
    </>
  );
}
