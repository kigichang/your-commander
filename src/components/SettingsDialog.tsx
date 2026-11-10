import { getName, getTauriVersion, getVersion } from "@tauri-apps/api/app";
import clsx from "clsx";
import type { LucideIcon } from "lucide-react";
import {
  ChevronDown,
  ChevronUp,
  Columns2,
  Eye,
  GripVertical,
  Info,
  Keyboard,
  Palette,
  Puzzle,
  Settings,
  Star,
  Wrench,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listFontFamilies, officeTools } from "../ipc/commands";
import { isMac } from "../lib/platform";
import type { ActionId } from "../keymap/actions";
import { actions } from "../keymap/actions";
import type { PresetName } from "../keymap/keymap";
import { chordOf, prettyChord } from "../keymap/keymap";
import type { ExternalTool } from "../ipc/types";
import type { Dialog } from "../store/dialogs";
import { useDialogs } from "../store/dialogs";
import { useKeymap } from "../store/keymapStore";
import { useOps } from "../store/ops";
import { selectFkeysWithoutFn, selectShowHidden, useSettings } from "../store/settings";
import { Icon } from "./icons";
import { Overlay } from "./Overlay";
import appIcon from "../../src-tauri/icons/128x128@2x.png";

/** Finder-style toolbar tabs: icon over label. */
const SECTIONS: ReadonlyArray<{ id: string; icon: LucideIcon }> = [
  { id: "general", icon: Settings },
  { id: "appearance", icon: Palette },
  { id: "favorites", icon: Star },
  { id: "quickview", icon: Eye },
  { id: "hotkeys", icon: Keyboard },
  { id: "panes", icon: Columns2 },
  { id: "tools", icon: Wrench },
  { id: "plugins", icon: Puzzle },
  { id: "about", icon: Info },
];

export function SettingsDialog({ d }: { d: Extract<Dialog, { type: "settings" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [section, setSection] = useState<string>(d.section ?? "general");
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
            "warning",
          );
        }
        setCapture(null);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [capture, t]);

  // While a chord is being recorded, the handler above owns Escape.
  return (
    <Overlay onClose={close} onEscape={() => !capture && close()} className="wide settings">
      <header className="settings-head">
        <button className="settings-close" title={t("settings.done")} onClick={close}>
          <Icon of={X} size={13} />
        </button>
        <h2>{t("settings.title")}</h2>
      </header>
      <nav className="settings-tabs" role="tablist">
        {SECTIONS.map(({ id, icon }) => (
          <button
            key={id}
            role="tab"
            aria-selected={section === id}
            className={clsx(section === id && "on")}
            onClick={() => setSection(id)}
          >
            <Icon of={icon} size={22} />
            <span>{t(`settings.sec_${id}`)}</span>
          </button>
        ))}
      </nav>
      <div className="settings-body">
        {section === "general" ? (
          <GeneralPane />
        ) : section === "appearance" ? (
          <AppearancePane />
        ) : section === "quickview" ? (
          <QuickViewPane />
        ) : section === "panes" ? (
          <PanesPane />
        ) : section === "tools" ? (
          <ToolsPane />
        ) : section === "favorites" ? (
          <FavoritesPane />
        ) : section === "plugins" ? (
          <PluginsPane />
        ) : section === "about" ? (
          <AboutPane />
        ) : (
          <HotkeysPane capture={capture} setCapture={setCapture} />
        )}
      </div>
    </Overlay>
  );
}

function GeneralPane() {
  const { t } = useTranslation();
  const showHidden = useSettings(selectShowHidden);
  const fkeysOn = useSettings(selectFkeysWithoutFn);
  const fkeysAvailable = useSettings((s) => s.fkeysAvailable);
  const config = useSettings((s) => s.config);

  return (
    <div className="settings-pane">
      <label className="check">
        <input
          type="checkbox"
          checked={config?.save_tabs ?? true}
          onChange={(e) => useSettings.getState().save({ save_tabs: e.target.checked })}
        />
        {t("settings.saveTabs")}
      </label>
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
    </div>
  );
}

