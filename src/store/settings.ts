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

interface SettingsStore {
  config: Config | null;
  home: string | null;
  /** The Trash folder (toolbar Trash button); null if unavailable. */
  trashDir: string | null;
  showHidden: boolean;
  fkeysWithoutFn: boolean;
  fkeysAvailable: boolean;
  /** UI font-size multiplier (Settings → Appearance); 1 = as designed. */
  fontScale: number;
  load: () => Promise<void>;
  toggleShowHidden: () => void;
  toggleFkeys: () => void;
  setLanguage: (lng: string) => void;
  setTheme: (theme: string) => void;
  setFonts: (patch: Partial<Pick<Config, "font_family" | "font_mono" | "font_scale">>) => void;
  /** Merge a patch into the config and persist it. */
  save: (patch: Partial<Config>) => void;
}

export const useSettings = create<SettingsStore>((set, get) => ({
  config: null,
  home: null,
  trashDir: null,
  showHidden: false,
  fkeysWithoutFn: true,
  fkeysAvailable: false,
  fontScale: 1,

  load: async () => {
    const [config, home, fkeysAvailable, trashDir] = await Promise.all([
      ipc.getConfig(),
      ipc.homeDir(),
      ipc.fkeysAvailable(),
      ipc.trashDir().catch(() => null),
    ]);
    set({
      config,
      home,
      trashDir,
      showHidden: config.show_hidden,
      fkeysWithoutFn: config.fkeys_without_fn,
      fkeysAvailable,
      fontScale: config.font_scale || 1,
    });
    if (config.language) i18n.changeLanguage(config.language);
    applyTheme(config.theme ?? "system");
    applyFonts(config);
  },

  toggleShowHidden: () => {
    const next = !get().showHidden;
    set({ showHidden: next });
    get().save({ show_hidden: next });
  },

  toggleFkeys: () => {
    const next = !get().fkeysWithoutFn;
    // fkeys_set persists the config itself and flips the mode right away;
    // mirror it locally without writing the file twice.
    set((s) => ({
      fkeysWithoutFn: next,
      config: s.config ? { ...s.config, fkeys_without_fn: next } : s.config,
    }));
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
    const config = { ...(get().config as Config), ...patch };
    applyFonts(config);
    set({ fontScale: config.font_scale || 1 });
    get().save(patch);
  },

  save: (patch) => {
    const config = { ...(get().config as Config), ...patch };
    set({ config });
    void ipc.setConfig(config);
  },
}));
