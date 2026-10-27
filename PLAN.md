# Your-Commander 重做計畫：Tauri 2 + React（Nocturne UI）

## Context

取得新的 UI 設計稿（`File Manager Screens.html`，內含 "Nocturne" 深色設計系統與 10 個畫面），重做專案並重新評估技術棧。

舊版為 Rust + egui，共 4,579 行。其中 `src/fs/`（FsProvider trait、local、SFTP via russh、SMB 掛載管理）與 `src/ops/`（操作佇列＋衝突處理、copy/move、壓縮/解壓、刪除）約 2,200 行是 UI 無關的，幾乎可直接移植；只有 `app.rs`、`ui/`、`main.rs` 綁定 egui 需捨棄。

**技術棧重評**：設計稿是精緻的自訂深色 Web UI（Inter 字體、OKLCH 色階、PDF 預覽、複雜對話框），egui 無法達到此還原度；設計稿本身就是 HTML/CSS，Web 前端可直接沿用其 design tokens。

**已定案決策：**
- 技術棧：**Tauri 2 + Rust 後端 + React + TypeScript（Vite）**
- 範圍：**分階段** — Phase 1 = 已定案畫面 3a（主視窗）+ 3d（Quick View）+ 基本檔案操作 + 壓縮 + SMB + SFTP；其餘 8 個「待討論」草稿畫面（傳輸佇列、批次改名、資料夾同步、搜尋、壓縮檔瀏覽、多協定連線、快捷鍵設定、指令面板）只預留擴充點
- 同一 repo 原地重構，刪除舊 egui UI，git 歷史保留舊版
- i18n：英文預設 + zh-TW（現有 `locales/*.yml` 轉為 i18next JSON）

## 設計稿重點（Nocturne）

Tokens：bg `#161826`、surface `#232532`、text `#e9e9ed`、accent `#9184d9`（藍紫）、accent-2 `#a7a1db`、divider = 16% 白、radius 4/8/14px、Inter 字體、OKLCH neutral/accent 色階（完整 CSS 在設計稿 `__bundler/template` 的 `<style>` 區塊內，可整段抽出為 `tokens.css`）。

畫面 3a：頂部工具列（Refresh、Brief/Full、Swap、Compare*、Rename*、Find*、Sync*、Connect；* = 未來階段按鈕，先 stub）；每面板：分頁列、磁碟下拉（含剩餘空間）、可編輯路徑列＋萬用字元過濾、我的最愛星號；檔案表格欄位 Name/Ext/Size/Date 可排序；游標 = accent 左線＋底色、標記檔 = accent 文字；狀態列「2 of 10 marked · 4.1 MB of 22 MB」；底部 F-key 列（F3 Quick View、F4 Edit、F5 Copy、F6 Move、F7 New Folder、F8 Trash、⌘K、⌘Q）；「F-keys 免按 fn（預設開）」。

畫面 3d：F3 將對側面板切為預覽（PDF 翻頁、Kind/Size/Modified/Tags、Open ↵ / Reveal），預覽跟隨游標，F3/✕ 返回。

## 1. Repo 重構

目標結構：

```
your-commander/
├── package.json, vite.config.ts, tsconfig.json, index.html
├── src/                       # React 前端
│   ├── main.tsx, App.tsx
│   ├── styles/   tokens.css（Nocturne）, base.css
│   ├── ipc/      commands.ts, events.ts, types.ts（typed invoke/Channel 包裝）
│   ├── store/    panels.ts, ops.ts, settings.ts, connections.ts（Zustand）
│   ├── keymap/   actions.ts, keymap.ts, useKeyboard.ts
│   ├── components/ Toolbar/, Panel/, FileTable/, QuickView/, FKeyBar/, dialogs/
│   ├── i18n/     index.ts, locales/en.json, locales/zh-TW.json
│   └── lib/      format.ts, path.ts, wildcard.ts
├── src-tauri/
│   ├── Cargo.toml, tauri.conf.json, build.rs, capabilities/default.json, icons/
│   └── src/
│       ├── main.rs, lib.rs（Builder、managed state、command 註冊）
│       ├── commands/  fs.rs, ops.rs, archive.rs, smb.rs, sftp.rs, config.rs, preview.rs, system.rs
│       ├── fs/        provider.rs, local.rs, sftp.rs, smb.rs      ← 自舊 src/ 移入
│       ├── ops/       queue.rs, transfer.rs, remote.rs, archive.rs, delete.rs ← 移入
│       ├── state.rs（AppState）, watch.rs（notify）, fnkeys.rs（M7）
│       └── config.rs, keychain.rs, util.rs（移入）, error.rs（新）
└── PLAN.md, AGENTS.md（更新）
```

