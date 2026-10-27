import { create } from "zustand";
import type { ActionId } from "../keymap/actions";
import type { PresetName } from "../keymap/keymap";
import { buildKeymap } from "../keymap/keymap";
import { useSettings } from "./settings";

interface KeymapStore {
  preset: PresetName;
  /** User overlay: chord → action id, "" = unbound. */
  overrides: Record<string, string>;
  /** The effective chord→action map the keyboard handler consults. */
  map: Record<string, ActionId>;
  load: () => void;
  setPreset: (preset: PresetName) => void;
  /** Bind a chord to an action, unbinding it from wherever it was. */
  bind: (chord: string, action: ActionId) => void;
  unbindChord: (chord: string) => void;
  reset: () => void;
}

export const useKeymap = create<KeymapStore>((set, get) => {
  const persist = () => {
    const { preset, overrides } = get();
    useSettings.getState().save({ hotkey_preset: preset, hotkeys: overrides });
  };
  const rebuild = (preset: PresetName, overrides: Record<string, string>) =>
    set({ preset, overrides, map: buildKeymap(preset, overrides) });

  return {
    preset: "commander",
    overrides: {},
    map: buildKeymap("commander", {}),

    load: () => {
      const config = useSettings.getState().config;
      const preset: PresetName = config?.hotkey_preset === "finder" ? "finder" : "commander";
      rebuild(preset, config?.hotkeys ?? {});
    },

    setPreset: (preset) => {
      // Switching presets drops the custom overlay (it was relative to it).
      rebuild(preset, {});
      persist();
    },

    bind: (chord, action) => {
      const overrides = { ...get().overrides, [chord]: action };
      rebuild(get().preset, overrides);
      persist();
    },

    unbindChord: (chord) => {
      const overrides = { ...get().overrides, [chord]: "" };
      rebuild(get().preset, overrides);
      persist();
    },

    reset: () => {
      rebuild(get().preset, {});
      persist();
    },
  };
});
