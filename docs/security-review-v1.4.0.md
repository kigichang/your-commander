# Your Commander 資安複審與交接修改規格（v1.4.0）

- **審查版本**：`v1.4.0`（commit `48e93dd`，branch `main`）
- **審查日期**：2026-11-24
- **前次報告對照**：`docs/code-review-report.md`（`v1.3.2 → v1.4.0` 已完成之第一輪修補紀錄）
- **文件用途**：整理 `v1.4.0` 剩餘之資安缺口、根因定位、具體最小修改方案與回歸測試要求，供後續開發 Agent（Claude Fable）直接接手實作。

---

## 給接手 Agent（Claude Fable）的執行守則

1. **驗證基線指令**：
   - 後端測試：`cd src-tauri && cargo test`
   - 後端靜態檢查：`cd src-tauri && cargo clippy --all-targets`
   - 前端型別與建置：`npx tsc -b && npm run build`
   - 相依套件檢查：`npm ls katex i18next`、`npm audit --omit=dev`、`cd src-tauri && cargo audit`
2. **跨平台注意事項**：
   - 專案同時支援 macOS 與 Linux（Ubuntu）。修改共用模組或 `src-tauri/src/platform/` 時，請同步檢查 `#[cfg(target_os = "macos")]` 與 `#[cfg(target_os = "linux")]` 兩側編譯相容性（若有 Docker 環境可用 `docker/linux-dev.Dockerfile` 跑 Linux `cargo test`）。
3. **修改與測試紀律**：
   - 建議**每個問題編號獨立一個 commit**，commit 訊息清楚記錄問題與解法。
   - 後端安全修正請務必附上**單元／回歸測試**（可參考既有 `preview.rs`、`stream_protocol.rs`、`safe_path.rs`、`office.rs` 的測試慣例），確保修正前可重現問題、修正後通過。
   - **切勿破壞既有開發流程**：例如 `tauri.conf.json` 的 `"signingIdentity": "-"` 是專案在無付費 Apple Developer 憑證環境下打包 `.app`/`.dmg` 所需（見 `AGENTS.md`），請勿擅自改動，除非使用者明確提供簽章身分。

---

## 問題總覽與追蹤表

| 編號 | 嚴重度 | 模組 / 檔案 | 摘要 | 狀態（由接手者更新） |
| :--- | :---: | :--- | :--- | :--- |
| **SEC-2-M1** | **Medium** | `src-tauri/src/commands/office.rs`<br>`src/components/QuickView.tsx` | Quick View（0-click）LibreOffice 轉檔因內容嗅探可繞過副檔名白名單，觸發靜默外部連線（IP 洩露 / SSRF）且本機檔案缺 `open_regular` 防護 | 已修正（外聯未能重現，見文末） |
| **SEC-2-M2** | **Medium** | `package.json`<br>`src/components/MarkdownPreview.tsx` | `@vscode/markdown-it-katex` 巢狀鎖定舊版 `katex@0.16.47`（`GHSA-238p-pmpm-9mq7`），且 Markdown / Mermaid 直接渲染於具完整 IPC 權限之主 DOM | 已修正 |
| **SEC-2-M3** | **Medium** | `src-tauri/src/state.rs`<br>`src-tauri/src/fs/archive_fs.rs`<br>`src-tauri/src/ops/{sync,transfer,archive}.rs` | `Location::Archive`、目錄同步、複製與壓縮未排除 FIFO / 特殊裝置檔，遇 named pipe 會造成**無法取消**的工作執行緒永久阻塞（DoS） | 已修正 |
| **SEC-2-L1** | **Low** | `src/components/ConnectDialog.tsx` | 手動輸入密碼（或經 `askPw` 輸入）後變更主機／埠號／帳號未清空 `secret`，可能將密碼誤送至新主機 | 已修正 |
| **SEC-2-L2** | **Low** | `package.json`、`package-lock.json`<br>`src-tauri/Cargo.lock` | `npm audit` 通告修補（`i18next` `GHSA-6jcc-5g8w-32mx`、巢狀 `katex`）與 Linux 傳遞相依追蹤 | 已修正（Linux 傳遞相依警告僅註記） |
| **SEC-2-L3** | **Low** | `.github/workflows/ci.yml` | GitHub Actions 缺少頂層最小 `permissions` 宣告，且第三方 Action 使用浮動標籤而非固定 commit SHA | 已修正 |

