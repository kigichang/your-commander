import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { create } from "zustand";
import i18n from "../i18n";
import * as ipc from "../ipc/commands";
import type { Config } from "../ipc/types";

const darkMedia = window.matchMedia("(prefers-color-scheme: dark)");

/** Resolve "system"/"dark"/"light" onto the document and the window chrome. */
function applyTheme(theme: string) {
  const resolved =
    theme === "light" || theme === "dark"
      ? theme
      : darkMedia.matches
        ? "dark"
        : "light";
  document.documentElement.dataset.theme = resolved;
  // Window chrome (titlebar tint, native menus); null = follow the system.
  void getCurrentWindow()
    .setTheme(theme === "light" || theme === "dark" ? theme : null)
    .catch(() => {});
}

/** CSS string literal for a font family name. */
const quoted = (family: string) => `"${family.replace(/["\\]/g, "\\$&")}"`;

/** Point the font tokens at the chosen families (falling back to the
 *  bundled ones) and set the global font-size multiplier. */
function applyFonts(c: Pick<Config, "font_family" | "font_mono" | "font_scale">) {
  const root = document.documentElement.style;
  const body = c.font_family ? `${quoted(c.font_family)}, ` : "";
  const mono = c.font_mono ? `${quoted(c.font_mono)}, ` : "";
  root.setProperty("--font-body", `${body}"Inter", system-ui, sans-serif`);
  root.setProperty("--font-heading", `${body}"Inter", system-ui, sans-serif`);
  root.setProperty(
    "--font-mono",
    `${mono}"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace`,
  );
  root.setProperty("--font-scale", String(c.font_scale || 1));
}

darkMedia.addEventListener("change", () => {
  const theme = useSettings.getState().config?.theme ?? "system";
  if (theme !== "light" && theme !== "dark") applyTheme(theme);
});

/** The config main.tsx fetched for i18n before the first render; the first
 *  load() takes it instead of asking the backend again. */
let bootConfig: Config | null = null;
export const setBootConfig = (config: Config | null) => {
  bootConfig = config;
};

interface SettingsStore {
  config: Config | null;
  home: string | null;
  /** The Trash folder (toolbar Trash button); null if unavailable. */
  trashDir: string | null;
  fkeysAvailable: boolean;
  load: () => Promise<void>;
  toggleShowHidden: () => void;
  toggleFkeys: () => void;
  setLanguage: (lng: string) => void;
  setTheme: (theme: string) => void;
  setFonts: (patch: Partial<Pick<Config, "font_family" | "font_mono" | "font_scale">>) => void;
  /** Merge a patch into the config and persist it. */
  save: (patch: Partial<Config>) => void;
}

/** Config fields read all over the UI; the backend's defaults until load(). */
type HasConfig = { config: Config | null };
export const selectShowHidden = (s: HasConfig) => s.config?.show_hidden ?? false;
export const selectFkeysWithoutFn = (s: HasConfig) => s.config?.fkeys_without_fn ?? true;
/** UI font-size multiplier (Settings → Appearance); 1 = as designed. */
export const selectFontScale = (s: HasConfig) => s.config?.font_scale || 1;

export const useSettings = create<SettingsStore>((set, get) => ({
  config: null,
  home: null,
  trashDir: null,
  fkeysAvailable: false,

  load: async () => {
    const boot = bootConfig;
    bootConfig = null;
    const [config, home, fkeysAvailable, trashDir] = await Promise.all([
      boot ?? ipc.getConfig(),
      ipc.homeDir(),
      ipc.fkeysAvailable(),
      ipc.trashDir().catch(() => null),
    ]);
    set({ config, home, trashDir, fkeysAvailable });
    if (config.language) i18n.changeLanguage(config.language);
    applyTheme(config.theme ?? "system");
    applyFonts(config);
  },

  toggleShowHidden: () => get().save({ show_hidden: !selectShowHidden(get()) }),

  toggleFkeys: () => {
    const next = !selectFkeysWithoutFn(get());
    // fkeys_set persists the config itself and flips the mode right away;
    // mirror it locally without writing the file twice.
    set((s) => ({ config: s.config ? { ...s.config, fkeys_without_fn: next } : s.config }));
    void ipc.fkeysSet(next);
  },

  setLanguage: (lng) => {
    i18n.changeLanguage(lng);
    get().save({ language: lng });
  },

  setTheme: (theme) => {
    applyTheme(theme);
    get().save({ theme });
  },

  setFonts: (patch) => {
    applyFonts({ ...(get().config as Config), ...patch });
    get().save(patch);
  },

  save: (patch) => {
    const config = { ...(get().config as Config), ...patch };
    set({ config });
    // Only the changed fields: sending the whole object would write this
    // window's copy of every other setting back over the other windows'.
    void ipc.updateConfig(patch);
  },
}));

/** Take another window's settings change: merge it and re-apply whatever
 *  shows (theme, fonts, language). Patches that arrive before load() are
 *  kept and merged once the config is in. */
let early: Partial<Config>[] = [];

function takePatch(patch: Partial<Config>) {
  const current = useSettings.getState().config;
  if (!current) {
    early.push(patch);
    return;
  }
  const config = { ...current, ...patch };
  useSettings.setState({ config });
  if ("theme" in patch) applyTheme(config.theme ?? "system");
  if ("font_family" in patch || "font_mono" in patch || "font_scale" in patch) applyFonts(config);
  if ("language" in patch && config.language) i18n.changeLanguage(config.language);
}

useSettings.subscribe((s, prev) => {
  if (s.config && !prev.config && early.length) {
    const pending = early;
    early = [];
    pending.forEach(takePatch);
  }
});

const ownLabel = getCurrentWindow().label;
void listen<{ source: string; patch: Partial<Config> }>("config-patched", (e) => {
  // This window applied its own change before sending it.
  if (e.payload.source !== ownLabel) takePatch(e.payload.patch);
});