步驟：
1. 手動 scaffold（避免 create-tauri-app 覆蓋）：加 `package.json`（vite、react、typescript、`@tauri-apps/cli@^2`、`@tauri-apps/api@^2`）、`vite.config.ts`（port 1420、`clearScreen:false`）、`npx tauri init` 指向 Vite devUrl。
2. 移植：`src/fs/`、`src/ops/`、`config.rs`、`keychain.rs`、`util.rs` → `src-tauri/src/`；刪除 `app.rs`、`ui/`、舊 `main.rs`。
3. `Cargo.toml` → `src-tauri/Cargo.toml`。移除 `eframe`/`egui`/`egui_extras`/`rust-i18n`/`sys-locale`；保留既有版本的 `russh 0.64`、`russh-sftp 3`、`zip 9`、`tar`、`flate2`、`sevenz-rust2`、`trash`、`notify`、`security-framework`、`ssh2-config`、`chrono`、`dirs`、`open`、`toml`、`serde`、`tokio`、`urlencoding`；新增 `tauri = "2"`、`tauri-build`、`serde_json`、`thiserror`、`libc`（statfs 磁碟空間）。
4. 去除 `fs/`+`ops/` 的 `rust_i18n::t!`：新增 `error.rs`，`YcError { code, detail }` 可 Serialize，前端以 i18next 對 code 翻譯。`FsEntry` 加 `Serialize`（camelCase、epoch-ms 時間）；`kind_label()` 移除（前端推導）。
5. `queue.rs` 移除 `egui::Context` 依賴；`OpsManager` 改為 Tauri managed state。
6. `npm i zustand @tanstack/react-virtual i18next react-i18next pdfjs-dist clsx`；`locales/*.yml` → `src/i18n/locales/*.json` 後刪除 `locales/`。
7. `.gitignore` 加 `node_modules/`、`dist/`；`target/` 移到 `src-tauri/` 下。

## 2. Rust 後端架構

**狀態切分**：前端擁有所有面板狀態（分頁、cwd、游標、標記、排序、過濾、檢視模式、Quick View）；後端擁有 SFTP 連線表、SMB 掛載、操作佇列、目錄監看、設定檔、Keychain。路徑定址：`Location = { kind:"local" } | { kind:"sftp", connId }` + path，所有 fs command 吃 Location 再分派給 `FsProvider`。

**managed state（state.rs）**：`connections: Mutex<HashMap<u32, Arc<SftpConnection>>>`、`ops: Mutex<HashMap<OpId, OpControl>>`（cancel AtomicBool + conflict_tx）、`watchers`、`config`。

**Commands（快速請求/回應）**：
- fs.rs：`list_dir(loc, path) -> Vec<FsEntry>`、`mkdir`、`rename`、`stat`、`exists`
- system.rs：`volumes()`（舊 `local::volumes()` + `libc::statfs` 補剩餘空間）、`home_dir`、`open_path`、`reveal_in_finder`（`open -R`）
- preview.rs：`read_preview(loc, path, max_bytes) -> tauri::ipc::Response`（raw bytes，local/SFTP 通用，餵 pdf.js / 圖片 / 文字）
- smb.rs：`smb_list_saved`、`smb_connect`（掛載→回傳 /Volumes 路徑）、`smb_unmount`、`smb_detect_mounted`
- sftp.rs：`ssh_hosts()`（ssh2-config 解析）、`sftp_connect -> connId`、`sftp_disconnect`
- config.rs：`get_config`、`set_config`；密碼只在連線當下短暫經過 JS，儲存一律走 Keychain
- watch.rs：`watch_dir`/`unwatch` → debounce ~300ms 後 emit `fs:changed`（Phase 1 僅 local，SFTP 手動刷新）

**長時間操作**：沿用舊 worker-thread 模型（`copy_op`/`trash_op`/`delete_op`/`compress_op`/`extract_op`/`upload_op`/`download_op` + `OpCtx`），僅換傳輸層：
- `start_op(request, on_event: tauri::ipc::Channel<OpEvent>) -> OpId`；Channel 依序送 `Progress { fraction, doneBytes, totalBytes, detail }`、`Conflict { src, dest }`、`Done { result }`。高頻進度用 Channel（有序、不污染全域 event bus），`emit` 只用於 watcher ping。
- 衝突往返：`OpCtx::ask_conflict` 保留阻塞 `mpsc`，Sender 存入 `AppState.ops[id].conflict_tx`；前端以 `resolve_conflict(opId, choice)` 回覆（支援 Overwrite-All/Skip-All sticky policy，沿用舊邏輯）；`cancel_op(opId)` 翻 AtomicBool 並釋放等待中的衝突。
- 跨 provider copy：`commands/ops.rs` 依兩端 Location 路由到 `transfer::copy_op`（local↔local）或 `remote::upload_op`/`download_op`（local↔SFTP），同舊 `app.rs` 作法。
- Trash 僅 local；SFTP 刪除一律永久（確認對話框註明），沿用舊區分。
- tokio：Tauri 自帶 runtime，`sftp.rs` 的私有 `runtime()` OnceLock 可保留（worker thread 內 block_on），勿在 async command 內 block_on。

