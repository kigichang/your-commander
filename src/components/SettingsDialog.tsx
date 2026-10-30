import clsx from "clsx";
import { ChevronDown, ChevronUp, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ActionId } from "../keymap/actions";
import { actions } from "../keymap/actions";
import type { PresetName } from "../keymap/keymap";
import { chordOf, prettyChord } from "../keymap/keymap";
import type { Dialog } from "../store/dialogs";
import { useDialogs } from "../store/dialogs";
import { useKeymap } from "../store/keymapStore";
import { useOps } from "../store/ops";
import { useSettings } from "../store/settings";
import { Icon } from "./icons";

const SECTIONS = ["general", "favorites", "hotkeys", "panes", "appearance", "transfers", "plugins"] as const;
const READY = new Set(["general", "favorites", "hotkeys"]);

export function SettingsDialog({ d }: { d: Extract<Dialog, { type: "settings" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [section, setSection] = useState<string>(d.section ?? "hotkeys");
  // Which action is recording a new chord, and which chord it replaces.
  const [capture, setCapture] = useState<{ action: ActionId; replace?: string } | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (capture) {
        e.preventDefault();
        e.stopPropagation();
        if (e.key === "Escape") return setCapture(null);
        if (["Shift", "Meta", "Alt", "Control"].includes(e.key)) return;
        const km = useKeymap.getState();
        if (e.key === "Backspace" && capture.replace) {
          km.unbindChord(capture.replace);
          setCapture(null);
          return;
        }
        const chord = chordOf(e);
        const owner = km.map[chord];
        if (capture.replace && capture.replace !== chord) km.unbindChord(capture.replace);
        km.bind(chord, capture.action);
        if (owner && owner !== capture.action) {
          useOps.getState().toast(
            t("settings.rebound", {
              chord: prettyChord(chord),
              action: t(`action.${owner}`),
            }),
          );
        }
        setCapture(null);
        return;
      }
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [capture, close, t]);

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="dialog wide settings">
        <h2>{t("settings.title")}</h2>
        <div className="settings-grid">
          <nav className="settings-nav">
            {SECTIONS.map((s) => (
              <button
                key={s}
                className={clsx(section === s && "on")}
                disabled={!READY.has(s)}
                title={READY.has(s) ? "" : t("status.comingSoon")}
                onClick={() => READY.has(s) && setSection(s)}
              >
                {t(`settings.sec_${s}`)}
              </button>
            ))}
          </nav>
          {section === "general" ? (
            <GeneralPane />
          ) : section === "favorites" ? (
            <FavoritesPane />
          ) : (
            <HotkeysPane capture={capture} setCapture={setCapture} />
          )}
        </div>
        <div className="buttons">
          <button className="btn primary" onClick={close}>
            {t("settings.done")}
          </button>
        </div>
      </div>
    </div>
  );
}

function GeneralPane() {
  const { t, i18n } = useTranslation();
  const showHidden = useSettings((s) => s.showHidden);
  const fkeysOn = useSettings((s) => s.fkeysWithoutFn);
  const fkeysAvailable = useSettings((s) => s.fkeysAvailable);
  const config = useSettings((s) => s.config);
  const plantumlMode = config?.plantuml_mode ?? "off";

  return (
    <div className="settings-pane">
      <label>{t("settings.theme")}</label>
      <select
        value={config?.theme ?? "system"}
        onChange={(e) => useSettings.getState().setTheme(e.target.value)}
      >
        <option value="system">{t("settings.theme_system")}</option>
        <option value="dark">{t("settings.theme_dark")}</option>
        <option value="light">{t("settings.theme_light")}</option>
      </select>
      <label>{t("settings.language")}</label>
      <select
        value={i18n.language.startsWith("zh") ? "zh-TW" : "en"}
        onChange={(e) => useSettings.getState().setLanguage(e.target.value)}
      >
        <option value="en">English</option>
        <option value="zh-TW">繁體中文</option>
      </select>
      <label className="check">
        <input
          type="checkbox"
          checked={showHidden}
          onChange={() => useSettings.getState().toggleShowHidden()}
        />
        {t("settings.showHidden")}
      </label>
      <label className="check">
        <input
          type="checkbox"
          disabled={!fkeysAvailable}
          checked={fkeysOn && fkeysAvailable}
          onChange={() => useSettings.getState().toggleFkeys()}
        />
        {t("settings.fkeys")}
      </label>

      <label>{t("settings.plantuml")}</label>
      <select
        value={plantumlMode}
        onChange={(e) => useSettings.getState().save({ plantuml_mode: e.target.value })}
      >
        <option value="off">{t("settings.plantumlOff")}</option>
        <option value="server">{t("settings.plantumlServer")}</option>
        <option value="jar">{t("settings.plantumlJar")}</option>
      </select>
      {plantumlMode === "server" && (
        <>
          <label>{t("settings.plantumlServerUrl")}</label>
          <input
            className="wide-in"
            value={config?.plantuml_server ?? ""}
            spellCheck={false}
            onChange={(e) => useSettings.getState().save({ plantuml_server: e.target.value })}
          />
          <div className="settings-note">{t("settings.plantumlPrivacy")}</div>
        </>
      )}
      {plantumlMode === "jar" && (
        <>
          <label>{t("settings.plantumlJarPath")}</label>
          <input
            className="wide-in"
            value={config?.plantuml_jar ?? ""}
            spellCheck={false}
            placeholder="/usr/local/lib/plantuml.jar"
            onChange={(e) => useSettings.getState().save({ plantuml_jar: e.target.value })}
          />
          <label>{t("settings.plantumlJavaPath")}</label>
          <input
            className="wide-in"
            value={config?.plantuml_java ?? ""}
            spellCheck={false}
            placeholder={t("settings.plantumlJavaAuto")}
            onChange={(e) => useSettings.getState().save({ plantuml_java: e.target.value })}
          />
          <div className="settings-note">{t("settings.plantumlJarNote")}</div>
        </>
      )}
    </div>
  );
}

