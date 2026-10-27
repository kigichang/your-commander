import { create } from "zustand";
import i18n from "../i18n";
import * as ipc from "../ipc/commands";
import type { Config } from "../ipc/types";

interface SettingsStore {
  config: Config | null;
  home: string | null;
  showHidden: boolean;
  load: () => Promise<void>;
  toggleShowHidden: () => void;
  setLanguage: (lng: string) => void;
  /** Merge a patch into the config and persist it. */
  save: (patch: Partial<Config>) => void;
}

export const useSettings = create<SettingsStore>((set, get) => ({
  config: null,
  home: null,
  showHidden: false,

  load: async () => {
    const [config, home] = await Promise.all([ipc.getConfig(), ipc.homeDir()]);
    set({ config, home, showHidden: config.show_hidden });
    if (config.language) i18n.changeLanguage(config.language);
  },

  toggleShowHidden: () => {
    const next = !get().showHidden;
    set({ showHidden: next });
    get().save({ show_hidden: next });
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