## 3. React 前端架構

**狀態：Zustand**。鍵盤驅動的單一 store 形狀最合適——actions 是普通函式可從 keymap 層直接呼叫，selector 細粒度避免檔案表格無謂重繪。Slices：`panels.ts`（`panels:[PanelState,PanelState]`、`activeSide`；Tab = location/path/entries/cursor/marked/sort/filter/viewMode）、`ops.ts`（進行中操作、待回覆衝突）、`settings.ts`、quickview 狀態。

**檔案表格**：`@tanstack/react-virtual` 固定列高 ~24px 虛擬捲動；Name/Ext/Size/Date 可排序欄頭；Brief 模式 = 多欄純檔名（同樣虛擬化）；游標列 = accent 左線＋tint、標記 = accent 文字；狀態列由 selector 計算。

**鍵盤模型**（為未來快捷鍵設定畫面鋪路）：
- `actions.ts`：`Record<ActionId, { run(ctx), when? }>` 註冊表（quickView、copy、move、newFolder、trash、navUp、enter、markToggle、swap…）
- `keymap.ts`：純資料 `ActionId → KeyChord[]` 預設表（`F5` 與 `⌘⇧C` 並列），與 config 中使用者覆寫合併——未來設定畫面只是編輯這張表
- `useKeyboard.ts`：單一 window keydown listener，scope 堆疊（dialog > quickview > panel）分派；輸入框聚焦時除白名單外忽略

**主題**：`tokens.css` 收錄 Nocturne 全部 custom properties（自設計稿抽出）；Inter 以 `@font-face` 打包（離線 app，不打 Google Fonts）；元件只消費變數。

**元件（3a/3d）**：`App` → `TitlebarRegion`（drag region 蓋過紅綠燈）＋ `Toolbar` ＋ `PanelPair` → 2×`Panel`（`TabStrip`、`DriveSelector`、`PathBar`、`FavoriteStar`）→ `FileTable` ＋ `StatusLine`；底部 `FKeyBar`。`QuickView` 取代非作用側面板主體：`PdfPreview`（pdf.js canvas + 翻頁）、`ImagePreview`、`TextPreview`、`MetaPanel`、Open/Reveal；訂閱對側游標（debounce）。Dialogs：`ConflictDialog`、`ProgressStrip`（Phase 1 最小進度 UI）、`NewFolder`、`Rename`、`ConfirmDelete`、`ConnectSmb`、`ConnectSftp`。

**預覽**：`pdfjs-dist`，worker 以 Vite `?url` 打包；吃 `read_preview` 回傳的 ArrayBuffer（local/SFTP 一致）。圖片走 blob URL；文字取前 256KB＋binary 偵測。

**i18n**：i18next + react-i18next，`navigator.language` 偵測（zh* → zh-TW），持久化於 config；後端錯誤 code 走同一套 bundle。

## 4. Tauri 2 注意事項

- **IPC 大小**：大目錄（5 萬項 ≈ 10MB JSON）可行但要精簡序列化；若量測到瓶頸，`list_dir` 改 `tauri::ipc::Response` raw bytes——`ipc/commands.ts` 包裝層確保只改一個檔。
- **Capabilities**：`capabilities/default.json` 僅需 `core:default` 等；自訂 command 註冊即可用，不引入 `tauri-plugin-shell`/`-fs`（shell/open 直接在 Rust 做），省去 scope 設定。
- **CSP**：`"default-src 'self'; img-src 'self' asset: blob: data:; worker-src 'self' blob:"`（pdf.js worker 與 blob 預覽需要）。
- **視窗**：`"titleBarStyle":"Overlay"`、`"hiddenTitle":true`、`trafficLightPosition {x:16,y:18}`、`"theme":"Dark"`、minWidth 800；工具列加 `data-tauri-drag-region`。不需 `macos-private-api`。
- **不簽章**：`bundle.macOS.signingIdentity: "-"`（ad-hoc），個人使用可直接執行。

## 5. Phase 1 里程碑（各含驗證）