function FavoritesPane() {
  const { t } = useTranslation();
  const config = useSettings((s) => s.config);
  const favorites = config?.favorites ?? [];

  const update = (next: typeof favorites) =>
    useSettings.getState().save({ favorites: next });
  const move = (from: number, to: number) => {
    if (to < 0 || to >= favorites.length) return;
    const next = favorites.slice();
    const [f] = next.splice(from, 1);
    next.splice(to, 0, f);
    update(next);
  };
  const rename = (index: number, name: string) => {
    const next = favorites.slice();
    next[index] = { ...next[index], name };
    update(next);
  };

  if (favorites.length === 0) {
    return (
      <div className="settings-pane">
        <div className="settings-note">{t("settings.favNone")}</div>
      </div>
    );
  }
  return (
    <div className="settings-pane">
      <div className="hk-table fav-table">
        {favorites.map((f, i) => (
          <div key={f.path} className="hk-row fav-row">
            <input
              className="fav-name"
              value={f.name}
              spellCheck={false}
              onChange={(e) => rename(i, e.target.value)}
            />
            <span className="fav-path" title={f.path}>
              {f.path}
            </span>
            <button
              className="fav-btn"
              disabled={i === 0}
              title={t("settings.favUp")}
              onClick={() => move(i, i - 1)}
            >
              <Icon of={ChevronUp} size={13} />
            </button>
            <button
              className="fav-btn"
              disabled={i === favorites.length - 1}
              title={t("settings.favDown")}
              onClick={() => move(i, i + 1)}
            >
              <Icon of={ChevronDown} size={13} />
            </button>
            <button
              className="fav-btn danger"
              title={t("panel.removeFavorite")}
              onClick={() => update(favorites.filter((_, j) => j !== i))}
            >
              <Icon of={X} size={13} />
            </button>
          </div>
        ))}
      </div>
      <div className="settings-note">{t("settings.favHint")}</div>
    </div>
  );
}

function HotkeysPane({
  capture,
  setCapture,
}: {
  capture: { action: ActionId; replace?: string } | null;
  setCapture: (c: { action: ActionId; replace?: string } | null) => void;
}) {
  const { t } = useTranslation();
  const preset = useKeymap((s) => s.preset);
  const map = useKeymap((s) => s.map);
  const [filter, setFilter] = useState("");

  const rows = useMemo(() => {
    const byAction = new Map<ActionId, string[]>();
    for (const [chord, action] of Object.entries(map)) {
      byAction.set(action, [...(byAction.get(action) ?? []), chord]);
    }
    const needle = filter.trim().toLowerCase();
    return (Object.keys(actions) as ActionId[])
      .map((id) => ({ id, label: t(`action.${id}`), chords: byAction.get(id) ?? [] }))
      .filter(
        (r) =>
          !needle ||
          r.label.toLowerCase().includes(needle) ||
          r.chords.some((c) => prettyChord(c).toLowerCase().includes(needle)),
      );
  }, [map, filter, t]);

  return (
    <div className="settings-pane">
      <div className="hk-bar">
        <select
          value={preset}
          onChange={(e) => useKeymap.getState().setPreset(e.target.value as PresetName)}
        >
          <option value="commander">{t("settings.presetCommander")}</option>
          <option value="finder">{t("settings.presetFinder")}</option>
        </select>
        <input
          className="hk-filter"
          placeholder={t("settings.filterHint")}
          value={filter}
          spellCheck={false}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button
          className="btn"
          onClick={() => {
            useKeymap.getState().reset();
            useOps.getState().toast(t("settings.resetDone"));
          }}
        >
          {t("settings.reset")}
        </button>
      </div>
      <div className="hk-table">
        {rows.map((row) => (
          <div key={row.id} className="hk-row">
            <span className="hk-action">{row.label}</span>
            <span className="hk-keys">
              {row.chords.map((chord) => (
                <span
                  key={chord}
                  className={clsx(
                    "hk-chip",
                    capture?.action === row.id && capture.replace === chord && "recording",
                  )}
                >
                  <button
                    className="hk-key"
                    title={t("settings.recordHint")}
                    onClick={() => setCapture({ action: row.id, replace: chord })}
                  >
                    {capture?.action === row.id && capture.replace === chord
                      ? t("settings.pressKeys")
                      : prettyChord(chord)}
                  </button>
                  <button
                    className="hk-x"
                    onClick={() => useKeymap.getState().unbindChord(chord)}
                  >
                    <Icon of={X} size={10} />
                  </button>
                </span>
              ))}
              <button
                className={clsx("hk-add", capture?.action === row.id && !capture.replace && "recording")}
                onClick={() => setCapture({ action: row.id })}
              >
                {capture?.action === row.id && !capture.replace ? t("settings.pressKeys") : "+"}
              </button>
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