---

## 詳細問題分析與修改規格

### [SEC-2-M1] Quick View（0-Click）LibreOffice 轉檔內容嗅探繞過、靜默外聯（SSRF / IP 洩露）與 FIFO 阻塞

#### 位置
- `src-tauri/src/commands/office.rs:40-44`（`CONVERTIBLE_EXTS`）
- `src-tauri/src/commands/office.rs:313-331, 367-389`（`convert_args`、`profile_dir`、`run_soffice`）
- `src-tauri/src/commands/office.rs:426-488`（`office_to_pdf`）
- `src/components/QuickView.tsx:538-558`

#### 問題說明
1. **內容嗅探繞過副檔名白名單（0-Click 網路外聯 / SSRF）**：
   - 當系統裝有 LibreOffice 時，即使 `office_use_lo` 為預設 `false`，只要使用者開啟 F3 Quick View 並將游標移至 `.doc`、`.ppt`、`.xls` 檔案上 180ms（**0-click**，包含 `~/Downloads`、SMB、SFTP 遠端或壓縮檔內檔案），前端就會自動呼叫 `office_to_pdf`。
   - `office_to_pdf` 雖以 `CONVERTIBLE_EXTS` 擋掉 `.html`/`.svg`，但 **LibreOffice 的 `TypeDetection` 依檔案內容而非副檔名選擇匯入器**。若攻擊者將含 `<img src="http://...">` 的 HTML、SVG、XML 或 Flat-ODF 檔案命名為 `invoice.doc`（遠端／壓縮檔分支亦會寫成 `document.doc` 送入 `soffice`），LibreOffice 在 `--convert-to pdf` 排版時仍會自動對外發起 HTTP/HTTPS 請求，造成 **0-click 真實 IP 洩露或對 `127.0.0.1` / 內網服務發起 SSRF**，違反專案在 `HtmlPreview` 與 `plantuml.rs`（預設 `off`）建立的「Quick View 絕不靜默外聯」安全邊界。
2. **本機路徑未驗證一般檔案（FIFO 鎖阻塞）**：
   - `office_to_pdf` 在 `Resolved::Local` 分支（lines 462-465）只呼叫 `require_absolute(&path)`，未使用 `crate::ops::safe_path::open_regular` 排除 FIFO 或裝置檔。若游標經過命名為 `x.doc` 的 FIFO，會獨佔卡住全域唯一的 `office.gate` Mutex 長達 90 秒（`CONVERT_TIMEOUT`）。
3. **缺少 LibreOffice 設定檔安全鎖定與作業系統網路隔離**：
   - `profile_dir()` 建立獨立 profile 目錄，但未預置 `user/registrymodifications.xcu` 鎖定巨集與外部連結設定，也未在支援的作業系統上隔離 `soffice` 網路存取。

#### 修改建議
1. **魔術位元組（Magic Bytes）與一般檔案前檢**：
   - 新增純函式（例如 `check_office_header(ext: &str, head: &[u8]) -> Result<(), String>`）：
     - 當 `ext` 為舊版二進位／RTF 家族（`"doc" | "ppt" | "xls"`）：要求 `head` 以 `OLE2_MAGIC`（`[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]`）開頭（`.doc` 視需求可決定是否允許標準 RTF `b"{\\rtf"`，但**必須拒絕** HTML/XML/SVG 等文字標記如 `<`、BOM 後 `<`）。
     - 當 `ext` 為現代壓縮封裝格式（`"docx" | "docm" | "dotx" | "pptx" | "pptm" | "ppsx" | "potx" | "xlsx" | "xlsm" | "xlsb" | "ods"`）：要求 `head` 以 ZIP local/empty header（`b"PK\x03\x04"` 或 `b"PK\x05\x06"`）開頭。
   - 在 `office_to_pdf` 的 `Resolved::Local` 分支：使用 `crate::ops::safe_path::open_regular(&path)` 開啟並讀取前 16 bytes 傳入 `check_office_header`；遠端／壓縮檔分支則直接對 `bytes` 前 16 bytes 呼叫 `check_office_header`。
   - 注意保留 `libreoffice_converts_to_pdf`（`#[ignore]` 測試）的可測試性（或將該測試輸入改為合法格式／直接測 `run_soffice`）。