function AboutPane() {
  const { t } = useTranslation();
  // Asked of the running binary, so it is the installed build's version
  // rather than whatever the frontend was compiled against.
  const [info, setInfo] = useState({ name: "Your Commander", version: "", tauri: "" });
  useEffect(() => {
    void Promise.all([getName(), getVersion(), getTauriVersion()])
      .then(([name, version, tauri]) => setInfo({ name, version, tauri }))
      .catch(() => {});
  }, []);

  return (
    <div className="settings-pane about-pane">
      <img className="about-icon" src={appIcon} alt="" draggable={false} />
      <div className="about-name">{info.name}</div>
      <div className="about-version">
        {t("settings.aboutVersion", { version: info.version || "…" })}
      </div>
      <dl className="about-info">
        <dt>{t("settings.aboutPlatform")}</dt>
        <dd>{isMac ? "macOS" : "Linux"}</dd>
        <dt>Tauri</dt>
        <dd>{info.tauri || "…"}</dd>
        <dt>{t("settings.aboutLicense")}</dt>
        <dd>MIT</dd>
      </dl>
      <div className="settings-note">© 2026 Kigi Chang</div>
    </div>
  );
}

const FONT_SCALES = [0.9, 1, 1.1, 1.25, 1.5];

function AppearancePane() {
  const { t, i18n } = useTranslation();
  const config = useSettings((s) => s.config);
  const [families, setFamilies] = useState<string[]>([]);
  useEffect(() => {
    void listFontFamilies()
      .then(setFamilies)
      .catch(() => {});
  }, []);

  /** Installed families, plus the saved one should it be gone now. */
  const fontOptions = (current: string) =>
    (current && !families.includes(current) ? [current, ...families] : families).map((f) => (
      <option key={f} value={f}>
        {f}
      </option>
    ));
  const { setFonts } = useSettings.getState();

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
      <label>{t("settings.font")}</label>
      <select
        value={config?.font_family ?? ""}
        onChange={(e) => setFonts({ font_family: e.target.value })}
      >
        <option value="">{t("settings.fontDefault", { name: "Inter" })}</option>
        {fontOptions(config?.font_family ?? "")}
      </select>
      <label>{t("settings.fontMono")}</label>
      <select
        value={config?.font_mono ?? ""}
        onChange={(e) => setFonts({ font_mono: e.target.value })}
      >
        <option value="">{t("settings.fontDefault", { name: "JetBrains Mono" })}</option>
        {fontOptions(config?.font_mono ?? "")}
      </select>
      <label>{t("settings.fontSize")}</label>
      <select
        value={String(
          // The config stores an f32; match it to the nearest choice.
          FONT_SCALES.find((s) => Math.abs(s - (config?.font_scale || 1)) < 0.01) ?? 1,
        )}
        onChange={(e) => setFonts({ font_scale: Number(e.target.value) })}
      >
        {FONT_SCALES.map((s) => (
          <option key={s} value={String(s)}>
            {Math.round(s * 100)}%{s === 1 ? ` (${t("settings.fontSizeDefault")})` : ""}
          </option>
        ))}
      </select>
    </div>
  );
}

function PluginsPane() {
  const { t } = useTranslation();
  const config = useSettings((s) => s.config);
  const plantumlMode = config?.plantuml_mode ?? "off";

  return (
    <div className="settings-pane">
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
      <OfficeSettings />
    </div>
  );
}

/** Office previews: LibreOffice status, its path, and whether the modern
 *  formats go through it too. */
