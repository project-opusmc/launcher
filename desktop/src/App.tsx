import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import LauncherTui, {
  type TuiInstallProgress,
  type TuiAccount,
  type TuiPage,
  type TuiSettings,
  type TuiSnapshot,
  type TuiUtilityAnchor,
  type TuiUtilityDefinition,
  type TuiUtilityId,
  type TuiUtilityPreference,
} from "./ui/LauncherTui";

type BuildEdition = "premium" | "qaOffline";

type DeveloperTestProfile = {
  available: boolean;
  active: boolean;
  simulationActive: boolean;
};

type InstallResult = {
  minecraftVersion: string;
  javaVersion: string;
  optifineReady: boolean;
  downloadedFiles: number;
  cachedFiles: number;
};

type OptiFineImportResult = {
  fileName: string;
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

type GameLaunchFinished = {
  sessionId: string;
  logDirectory: string | null;
  accountId: string;
  outcome: "exited" | "failed";
  message: string;
  simulated: boolean;
};

type UtilitySettings = {
  schemaVersion: number;
  utilities: Record<TuiUtilityId, TuiUtilityPreference>;
};

type InstallProgressEvent = TuiInstallProgress;

const defaultSettings: TuiSettings = {
  maxMemoryMib: 2048,
  closeLauncherOnGameStart: false,
};

const utilityDefinitions: readonly TuiUtilityDefinition[] = [
  {
    id: "fps",
    category: "performance",
    name: "FPS Counter",
    description: "Live frame rate",
    purpose: "Display the current Minecraft frame rate using the game's live counter.",
    icon: "FPS",
  },
  {
    id: "cps",
    category: "performance",
    name: "CPS",
    description: "Clicks in the last second",
    purpose: "Display left-clicks counted over the previous one-second window.",
    icon: "CPS",
  },
  {
    id: "memory",
    category: "performance",
    name: "Memory",
    description: "Current Java heap usage",
    purpose: "Display memory currently used by the running Minecraft process.",
    icon: "MEM",
  },
  {
    id: "coordinates",
    category: "hud",
    name: "Coordinates",
    description: "Live player position",
    purpose: "Display the player's current XYZ position from local game state.",
    icon: "XYZ",
  },
  {
    id: "clock",
    category: "hud",
    name: "Clock",
    description: "Local system time",
    purpose: "Display a live 24-hour clock without contacting a server.",
    icon: "TIME",
  },
  {
    id: "keystrokes",
    category: "input",
    name: "Keystrokes",
    description: "WASD input state",
    purpose: "Display the live pressed state of the W, A, S and D keys.",
    icon: "WASD",
  },
];

const defaultUtilitySettings: UtilitySettings = {
  schemaVersion: 1,
  utilities: {
    fps: { enabled: false, anchor: "top-left", offset: "12 x 12", scale: 100, opacity: 100 },
    cps: { enabled: false, anchor: "top-left", offset: "12 x 26", scale: 100, opacity: 100 },
    memory: { enabled: false, anchor: "top-right", offset: "12 x 12", scale: 100, opacity: 100 },
    coordinates: { enabled: false, anchor: "bottom-left", offset: "12 x 28", scale: 100, opacity: 100 },
    clock: { enabled: false, anchor: "top-right", offset: "12 x 26", scale: 100, opacity: 100 },
    keystrokes: { enabled: false, anchor: "bottom-right", offset: "12 x 28", scale: 100, opacity: 100 },
  },
};

const minecraftUsernamePattern = /^[A-Za-z0-9_]{3,16}$/;

function createDefaultUtilitySettings(): UtilitySettings {
  return {
    schemaVersion: 1,
    utilities: Object.fromEntries(
      Object.entries(defaultUtilitySettings.utilities).map(([id, preference]) => [id, { ...preference }]),
    ) as Record<TuiUtilityId, TuiUtilityPreference>,
  };
}

function normalizeUtilitySettings(candidate: Partial<UtilitySettings> | null | undefined): UtilitySettings {
  const fallback = createDefaultUtilitySettings();
  const validAnchors: readonly TuiUtilityAnchor[] = ["top-left", "top-right", "bottom-left", "bottom-right"];
  const utilities = Object.fromEntries(utilityDefinitions.map((utility) => {
    const next = candidate?.utilities?.[utility.id];
    const fallbackPreference = fallback.utilities[utility.id];
    const anchor = next?.anchor && validAnchors.includes(next.anchor)
      ? next.anchor
      : fallbackPreference.anchor;
    return [utility.id, {
      enabled: candidate?.schemaVersion === fallback.schemaVersion && typeof next?.enabled === "boolean"
        ? next.enabled
        : fallbackPreference.enabled,
      anchor,
      offset: typeof next?.offset === "string" ? next.offset : fallbackPreference.offset,
      scale: typeof next?.scale === "number" ? Math.min(150, Math.max(50, next.scale)) : fallbackPreference.scale,
      opacity: typeof next?.opacity === "number" ? Math.min(100, Math.max(25, next.opacity)) : fallbackPreference.opacity,
    }];
  })) as Record<TuiUtilityId, TuiUtilityPreference>;
  return { schemaVersion: fallback.schemaVersion, utilities };
}

function isTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function previewSnapshot(): TuiSnapshot {
  const account: TuiAccount = {
    id: "offline:preview",
    username: "PreviewPlayer",
    uuid: "00000000000000000000000000000000",
    kind: "offline",
    badge: "unofficial",
    ready: true,
    selected: true,
  };
  return {
    platform: "UI Preview",
    dataDirectory: "~/.opus-launcher",
    minecraftReady: true,
    minecraftStatus: "Forge 1.8.9 preview state",
    optifineReady: true,
    optifineStatus: "OptiFine HD U M5 preview state",
    javaStatus: "Java 8 preview state",
    accountStored: false,
    gameLaunchReady: true,
    buildEdition: "premium",
    offlineProfile: { username: account.username, valid: true },
    accounts: [account],
    selectedAccountId: account.id,
    activeLaunches: 0,
    activeAccountIds: [],
    developerTestProfile: { available: false, active: false, simulationActive: false },
  };
}

function applyAccountCatalog(
  snapshot: TuiSnapshot,
  accounts: TuiAccount[],
  selectedAccountId: string | null,
): TuiSnapshot {
  const selectedId = selectedAccountId && accounts.some((account) => account.id === selectedAccountId)
    ? selectedAccountId
    : accounts[0]?.id ?? null;
  const normalizedAccounts = accounts.map((account) => ({
    ...account,
    selected: account.id === selectedId,
  }));
  const offline = normalizedAccounts.find((account) => account.kind === "offline");
  return {
    ...snapshot,
    accounts: normalizedAccounts,
    selectedAccountId: selectedId,
    accountStored: normalizedAccounts.some((account) => account.kind === "microsoft" && account.ready),
    offlineProfile: offline ? { username: offline.username, valid: offline.ready } : null,
  };
}

function mergeAccount(snapshot: TuiSnapshot, account: TuiAccount): TuiSnapshot {
  const accounts = snapshot.accounts.some((current) => current.id === account.id)
    ? snapshot.accounts.map((current) => current.id === account.id ? account : current)
    : [...snapshot.accounts, account];
  return applyAccountCatalog(snapshot, accounts, account.selected ? account.id : snapshot.selectedAccountId);
}

function selectSnapshotAccount(snapshot: TuiSnapshot, accountId: string): TuiSnapshot {
  return applyAccountCatalog(snapshot, snapshot.accounts, accountId);
}

export default function App() {
  const [page, setPage] = useState<TuiPage>("home");
  const [snapshot, setSnapshot] = useState<TuiSnapshot | null>(null);
  const [settings, setSettings] = useState<TuiSettings>(defaultSettings);
  const [utilitySettings, setUtilitySettings] = useState<UtilitySettings>(createDefaultUtilitySettings);
  const [utilitySettingsLoaded, setUtilitySettingsLoaded] = useState(false);
  const [utilitySaveStatus, setUtilitySaveStatus] = useState<"loading" | "saved" | "saving" | "error">("loading");
  const [installProgress, setInstallProgress] = useState<InstallProgressEvent | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [loginCancelling, setLoginCancelling] = useState(false);
  const [runningSessions, setRunningSessions] = useState<Record<string, string>>({});
  const [qaUsername, setQaUsername] = useState("");
  const [optifineSourcePath, setOptifineSourcePath] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const utilitySettingsHydrated = useRef(false);
  const finishedSessions = useRef(new Set<string>());
  const knownSessions = useRef(new Set<string>());

  const refresh = useCallback(async () => {
    setError(null);
    if (!isTauriRuntime()) {
      setSnapshot(previewSnapshot());
      setSettings(defaultSettings);
      setUtilitySettings(createDefaultUtilitySettings());
      setUtilitySettingsLoaded(true);
      setUtilitySaveStatus("saved");
      setNotice("Browser UI preview: backend actions are available in the Tauri desktop build.");
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
      setUtilitySettingsLoaded(true);
      setUtilitySaveStatus("saved");
      return true;
    } catch (reason) {
      setSnapshot(null);
      setError(String(reason));
      setUtilitySaveStatus("error");
      return false;
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 6500);
    return () => window.clearTimeout(timer);
  }, [notice]);

  useEffect(() => {
    if (!utilitySettingsLoaded) {
      return;
    }
    if (!utilitySettingsHydrated.current) {
      utilitySettingsHydrated.current = true;
      return;
    }
    setUtilitySaveStatus("saving");
    const timer = window.setTimeout(() => {
      if (!isTauriRuntime()) {
        setUtilitySaveStatus("saved");
        return;
      }
      void invoke("save_utility_settings", { settings: utilitySettings })
        .then(() => setUtilitySaveStatus("saved"))
        .catch((reason) => {
          setUtilitySaveStatus("error");
          setError(String(reason));
        });
    }, 360);
    return () => window.clearTimeout(timer);
  }, [utilitySettings, utilitySettingsLoaded]);

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
      if (result.simulated) {
        setSnapshot((current) => current ? {
          ...current,
          developerTestProfile: { ...current.developerTestProfile, simulationActive: false },
        } : current);
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
        activeAccountIds: current.activeAccountIds.filter((accountId) => accountId !== result.accountId),
      } : current);
      const detail = `${result.simulated ? "Developer test" : "Minecraft"}: ${result.message}${result.logDirectory ? ` Logs: ${result.logDirectory}` : ""}`;
      if (result.outcome === "failed") setError(detail);
      else setNotice(detail);
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<InstallProgressEvent>("opus://install-progress", (event) => {
      if (!disposed) setInstallProgress(event.payload);
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const run = useCallback(async (name: string, action: () => Promise<void>) => {
    setBusy(name);
    setError(null);
    setNotice(null);
    try {
      await action();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }, []);

  const startMicrosoftLogin = useCallback(() => {
    void run("login", async () => {
      if (!isTauriRuntime()) throw new Error("Microsoft sign-in is available in the Tauri desktop build.");
      setLoginCancelling(false);
      const account = await invoke<AccountResult>("login_with_microsoft");
      setSnapshot((current) => current ? mergeAccount(current, account.account) : current);
      setNotice(`Connected as ${account.profile}. Java Edition ownership verified.`);
    });
  }, [run]);

  const cancelMicrosoftLogin = useCallback(() => {
    setLoginCancelling(true);
    void invoke<boolean>("cancel_microsoft_login")
      .then((signalled) => {
        if (signalled) setNotice("Cancelling Microsoft sign-in...");
      })
      .catch((reason) => setError(String(reason)))
      .finally(() => setLoginCancelling(false));
  }, []);

  const installMinecraft = useCallback(() => {
    void run("install", async () => {
      if (!isTauriRuntime()) throw new Error("Installation is available in the Tauri desktop build.");
      setInstallProgress(null);
      const result = await invoke<InstallResult>("install_minecraft");
      setSnapshot((current) => current ? {
        ...current,
        minecraftReady: true,
        minecraftStatus: `Forge ${result.minecraftVersion} verified`,
        optifineReady: result.optifineReady,
        optifineStatus: result.optifineReady ? "OptiFine 1.8.9 HD U M5 verified" : "Import your local OptiFine 1.8.9 HD U M5 JAR to launch",
        javaStatus: `Java ${result.javaVersion} ready`,
      } : current);
      setNotice(`Forge 1.8.9 is ready. ${result.downloadedFiles} downloaded, ${result.cachedFiles} verified.`);
      void refresh();
    });
  }, [refresh, run]);

  const importOptiFine = useCallback(() => {
    void run("optifine", async () => {
      if (!isTauriRuntime()) throw new Error("OptiFine import is available in the Tauri desktop build.");
      if (!optifineSourcePath.trim()) throw new Error("Enter the local OptiFine 1.8.9 HD U M5 JAR path first.");
      const result = await invoke<OptiFineImportResult>("import_optifine", { sourcePath: optifineSourcePath.trim() });
      setOptifineSourcePath("");
      setSnapshot((current) => current ? { ...current, optifineReady: true, optifineStatus: `${result.fileName} verified` } : current);
      setNotice(`${result.fileName} was verified and added to this Opus runtime.`);
      void refresh();
    });
  }, [optifineSourcePath, refresh, run]);

  const launchMinecraft = useCallback((accountId: string) => {
    void run("launch", async () => {
      if (!isTauriRuntime()) throw new Error("Minecraft launch is available in the Tauri desktop build.");
      const started = await invoke<GameLaunchStarted>("launch_game", { settings, accountId });
      if (finishedSessions.current.delete(started.sessionId)) return;
      knownSessions.current.add(started.sessionId);
      setRunningSessions((current) => ({ ...current, [started.accountId]: started.sessionId }));
      setSnapshot((current) => current ? {
        ...current,
        activeLaunches: current.activeAccountIds.includes(started.accountId)
          ? current.activeLaunches
          : current.activeLaunches + 1,
        activeAccountIds: current.activeAccountIds.includes(started.accountId)
          ? current.activeAccountIds
          : [...current.activeAccountIds, started.accountId],
      } : current);
      setNotice("Minecraft instance is starting. You can select another identity and launch it now.");
      if (started.accountId !== accountId) void refresh();
    });
  }, [refresh, run, settings]);

  const saveSettings = useCallback(() => {
    void run("settings", async () => {
      if (!isTauriRuntime()) throw new Error("Settings persistence is available in the Tauri desktop build.");
      await invoke("save_settings", { settings });
      setNotice("Settings saved.");
    });
  }, [run, settings]);

  const saveQaProfile = useCallback(() => {
    void run("offline-profile", async () => {
      if (!isTauriRuntime()) throw new Error("Offline profile persistence is available in the Tauri desktop build.");
      if (!minecraftUsernamePattern.test(qaUsername)) throw new Error("Use 3-16 letters, numbers, or underscores.");
      const account = await invoke<TuiAccount>("save_offline_profile", { username: qaUsername });
      setSnapshot((current) => current ? mergeAccount(current, account) : current);
      setQaUsername("");
      setNotice(`Unofficial profile ${account.username} was added to the launch list.`);
    });
  }, [qaUsername, run]);

  const selectAccount = useCallback((accountId: string) => {
    if (snapshot?.selectedAccountId === accountId) return;
    void run("account-select", async () => {
      if (!isTauriRuntime()) {
        setSnapshot((current) => current ? selectSnapshotAccount(current, accountId) : current);
        return;
      }
      const account = await invoke<TuiAccount>("select_account", { accountId });
      setSnapshot((current) => current ? mergeAccount(current, account) : current);
      setNotice(`${account.username} is selected for the next launch.`);
    });
  }, [run, snapshot?.selectedAccountId]);

  const removeAccount = useCallback((accountId: string) => {
    void run("account-remove", async () => {
      if (!isTauriRuntime()) throw new Error("Account management is available in the Tauri desktop build.");
      const removed = await invoke<boolean>("remove_account", { accountId });
      if (!removed) throw new Error("The selected account no longer exists.");
      await refresh();
      setNotice("The identity and its saved credential were removed from Opus Launcher.");
    });
  }, [refresh, run]);

  const runDeveloperSimulation = useCallback(() => {
    void run("developer-simulation", async () => {
      if (!isTauriRuntime()) throw new Error("Developer simulation is available in the Tauri desktop build.");
      setSnapshot((current) => current ? { ...current, developerTestProfile: { ...current.developerTestProfile, simulationActive: true } } : current);
      try {
        const started = await invoke<GameLaunchStarted>("simulate_developer_game");
        if (!started.simulated) throw new Error("Opus did not confirm a developer test session.");
        setNotice("Developer test session is running. Minecraft was not launched.");
      } catch (reason) {
        setSnapshot((current) => current ? { ...current, developerTestProfile: { ...current.developerTestProfile, simulationActive: false } } : current);
        throw reason;
      }
    });
  }, [run]);

  const onQuit = useCallback(() => {
    if (isTauriRuntime()) {
      void getCurrentWindow().close().catch((reason) => setError(String(reason)));
    } else {
      setNotice("Q is wired to close the desktop window in the Tauri build.");
    }
  }, []);

  const onUtilityPreferenceChange = useCallback((id: TuiUtilityId, update: Partial<TuiUtilityPreference>) => {
    setUtilitySettings((current) => ({
      ...current,
      utilities: { ...current.utilities, [id]: { ...current.utilities[id], ...update } },
    }));
  }, []);

  return (
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
      qaUsername={qaUsername}
      optifineSourcePath={optifineSourcePath}
      onNavigate={setPage}
      onRefresh={() => void refresh()}
      onLaunch={launchMinecraft}
      onInstall={installMinecraft}
      onImportOptiFine={importOptiFine}
      onSaveSettings={saveSettings}
      onStartMicrosoftLogin={startMicrosoftLogin}
      onCancelMicrosoftLogin={cancelMicrosoftLogin}
      onSaveQaProfile={saveQaProfile}
      onSelectAccount={selectAccount}
      onRemoveAccount={removeAccount}
      onRunDeveloperSimulation={runDeveloperSimulation}
      onQuit={onQuit}
      onClearError={() => setError(null)}
      onClearNotice={() => setNotice(null)}
      onSettingsChange={setSettings}
      onQaUsernameChange={setQaUsername}
      onOptiFineSourcePathChange={setOptifineSourcePath}
      onUtilityPreferenceChange={onUtilityPreferenceChange}
    />
  );
}

export type { BuildEdition, DeveloperTestProfile };