2. **封鎖 `soffice` 轉檔期間的網路存取（縱深防禦）**：
   - **macOS**：`/usr/bin/sandbox-exec` 為系統內建，若存在可考慮以 `sandbox-exec -p "(version 1)(allow default)(deny network*)" <soffice> ...` 啟動，從核心層徹底阻斷任何外部連結、遠端圖片、WebDAV 或 DNS 請求（請於 macOS 實測 LibreOffice 在 `deny network*` 下仍能正常使用 unix domain socket / 本地 profile 完成 `--convert-to pdf`，若 LibreOffice 內部需要 `network-outbound (local unix-socket)`，profile 改寫為 `(version 1)(allow default)(deny network* (remote ip))`）。
   - **Profile 加固（選配加分）**：若 `lo-profile/user/registrymodifications.xcu` 尚未存在，可預寫 `MacroSecurityLevel = 3`（Very High）。

#### 回歸測試要求
- 單元測試驗證 `check_office_header`：
  - 偽裝為 `evil.doc` 或 `evil.docx` 的 HTML（`<!doctype html>...`、`   <html>...`、UTF-8 BOM + `<svg>`、`<?xml ...`）皆被拒絕。
  - 合法 `OLE2_MAGIC` 的 `.doc`/`.xls`/`.ppt` 與 `PK\x03\x04` 的 `.docx`/`.xlsx`/`.pptx`/`.ods` 正常通過。
- 單元測試驗證命名為 `fifo.doc` 的本機 FIFO（`libc::mkfifo`）在進入 `run_soffice` 前即被立即拒絕且不阻塞。

---

### [SEC-2-M2] `MarkdownPreview` 巢狀鎖定舊版 `katex@0.16.47` 與主視窗特權 DOM 隔離強化

#### 位置
- `package.json:15, 22, 26` 與 `package-lock.json`
- `src/components/MarkdownPreview.tsx:15-39, 104-130, 165-213`

#### 問題說明
1. **巢狀相依鎖定含已知漏洞之舊版 KaTeX（`GHSA-238p-pmpm-9mq7`）**：
   - `package.json` 雖宣告 `"katex": "^0.19.1"`，但 `npm ls katex` 顯示 `@vscode/markdown-it-katex@1.1.2` 內部 `require("katex")` 實際解析到其子目錄下的 **`katex@0.16.47`**，`mermaid@12.2.0` 亦解析到 `katex@0.18.10`。因此執行期渲染數學公式的並非 `0.19.1`。
2. **主視窗特權 DOM 缺乏獨立淨化／隔離**：
   - 主 WebView（`tauri://localhost`）擁有完整 Tauri IPC 權限（可透過 `update_config` + `open_with` 執行任意本機程式）。
   - `HtmlPreview` 與 `DocxPreview` 皆隔離於 `<iframe sandbox="">`，唯獨 `MarkdownPreview` 在 0-click 預覽 `.md` 時直接將 `markdown-it` + `@vscode/markdown-it-katex` + `highlight.js` 輸出透過 `dangerouslySetInnerHTML` 掛入主 DOM（未經獨立 `DOMPurify`），並接著呼叫 `mermaid.run()` 在主 DOM 內直接插入含 `<foreignObject>` 的 `<svg>`。

#### 修改建議
1. **統一鎖定新版 `katex`**：
   - 在 `package.json` 新增 `"overrides"`：
     ```json
     "overrides": {
       "katex": "^0.19.1"
     }
     ```
   - 執行 `npm install` 更新 `package-lock.json`，確認 `npm ls katex` 所有節點皆指向 `0.19.x` 且無重複舊版。