1. **M1 骨架與移植編譯**：Vite+React 空殼（Nocturne 底色、overlay titlebar 正確）；src-tauri 移入 fs/ops/config/keychain 並編譯過（去 egui/rust-i18n、error.rs 就位）。驗證：`npm run tauri dev` 開出深色視窗；`cargo test`（util/queue 既有測試）通過。
2. **M2 本機瀏覽（3a 唯讀）**：list_dir/volumes/config commands；Zustand 面板＋分頁；虛擬化表格（排序/游標/標記/Mac 式選取）；路徑列＋萬用字元過濾；磁碟下拉；狀態列；FKeyBar；i18n；鍵盤導覽。驗證：1 萬檔目錄順暢捲動、各欄排序、過濾、Tab 切面板、Enter/Backspace 導覽、語言切換。
3. **M3 檔案操作＋進度＋監看**：op Channel 接通；F5 copy / F6 move（active→對側）＋ ProgressStrip＋取消＋ConflictDialog（含 apply-to-all / keep-both）；F7 新資料夾；rename；F8 垃圾筒＋⌘⇧⌫ 永久刪除（確認）；notify 自動刷新。驗證：多 GB 資料夾複製進度正確、中途取消；製造衝突測全部選項；Finder 中增刪檔案面板自動更新。
4. **M4 壓縮**：壓縮對話框（zip/tar/gz/tgz，排除 `.DS_Store`/`._*`）輸出到對側；解壓 zip/7z/tar/gz/tgz。驗證：各格式 round-trip、`zipinfo` 確認排除、解壓外部 7z、中途取消。
5. **M5 Quick View（3d）**：F3 切換；pdf.js 翻頁；圖片/文字預覽；中繼資料（Tags 走 `mdls`，失敗則優雅略過）；Open/Reveal；預覽跟隨游標（debounce）。驗證：方向鍵掃過 PDF/圖片/程式碼資料夾預覽即時跟隨；500MB 檔案不凍結（大小上限生效）。
6. **M6 SMB + SFTP**：ConnectSmb（已存連線、Keychain、掛載→導向 /Volumes/x、偵測已掛載、卸載）；ConnectSftp（ssh2-config host 清單、金鑰＋密碼、Keychain）；SFTP 分頁（Location{sftp,connId}）；跨 provider F5/F6 串流；SFTP 永久刪除警示。驗證：真實 NAS/SSH 主機——掛載後 Finder 可見、SFTP 雙向多檔傳輸含進度與衝突、斷線關閉分頁。
7. **M7 收尾＋F-keys**：`fnkeys.rs` CGEventTap（app 聚焦時媒體鍵→F1–F12，設定開關預設開，Accessibility 權限提示、拒絕時降級用 ⌘ 等價鍵）；Brief/Full 與 Swap；持久化分頁/路徑/排序/視窗；zh-TW 補完；`tauri build` 出 ad-hoc 簽章 .app。驗證：授權後 F5 免 fn 可用；拒絕後 ⌘⇧C 仍可；重啟還原 session；build 的 .app 可從 /Applications 執行。

## 6. 未來 8 畫面擴充點

1. **傳輸佇列（1e）**：ProgressStrip 換成完整佇列視圖，`ops.ts` store 與後端多 op 併發已支援。
2. **批次改名（1f）**：新 batch-rename op 重用 `transfer::rename`，對話框吃 active tab 的 `marked` 集合。
3. **資料夾同步（1g）**：新 `ops/sync.rs` 以 `FsProvider::list_dir` 比對兩個 Location 出 diff 模型；工具列按鈕已 stub。
4. **搜尋（1h）**：新 streaming command（`Channel<FoundEntry>`）走訪 Location；結果畫面重用 FileTable。
5. **壓縮檔瀏覽（2d）**：第三種 Location `{archive, inner}` + `ArchiveFs: FsProvider` 包現有 zip/tar reader；面板層已與 Location 無關。
6. **多協定連線（2e）**：由 ConnectSmb/ConnectSftp 對話框長出，後端連線表以協定為 key。
7. **快捷鍵設定（2f）**：純前端——編輯 `keymap.ts` 使用者覆寫表並存 config。
8. **指令面板 ⌘K（2g）**：對 `actions.ts` 註冊表做 fuzzy 清單（action 已有 id＋i18n label）。

## 關鍵檔案

- 移植來源：`src/fs/provider.rs`、`src/fs/sftp.rs`、`src/fs/smb.rs`、`src/ops/queue.rs`、`src/ops/archive.rs`、`src/config.rs`、`src/keychain.rs`
- 設計來源：`File Manager Screens.html`（`__bundler/template` 內的 Nocturne `<style>` 區塊 → `src/styles/tokens.css`）
- 版本依據：現有 `Cargo.toml` 的 known-good crate 版本

## 收尾事項

- 更新 AGENTS.md 註明新技術棧與啟動方式（`npm run tauri dev`）。
- `File Manager Screens.html` 保留在 repo 根目錄作為設計參照。
