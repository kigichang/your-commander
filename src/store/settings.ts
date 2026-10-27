import { create } from "zustand";
import i18n from "../i18n";
import * as ipc from "../ipc/commands";
import type { Config } from "../ipc/types";

interface SettingsStore {
  config: Config | null;
  home: string | null;
  showHidden: boolean;
  fkeysWithoutFn: boolean;
  fkeysAvailable: boolean;
  load: () => Promise<void>;
  toggleShowHidden: () => void;
  toggleFkeys: () => void;
  setLanguage: (lng: string) => void;
  /** Merge a patch into the config and persist it. */
  save: (patch: Partial<Config>) => void;
}

export const useSettings = create<SettingsStore>((set, get) => ({
  config: null,
  home: null,
  showHidden: false,
  fkeysWithoutFn: true,
  fkeysAvailable: false,

  load: async () => {
    const [config, home, fkeysAvailable] = await Promise.all([
      ipc.getConfig(),
      ipc.homeDir(),
      ipc.fkeysAvailable(),
    ]);
    set({
      config,
      home,
      showHidden: config.show_hidden,
      fkeysWithoutFn: config.fkeys_without_fn,
      fkeysAvailable,
    });
    if (config.language) i18n.changeLanguage(config.language);
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

  save: (patch) => {
    const config = { ...(get().config as Config), ...patch };
    set({ config });
    void ipc.setConfig(config);
  },
}));