2. **為 `MarkdownPreview` 補上獨立 Sanitizer 並避免 Mermaid 直接操作主 DOM**：
   - **Mermaid 改轉 `<img>`（比照同檔案 `renderPlantumlBlocks` 的既有做法，lines 84-91）**：
     - 將 `mermaid.run({ nodes })` 改為對每個 `pre.mermaid` 讀取 `node.textContent`，呼叫 `const { svg } = await mermaid.render(uniqueId, code)`，再建立 `<img src={"data:image/svg+xml," + encodeURIComponent(svg)} draggable={false} />` 取代節點內容（渲染失敗時維持顯示原始碼文字）。
     - 好處：徹底消除主 DOM 內的 `<svg>` / `<foreignObject>` 攻擊面，同時保留 Markdown 一般超連結點擊透過 `onClick` → `ipc.openUrl` 開啟系統瀏覽器的既有互動體驗（注意：若 Mermaid 圖表本身幾乎不使用外部點擊連結，此法最乾淨安全；或改將整個 `MarkdownPreview` 放入沙箱 `iframe` 並用 `postMessage` 轉發連結點擊亦可，擇一實作即可）。
   - **HTML 淨化**：引入 `dompurify`（及 `@types/dompurify`）對 `md.render(text)` 產出的 HTML 字串執行 `DOMPurify.sanitize(rawHtml, { FORBID_TAGS: ["style", "form", "input"], FORBID_ATTR: ["style"] })`（注意保留 KaTeX 所需的 `span` / MathML 標籤與 KaTeX inline `style`——若 KaTeX 需要特定 `style` 屬性如 `height`/`vertical-align`，請確認 KaTeX 渲染公式如 `$\frac{a}{b}$` 外觀正常，或僅使用 `DOMPurify.sanitize(rawHtml)` 預設安全白名單）。

#### 驗證要求
- `npm ls katex` 僅出現單一 `0.19.x` 版本。
- `npx tsc -b && npm run build` 通過。
- 確認含程式碼高亮、GFM 待辦清單（`input[type=checkbox][disabled]`）、KaTeX 公式、Mermaid 圖表、PlantUML 圖表與外部 `https://` 連結的 Markdown 預覽皆正常運作。

---

### [SEC-2-M3] `Location::Archive`、目錄同步、檔案複製與壓縮遺漏 FIFO / 特殊檔案防護（無法取消之永久阻塞 DoS）

#### 位置
- `src-tauri/src/state.rs:89-95`（`AppState::resolve`）
- `src-tauri/src/fs/archive_fs.rs:97-99, 125, 196, 307, 449, 515`
- `src-tauri/src/ops/sync.rs:34-59`（`collect_local`）
- `src-tauri/src/ops/transfer.rs:236-237, 307-314`（`copy_file`、`same_contents`）
- `src-tauri/src/ops/archive.rs:65-98, 205-206, 238-246, 252-255, 348-387, 412, 585`

#### 問題說明
專案已在 `preview.rs`、`find.rs`、`stream_protocol.rs`、`media_server.rs` 使用 `safe_path::open_regular` 阻擋 FIFO（named pipe）與字元／區塊裝置（如 `/dev/zero`），但在下列路徑遺漏：
1. **`Location::Archive { archive }`**：
   - `state.resolve()` 未對 `archive` 檢查 `crate::commands::require_absolute(&archive)`。
   - `archive_fs` 所有開檔點皆直接呼叫 `std::fs::File::open(archive)`；若目錄中有命名為 `trap.tar` 或 `trap.zip` 的 FIFO，使用者在面板按 Enter 進入（`actions.ts:168` 僅判斷 `!entry.isDir && BROWSABLE_ARCHIVE_RE.test(entry.name)`）會使後端 blocking worker 永久卡死在 `open(2)`。
2. **目錄比對與同步（`ops/sync.rs`）**：
   - `collect_local`（lines 42-56）僅排除 `kind.is_symlink()` 與 `kind.is_dir()`，因此目錄樹中的 FIFO 或 Unix domain socket 會進入 `else if let Ok(meta) = entry.metadata()` 被收進同步清單。
   - 勾選「依內容比對（`by_content`）」時 `same_contents` 會呼叫 `File::open`；執行同步時 `copy_file` 亦會呼叫 `File::open`，導致**工作執行緒永久阻塞在 `open(2)`，連 UI 的 Cancel 按鈕都無法中斷**（`ctx.cancelled()` 在 `read` 迴圈才輪詢）。
3. **本機複製與壓縮／解壓縮（`ops/transfer.rs`、`ops/archive.rs`）**：
   - 複製含 FIFO（例如專案執行期 socket/pipe、容器掛載目錄）的資料夾時，`transfer::copy_file` 直接 `File::open(src)` 永久卡死。
   - 壓縮含 FIFO 的資料夾時，`collect_tree` 將其視為一般非目錄項目收錄，`write_zip` / `gz_file` 在 `File::open` 永久卡死。
   - `extract_op` 開啟來源壓縮檔時亦直接 `File::open(archive)` 而非 `open_regular`。