function OfficeSettings() {
  const { t } = useTranslation();
  const config = useSettings((s) => s.config);
  const custom = config?.office_soffice ?? "";
  const [soffice, setSoffice] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    let gone = false;
    // Typing a path re-checks it; a short pause keeps that to one lookup.
    const timer = setTimeout(() => {
      void officeTools(custom)
        .then((r) => !gone && setSoffice(r.soffice))
        .catch(() => !gone && setSoffice(null));
    }, 300);
    return () => {
      gone = true;
      clearTimeout(timer);
    };
  }, [custom]);

  return (
    <>
      <label>{t("settings.office")}</label>
      <div className="settings-note office-status">
        {soffice === undefined
          ? "…"
          : soffice
            ? t("settings.officeFound", { path: soffice })
            : t("settings.officeMissing")}
      </div>
      <label>{t("settings.officeSofficePath")}</label>
      <input
        className="wide-in"
        value={custom}
        spellCheck={false}
        placeholder={t("settings.officeSofficeAuto")}
        onChange={(e) => useSettings.getState().save({ office_soffice: e.target.value })}
      />
      <label className="check">
        <input
          type="checkbox"
          disabled={!soffice}
          checked={!!soffice && (config?.office_use_lo ?? false)}
          onChange={(e) => useSettings.getState().save({ office_use_lo: e.target.checked })}
        />
        {t("settings.officeUseLo")}
      </label>
      <div className="settings-note">{t("settings.officeNote")}</div>
    </>
  );
}

function PanesPane() {
  const { t } = useTranslation();
  const config = useSettings((s) => s.config);

  return (
    <div className="settings-pane">
      <label>{t("settings.menus")}</label>
      <label className="check">
        <input
          type="checkbox"
          checked={config?.menu_vscode ?? false}
          onChange={(e) => useSettings.getState().save({ menu_vscode: e.target.checked })}
        />
        {t("action.newVscodeHere")}
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={config?.menu_zed ?? false}
          onChange={(e) => useSettings.getState().save({ menu_zed: e.target.checked })}
        />
        {t("action.newZedHere")}
      </label>
    </div>
  );
}

function QuickViewPane() {
  const { t } = useTranslation();
  const config = useSettings((s) => s.config);

  return (
    <div className="settings-pane">
      <label>{t("settings.qvVideo")}</label>
      <label className="check">
        <input
          type="checkbox"
          checked={config?.qv_autoplay ?? false}
          onChange={(e) => useSettings.getState().save({ qv_autoplay: e.target.checked })}
        />
        {t("settings.qvAutoplay")}
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={config?.qv_mute ?? false}
          onChange={(e) => useSettings.getState().save({ qv_mute: e.target.checked })}
        />
        {t("settings.qvMute")}
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={config?.qv_ffmpeg_poster ?? false}
          onChange={(e) =>
            useSettings.getState().save({ qv_ffmpeg_poster: e.target.checked })
          }
        />
        {t("settings.qvFfmpegPoster")}
      </label>
    </div>
  );
}

const EMPTY_TOOL: ExternalTool = { name: "", app: "", extensions: "", dirs: false };