> **雲端佔位檔相容性提示**：`safe_path::open_regular`（`src-tauri/src/ops/safe_path.rs:109-131`）已特別為 macOS File Provider（iCloud / Google Drive dataless files）設計——它不使用 `O_NONBLOCK`，而是先以 `std::fs::metadata(path)?.is_file()` 檢查類型後才 `File::open`，因此 FIFO / socket / device 在 `open` 前就會立即回錯，而雲端一般檔案仍可正常阻塞下載。

#### 修改建議
1. **`state.rs` 與 `archive_fs.rs`**：
   - 在 `AppState::resolve` 的 `Location::Archive { archive }` 分支加上 `crate::commands::require_absolute(archive)?;`。
   - 將 `archive_fs.rs` 中開啟 `archive` 的 `std::fs::File::open(archive)` 統一改為 `crate::ops::safe_path::open_regular(archive)`。
2. **`ops/sync.rs`**：
   - `collect_local` line 48 改為 `else if kind.is_file() && let Ok(meta) = entry.metadata()`，明確只將一般檔案納入比對清單（略過 FIFO、socket、device，與略過 symlink 行為一致）。
3. **`ops/transfer.rs`**：
   - `same_contents` 改用 `super::safe_path::open_regular` 開啟 `a` 與 `b`。
   - `copy_file` 在 symlink 分支之後（line 236），改用 `super::safe_path::open_regular(src)` 開啟來源檔（這會同時涵蓋 `follow_symlinks = true` 時指向 FIFO 的 symlink！注意 line 226 的 `src.is_file()` 對於指向 FIFO 的 symlink 本就為 `false`，但直接選取或遍歷到本機 FIFO 時，`open_regular` 可立即回傳明確錯誤而非永久卡住；或者在 `copy_dir` / `copy_file` 遇到 FIFO/socket 時選擇明確報錯 `not a regular file`，讓使用者知道該特殊檔無法複製，絕不死鎖）。
4. **`ops/archive.rs`**：
   - `collect_tree`：對非目錄項目，檢查 `meta.is_file() || meta.is_symlink()`；若為 FIFO / socket / device 則略過（或報錯，建議與 `extract_tar` 略過特殊檔對稱，或回傳明確錯誤——請在測試中明確斷言行為）。
   - `gz_file`（以及 `compress_op` 的 `.gz` 單檔檢查）與 `extract_op` 開啟壓縮檔本身時改用 `super::safe_path::open_regular`。

#### 回歸測試要求
- 以 `libc::mkfifo` 建立測試用 FIFO：
  1. `archive_fs::list` / `archive_fs::read_entry` 傳入命名為 `fifo.zip` 與 `fifo.tar` 的 FIFO 時立即回錯、不阻塞；`state.resolve` 拒絕相對路徑的 `Location::Archive`。
  2. `sync::compare(..., by_content = true)` 在含 FIFO 的資料夾中可正常完成（FIFO 被略過、不阻塞）。
  3. `transfer::copy_op`、`archive::compress_op`（zip / tar / gz）、`archive::extract_op` 遇到 FIFO 時立即結束（報錯或略過），絕不永久卡住執行緒。

---

### [SEC-2-L1] `ConnectDialog` 變更連線目標未清空手動輸入之 `secret`

#### 位置
- `src/components/ConnectDialog.tsx:260-305, 341-346, 446-468, 534-558, 619-649`

#### 問題說明
1. 使用者在密碼欄手動輸入 `secret` 後，若修改 SFTP 的 `host`、`port`、`username`（lines 450, 458, 467）或 SMB 的 `server`、`share`、`username`（lines 537, 546, 556），`secret` 並不會被清空（目前僅在切換協定、`authMode` 或從 `~/.ssh/config` 選取時清空）。此時按 **Test** 或 **Connect** 會把前一個目標輸入的密碼直接送往新主機。
2. 當 SMB 未填密碼且無 Keychain 紀錄時，彈出 `askPw` 對話框；`submitPw()`（lines 341-346）呼叫了 `setSecret(pwInput)`，若接著進入 `shareList` 挑選視窗後使用者按取消（或 `smbShares` 失敗），剛輸入的密碼仍殘留在表單的 `secret` state 中。

#### 修改建議
- 在修改連線身分／目標欄位（SFTP: `host`、`port`、`username`；SMB: `server`、`username`，至於 `smb.share` 若使用者習慣先填帳號密碼再填 share 可斟酌保留，但 `server` 與 `username` 變更時務必 `setSecret("")`）。
- 將 `shareList` state 改為 `{ shares: string[]; password: string } | null`：讓 `mountSmb(password)` 把當次 `password` 帶入 `shareList`，點選 share 時呼叫 `doMount(shareList.password, share)`，並移除 `submitPw()` 中的 `setSecret(pwInput)`，使取消或失敗時不殘留密碼。

---

### [SEC-2-L2] 前後端相依套件安全通告修補

#### 位置
- `package.json`、`package-lock.json`
- `src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`

#### 問題說明與修改建議
1. **前端 `npm audit --omit=dev`**：
   - `katex`（`GHSA-238p-pmpm-9mq7`）：透過 **SEC-2-M2** 的 `"overrides": { "katex": "^0.19.1" }` 一併解決。
   - `i18next@24.2.3`（`GHSA-6jcc-5g8w-32mx`，Moderate）：`deepExtend` 存在原型鏈內建物件污染 DoS。先檢查最新修補版（如 `i18next@^26.3.4` 或 `^25.x` 是否有 backport，以及與 `react-i18next@^15` 的 peerDependencies 相容性）；若升級至修補版與現有 `src/i18n/index.ts` 完全相容且 `npx tsc -b && npm run build` 通過則直接升級，若涉及 breaking change 則記錄原因。
2. **後端 `cargo audit`**：
   - 目前 0 個 CVE，僅有 Linux 傳遞相依警告（`glib 0.18.5` `RUSTSEC-2024-0429` unsound 警告，以及 `keyring` Linux `sync-secret-service` 帶入的 `zbus_names 3.0.0` / `zvariant 4.2.0` unmaintained 警告）。可執行 `cargo tree -i glib --target x86_64-unknown-linux-gnu` 確認來源（通常鎖定於 `tauri` / `gtk-rs` 或 `drag` crate 上游），若上游尚無無痛升級路徑，在報告中註明即可，勿強行破壞相容性。

---

### [SEC-2-L3] GitHub Actions CI 權限收斂與第三方 Action SHA 釘選

#### 位置
- `.github/workflows/ci.yml:1-86`

#### 問題說明
1. Workflow 未宣告頂層 `permissions`，若儲存庫預設設定較寬，工作流中的 `GITHUB_TOKEN` 將擁有不必要的寫入權限。
2. 使用了浮動標籤的第三方 Action（`dtolnay/rust-toolchain@stable`、`Swatinem/rust-cache@v2`），且 `linux-deb` job 會在 `refs/tags/*` 上建置發行版 `.deb` 產物，若上游 Action 標籤遭供應鏈劫持可能影響建置環境與產物。

#### 修改建議
- 在 `.github/workflows/ci.yml` 頂層（`on:` 與 `jobs:` 之間）補上最小權限：
  ```yaml
  permissions:
    contents: read
  ```
- 將第三方 Action（至少 `dtolnay/rust-toolchain` 與 `Swatinem/rust-cache`，理想上連同官方 `actions/checkout`、`actions/setup-node`、`actions/upload-artifact`）釘選至明確的 40-char commit SHA 並在行尾附上版本註解（例如 `uses: Swatinem/rust-cache@<sha> # v2.x.x`）。

---

## 處理結果（2026-11-24，branch `security-review-2`）

各項皆先對照 `v1.4.0` 程式碼查證後才修改。

### SEC-2-M1：部分屬實，已修正

- **查證**：在 Ubuntu 26.04 容器以 LibreOffice 26.2.6 實測，命名為 `.doc` 的 HTML、SVG、Flat-ODF 確實分別由 `writer_web`、`draw`、`writer` 匯入器處理，副檔名白名單可被內容繞過，屬實。但 `<img>`、`<link>`、`<iframe>`、`draw:image`、`text:section-source`、SVG `<image>`、RTF `INCLUDEPICTURE` 七種外部參照在 `--headless --convert-to pdf` 下**皆未對本機監聽埠發出任何請求**（對照組請求有記錄到）。「0-click 外聯」在此版本未能重現；其他版本或 macOS 版未測。本機 FIFO 未擋屬實。
- **修改**（`commands/office.rs`、`platform/`）：
  - `check_office_header`：`.doc` 需為 OLE2 或 RTF，`.ppt`/`.xls` 需為 OLE2，其餘需為 zip 檔頭；`convert_input` 對本機檔先 `open_regular` 再讀檔頭，遠端／壓縮檔則檢查讀回的位元組。
  - macOS 以 `sandbox-exec` 啟動 `soffice`：禁止所有網路與 mDNSResponder（DNS），保留其餘 unix socket（LibreOffice 以此偵測既有執行個體）。Linux 沒有免特權且各桌面皆具備的對應機制，維持直接啟動。
  - profile 預寫 `registrymodifications.xcu`：`MacroSecurityLevel=3`、`BlockUntrustedRefererLinks=true`、HTTP(S) proxy 指向無人監聽的 `127.0.0.1:9`。容器實測這些設定被 LibreOffice 接受並保留，轉檔正常；因外聯本身未能重現，其阻擋效果未經實證。