function ToolsPane() {
  const { t } = useTranslation();
  const config = useSettings((s) => s.config);
  const tools = config?.external_tools ?? [];
  const [selected, setSelected] = useState<number | null>(null);
  const [draft, setDraft] = useState<ExternalTool>(EMPTY_TOOL);

  const update = (next: ExternalTool[]) =>
    useSettings.getState().save({ external_tools: next });
  const edit = (i: number, patch: Partial<ExternalTool>) => {
    const next = tools.slice();
    next[i] = { ...next[i], ...patch };
    update(next);
  };
  const remove = (i: number) => {
    update(tools.filter((_, j) => j !== i));
    setSelected((s) => (s === null || s === i ? null : s > i ? s - 1 : s));
  };

  // The form edits the selected tool in place (instant save, like the rest
  // of Settings) or, with nothing selected, a draft for the Add button.
  const sel = selected !== null && selected < tools.length ? selected : null;
  const cur = sel !== null ? tools[sel] : draft;
  const set = (patch: Partial<ExternalTool>) =>
    sel !== null ? edit(sel, patch) : setDraft({ ...draft, ...patch });

  return (
    <div className="settings-pane">
      {tools.length === 0 ? (
        <div className="settings-note">{t("settings.toolNone")}</div>
      ) : (
        <div className="saved-list tool-list">
          {tools.map((tool, i) => (
            <div key={i} className={clsx("saved", sel === i && "on")}>
              <button className="pick" onClick={() => setSelected(i)}>
                <span className="label">{tool.name}</span>
                <span className="sub">
                  {tool.app} · {tool.extensions || t("settings.toolAllFiles")}
                  {tool.dirs ? ` · ${t("settings.toolDirs")}` : ""}
                </span>
              </button>
              <button
                className="close"
                title={t("settings.toolRemove")}
                onClick={() => remove(i)}
              >
                <Icon of={X} size={12} />
              </button>
            </div>
          ))}
          <button
            className="tab-new new-conn"
            onClick={() => {
              setSelected(null);
              setDraft(EMPTY_TOOL);
            }}
          >
            + {t("settings.toolNew")}
          </button>
        </div>
      )}
      <div className="tool-form-title">
        {sel !== null
          ? t("settings.toolEdit", { name: cur.name })
          : t("settings.toolNew")}
      </div>
      <label>{t("settings.toolName")}</label>
      <input
        className="wide-in"
        value={cur.name}
        placeholder={isMac ? "Extract with Keka" : "Extract with Xarchiver"}
        spellCheck={false}
        onChange={(e) => set({ name: e.target.value })}
      />
      <label>{t("settings.toolApp")}</label>
      <input
        className="wide-in"
        value={cur.app}
        placeholder={isMac ? "Keka" : "xarchiver"}
        spellCheck={false}
        onChange={(e) => set({ app: e.target.value })}
      />
      <label>{t("settings.toolExts")}</label>
      <input
        className="wide-in"
        value={cur.extensions}
        placeholder="zip,7z,rar,tar,gz,tgz"
        spellCheck={false}
        onChange={(e) => set({ extensions: e.target.value })}
      />
      <label className="check">
        <input
          type="checkbox"
          checked={cur.dirs}
          onChange={(e) => set({ dirs: e.target.checked })}
        />
        {t("settings.toolDirs")}
      </label>
      {sel === null && (
        <div className="tool-add">
          <button
            className="btn"
            disabled={!draft.name.trim() || !draft.app.trim()}
            onClick={() => {
              update([
                ...tools,
                { ...draft, name: draft.name.trim(), app: draft.app.trim() },
              ]);
              setDraft(EMPTY_TOOL);
              setSelected(tools.length);
            }}
          >
            {t("settings.toolAdd")}
          </button>
        </div>
      )}
      <div className="settings-note">{t("settings.toolHint")}</div>
    </div>
  );
}

function FavoritesPane() {
  const { t } = useTranslation();
  const config = useSettings((s) => s.config);
  const favorites = config?.favorites ?? [];
  // Index of the row being dragged by its grip (mouse-based: the webview's
  // native drag-drop interception breaks HTML5 DnD on macOS).
  const [dragIdx, setDragIdx] = useState<number | null>(null);

  useEffect(() => {
    if (dragIdx === null) return;
    const up = () => setDragIdx(null);
    window.addEventListener("mouseup", up);
    return () => window.removeEventListener("mouseup", up);
  }, [dragIdx]);

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
          <div
            key={f.path}
            className={clsx("hk-row fav-row", dragIdx === i && "dragging")}
            onMouseEnter={() => {
              // Live reorder: the dragged row follows the cursor row by row.
              if (dragIdx !== null && dragIdx !== i) {
                move(dragIdx, i);
                setDragIdx(i);
              }
            }}
          >
            <span
              className="fav-grip"
              title={t("settings.favDrag")}
              onMouseDown={(e) => {
                e.preventDefault();
                setDragIdx(i);
              }}
            >
              <Icon of={GripVertical} size={13} />
            </span>
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