- **未驗證**：這台 Mac 未安裝 LibreOffice，`sandbox-exec` 包住真實 `soffice` 的轉檔未實測（僅以 `nc` 驗證 sandbox 設定檔本身）。裝有 LibreOffice 時請跑 `cargo test -- --ignored libreoffice_converts_to_pdf`。
- **行為變更**：內容其實是 HTML 的 `.xls`（部分網站的「匯出 Excel」）與加密的 OOXML 不再送交 LibreOffice，Quick View 顯示錯誤訊息。

### SEC-2-M2：屬實，已修正

- `overrides: { "katex": "$katex" }`，`npm ls katex` 全部為 `0.19.1`。
- `md.render` 輸出經 `DOMPurify`（禁 `<style>`、`<form>`；保留 KaTeX 的 `style` 屬性與 `semantics`/`annotation`）。以 jsdom 比對：含待辦清單、KaTeX、程式碼高亮、表格、連結的文件淨化前後完全相同。
- Mermaid 改 `mermaid.render` → `<img src="data:image/svg+xml,…">`，並設 `htmlLabels: false`、`suppressErrorRendering: true`；失敗的圖表維持原始碼。以 headless Chrome 實測流程圖、循序圖與錯誤圖表顯示正常；**未在 WKWebView／WebKitGTK 內目視確認**。
- **行為變更**：Mermaid 標籤改為純 SVG 文字（不再支援標籤內 HTML／數學式），圖表內連結不可點。

### SEC-2-M3：屬實，已修正

- `AppState::resolve` 對 `Location::Archive` 要求絕對路徑；`archive_fs` 五處開檔改走 `open_regular`。
- `sync::collect_local` 只收一般檔案；`transfer::copy_file`、`same_contents` 改用 `open_regular`（複製到 FIFO 回報 `not a regular file`）。
- `archive::collect_tree`：資料夾內的 FIFO／socket／裝置檔略過（與解壓縮略過特殊項目對稱），直接選取則報錯；`write_zip`、`gz_file` 改用 `open_regular`；`extract_op` 於每個壓縮檔開頭檢查（涵蓋 zip／7z／tar／gz）。
- 報告未列但同類：`sftp::upload_file` 開啟本機來源亦改用 `open_regular`。
- 各處皆有 `mkfifo` 回歸測試。

### SEC-2-L1：屬實，已修正

- SFTP 的 host／port／username 與 SMB 的 server／username 變更時清空 `secret`（SMB share 變更保留）。
- `shareList` 改為 `{ shares, password }`，`submitPw` 不再寫入 `secret`。

### SEC-2-L2：屬實，已修正

- `i18next` `24.2.3 → 26.7.2`（26.0 的破壞性變更為移除 `initImmediate`、舊式 `interpolation.format`、`simplifyPluralSuffix`，專案皆未使用）；`npm audit --omit=dev` 為 0 個通告。
- `cargo audit`：0 個漏洞，6 個警告（`glib 0.18.5` unsound、`zbus`／`zvariant` 系列 unmaintained），皆為 Linux 傳遞相依且上游無修補版，未處理。

### SEC-2-L3：屬實，已修正

- 頂層 `permissions: contents: read`；五個 Action 釘選至目前 `v4`／`v2`／`stable` 所指的 commit（行為不變）。修改後的 workflow 尚未在 GitHub 上實際執行。

### 驗證

- macOS：`cargo test` 192 passed（2 ignored）、`cargo clippy --all-targets` 無警告、`npx tsc -b && npm run build` 通過。
- Linux（`yc-linux-dev` 容器）：`cargo test` 186 passed（1 ignored）；容器未裝 clippy。
