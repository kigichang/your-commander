# Your Commander Code Review 報告（安全性與多餘程式碼審查）

- **審查版本**：`v1.3.2`（branch `main`）
- **審查日期**：2026-11-19
- **審查範圍**：前端 React / TypeScript（`src/`、`index.html`、`public/`、`package.json`）、Tauri 設定與權限（`src-tauri/tauri*.conf.json`、`capabilities/`）、Rust 後端全模組（`src-tauri/src/`、`Cargo.toml`）
- **自動化檢查基線**：
  - `npx tsc --noEmit`（含 `strict`、`noUnusedLocals`、`noUnusedParameters`）：**0 errors**
  - `cargo clippy --all-targets`：**0 warnings**
  - `cargo test`：沙箱環境下 **158 passed / 2 ignored**（另 4 項 `media_server::tests` 因沙箱禁止 `127.0.0.1` socket 連線而未執行）

---

## 修正結果（2026-11-19，branch `review-fixes-3`）

本報告的每一項皆已對照程式碼查證；屬實者已修正，每個問題一個 commit（commit 訊息記錄問題與解法）。以下各表的「判定」是查證結果，「未採納」是查證後決定不改的部分與理由。

**驗證狀態**

- `cargo test`：**177 passed / 2 ignored**（原 158；新增 19 個回歸測試）。`cargo clippy --all-targets`：0 warnings。`npx tsc -b` 與 `npm run build`：通過。
- **未驗證的部分**：
  - 修正是在 macOS 上完成的，`#[cfg(target_os = "linux")]` 的程式碼（`platform/linux/*`、`media_server` 的正式路徑、`lib.rs` 的 Linux 分支）**沒有經過編譯**，只做了人工檢查。合併前請在 Linux 上跑一次 `cargo test`。
  - 沒有啟動 App 做人工操作測試。下列變更請實際操作確認：HTML 預覽載入同資料夾圖片／CSS（CSP 新增 `base-uri`）、影片／音訊預覽（`stream://` 改為 token）、連線對話框（已儲存密碼改由後端讀取）、Mermaid 圖表外觀。
  - SMB／SFTP 的變更沒有對真實伺服器測試。

### 高風險（High）

| 編號 | 判定 | Commit | 修正方式 | 未採納／備註 |
| :--- | :--- | :--- | :--- | :--- |
| SEC-H1 | 部分屬實 | `b8311c4` | Mermaid `secure` 清單加入 `themeCSS`／`fontFamily`／`altFontFamily`；`.markdown-body` 加 `contain: layout paint; isolation: isolate`；KaTeX 設 `maxSize`／`maxExpand`、明示 `trust: false`。 | 未改為 iframe 沙箱：Mermaid 需要 script 才能渲染，連結點擊也要經過 IPC。未加 DOMPurify：markdown-it 已 `html: false`，Mermaid strict 模式自帶淨化。 |
| SEC-H2 | 屬實 | `7cb67c9` | `ssh_saved_password`／`smb_saved_password` 改為只回傳布林的 `ssh_has_password`／`smb_has_password`；未輸入密碼時由後端（`sftp_connect`、`sftp_test`、SMB 掛載）自行讀 Keychain。對話框的 `secret` 只保存使用者輸入；切換協定／驗證模式／ssh config 主機時清空；`pick()` 加序號防競態；`remember` 未勾選不寫入 Keychain；`pwInput` 送出後清空。 | SMB 掛載時使用者「當場輸入」的密碼仍會隨 op 請求暫存於 store，job 結束即移除。 |
| SEC-H3 | 屬實 | `29b1895` | `stream://` 只服務經 `stream_url` 指令註冊、以 128-bit 隨機 token 命名的檔案（註冊表與 Linux media server 共用）；非 GET／HEAD 回 405；只開一般檔案；範圍運算改飽和運算；僅在 macOS 註冊，Linux CSP 移除 `stream:`。 | — |
| SEC-H4 | 屬實 | `dfba3a8` | `open_with` 的 `app` 必須是設定中的外部工具、偵測到的影片播放器，或系統對該檔案列出的 handler；Linux `.desktop` id 不得含路徑；PlantUML jar 必須是絕對路徑的一般檔案。 | `update_config` 存檔時不驗證路徑：設定頁每次按鍵即存檔，驗證會擋掉輸入中的值；`soffice`／`java` 在使用時本來就要求絕對路徑的一般檔案。macOS `open -a` 未加 `--`：路徑已強制為絕對路徑。 |
| SEC-H5 | 屬實 | `e3abc0d` | 伺服器必須是有 host、無帳密／query／fragment 的 http(s) URL；停用重導向（3xx 回報 Location 讓使用者改設定）；原始碼上限 1 MiB。 | 仍允許 `http://`：區網內的 PlantUML 伺服器是正常用法，位址由使用者自己設定。 |
| SEC-H6 | 部分屬實 | `4eadb85` | ZIP symlink 目標最多讀 4096 bytes；標頭大小加總改飽和加法；瀏覽／預覽 `.tar.gz` 解壓超過 8 GiB 即停止並提示改用解壓縮；試算表先檢查 ZIP 宣告的解壓總量（512 MiB）、最多 100 張工作表、儲存格文字截至 4096 字元；前端 docx／pptx 先檢查解壓總量（300 MiB），圖片解壓前先比對預算。 | 完整解壓縮不設總量上限：那是使用者明確下的指令，有進度、可取消，上限就是磁碟空間；設上限只會擋住正常的大型壓縮檔。 |
| SEC-H7 | 屬實（已實測 macOS 對唯讀檔 `setxattr` 回 EACCES） | `07f0190` | `Quarantine::mark` 被拒時暫時加上擁有者寫入權限再設定、設完還原；`mark_tree` 遇到擁有者無 rwx 的資料夾會暫時開啟再還原（涵蓋 zip／tar／7z／gz）；`copy_file` 改為先複製 xattr 再套用權限。 | — |
| SEC-H8 | 屬實 | `d4428e1` | `create_dir_inside` 的不存在尾段只接受一般名稱；`.zip`／`..zip`／`...zip` 改解壓到 `archive` 資料夾，目標以 lstat 檢查；`classify` 改以 ASCII 位元組比對；`copy_op` 以 (device, inode) 判斷同資料夾與資料夾複製到自身，`copy_file` 拒絕目的地就是來源檔的情況。 | — |
| SEC-H9 | 屬實 | `38bf5f2` | 後端將 `max_bytes` 限制在 256 MiB（前端最大請求值）；本機路徑必須是絕對路徑且為一般檔案（`safe_path::open_regular`）。 | 未使用 `O_NONBLOCK`：該旗標會留在 descriptor 上，而雲端佔位檔必須能在下載期間阻塞；改為開檔前後各檢查一次檔案類型。 |
| SEC-H10 | 屬實 | `1b10c3e` | 新增共用的 `platform::scrub_password`，macOS 掛載與列出分享的錯誤訊息、Linux 端皆使用。 | 未改用 `NetFSMountURLSync` 或 `mount volume … with password`：無法對真實伺服器驗證，不在沒有測試的情況下更動掛載主流程。 |

### 中風險（Medium）

| 編號 | 判定 | Commit | 修正方式 | 未採納／備註 |
| :--- | :--- | :--- | :--- | :--- |
| SEC-M1 | 屬實 | `59e97b3` | 報告列出的所有位置皆改用 `displayName()`；過濾範圍加入 U+200E、U+200F、U+061C 與 C0／C1 控制字元。 | — |
| SEC-M2 | 部分屬實 | `bf073f2` | CSP 加入 `object-src 'none'`、`form-action 'none'`、`base-uri`（僅 asset 協定）；`allow_asset_dir` 拒絕家目錄下的隱藏資料夾與 `~/Library`。 | `base-uri` 不能設 `'none'`：沙箱中的 HTML 預覽會繼承 App 的 CSP，並需要 `<base>` 指向檔案所在資料夾。授權仍維持整個 session（Tauri 的 scope 無法撤銷）。Linux 的 `http://127.0.0.1:*` 保留：media server 的 port 是執行時才決定的。 |
| SEC-M3 | 屬實 | `ac5c87b` | wildcard 改為線性 glob 比對；計數器寬度上限 32；JSON 縮排上限 64 層、格式化輸出超過 2 MiB 就改顯示原文；PDF canvas 單邊上限 8192 px；根節點加上 `ErrorBoundary`。 | 批次改名的 regex 是使用者自己輸入、作用在自己的檔名上，維持原樣。 |
| SEC-M4 | 屬實 | `b1fd210` | 所有 ffmpeg／ffprobe 呼叫加上 `-protocol_whitelist file`（已用本機 ffmpeg 實測四種呼叫形式）；symlink 檢查移到編碼開始前一刻；stderr 只保留前 64 KiB、其餘讀掉丟棄；`finish_repair` 檢查絕對路徑。 | 轉檔不設逾時（長影片本來就久）；暫存檔名不變（`finish_repair` 靠它辨識）。 |
| SEC-M5 | 屬實 | `e18f5bc` | 子程序以獨立 process group 啟動，逾時時整組 kill；暫存資料夾改用隨機名稱；沒有 cache 目錄時直接失敗而不退回共用的 /tmp；`office_to_pdf` 只接受 Quick View 的 Office 副檔名。 | 未預置 `registrymodifications.xcu`：這裡沒有 LibreOffice 可以驗證設定鍵值。 |
| SEC-M6 | 屬實 | `efffc06` | 驗證 `Host` 必須是 `127.0.0.1:<port>`（否則 403）；加上 `nosniff`；只開一般檔案；移除重複的 `Connection: close`。（`url_for` 的絕對路徑檢查已隨 SEC-H3 的共用註冊表修正。） | 不設寫入逾時：播放器暫停時本來就會停止讀取，連線數上限已經限制了閒置連線。 |
| SEC-M7 | 屬實 | `973fb06` | `extract_tar` 略過 FIFO、裝置節點與 hard link；壓縮檔以 `create_new` 建立一次後交給各 writer，失敗時只刪除自己建立的檔案；`.tar.gz` 呼叫 `finish()` 並回報錯誤；`.gz` 失敗會清除半成品。 | — |
| SEC-M8 | 部分屬實 | `5ac336d` | macOS：`ptbN` 必須是單一檔名、`ptbL` 不得含 `..` 或 NUL；`.DS_Store` 必須是 32 MiB 以內的一般檔案。Linux：`Path=` 含 `..` 則拒絕。`restore_one` 要求有檔名。 | B-tree 位移已有邊界檢查的 reader，64-bit 上不會溢位（此點不成立）。「檢查存在後 rename」的極短競態保留：要關閉需要 `renamex_np`／`renameat2`。 |
| SEC-M9 | 屬實 | `3b43420` | 暫存檔名加入行程內遞增的批次編號；本機 `mkdir`／`rename`／`batch_rename` 要求絕對路徑；批次項目必須有檔名。 | — |
| SEC-M10 | 部分屬實 | `0ce26e4` | 整個連線流程（撥號、金鑰交換、驗證、開 SFTP channel）加上 60 秒逾時；keepalive 每 30 秒、3 次無回應即斷線；host 不得為空或含空白／控制字元；金鑰檔必須是絕對路徑；`download_item` 把同名的 symlink 視為衝突，並使用 `create_dir_inside` 回傳的路徑。 | 「SFTP 請求沒有逾時」不成立：russh-sftp 預設每個請求 10 秒逾時。 |
| SEC-M11 | 部分屬實 | `342babc` | macOS：PTY slave 加 `FD_CLOEXEC`、提示緩衝只留最後 4 KiB、寫入密碼失敗時 kill 並回收子程序。Linux：使用者名稱含 `%` 時拒絕、空的 `XDG_RUNTIME_DIR` 退回預設路徑。兩平台的卸載都加上 60 秒逾時並要求絕對路徑。 | `get_info` 的 AppleScript 注入不成立：引號與反斜線已跳脫，字串內換行是合法的 AppleScript（已用 `osascript` 實測）。Linux 工具仍經由 PATH 尋找（那是使用者自己的環境）。 |
| SEC-M12 | 屬實 | `9436035` | `unescape` 改以位元組處理、最後才解 UTF-8（修正中文掛載點變亂碼與潛在 panic）；mtime、statfs 改飽和乘法，日期為 0 視為 1 日；內容搜尋只開一般檔案；`watch_dir` 註冊後若視窗已關閉則移除 watcher，並要求絕對路徑；`conflict()` 放入 sender 後檢查取消旗標。 | 本機搜尋不設深度上限：它不跟隨 symlink，深度受系統路徑長度限制。 |

### 低風險（Low）

| 編號 | 判定 | Commit | 修正方式 | 未採納／備註 |
| :--- | :--- | :--- | :--- | :--- |
| SEC-L1 | 屬實 | `b846ad4` | `open_url` 開啟的是解析後重新輸出的 URL。 | — |
| SEC-L2 | 不修正 | — | — | 鍵名碰撞需要 host 含 `@`，而 host／server 的檢查不允許；改變鍵名格式會讓既有的 Keychain 項目全部失效。zeroize 需要新增相依套件，而密碼在交給 russh／系統工具時仍會被複製。 |
| SEC-L3 | 屬實 | `7d89063` | 先包成 `CFType` 再 `downcast_into::<CFNumber>()`。 | — |
| SEC-L4 | 部分屬實 | `3daa924` | `keep_both_with` 編號用盡時改用時間戳檔名，不再回傳原路徑；權限改設在已開啟的檔案上（解壓與複製）；7z 的空檔與資料夾也檢查取消；SMB Test 不再對已含 port 的 server 再加 `:445`。 | 保留 setuid／setgid（同擁有者下不增加任何權限，與 `cp -p` 相同）；遠端為來源的 sync 不做 `real_path` 檢查（讀到的本來就是該伺服器自己的資料）；外部指令的路徑皆為絕對路徑，不需要 `--`。 |
| SEC-L5 | 屬實 | `39c5a4c` | Copy/Move：對側是 SFTP 時，手動輸入的路徑仍屬於該伺服器；Compress：檔名含 `/` 則拒絕；批次改名：結果含控制字元標為無效；SMB `start_in` 含 `..` 時忽略；退出磁碟的路徑比對加上邊界。 | 批次改名不拒絕 Bidi 字元：它們只可能來自既有檔名或使用者自己的遮罩，擋掉反而讓這些檔案無法改名。 |
| SEC-L6 | 部分修正 | `2bbaf6c` | TOML 解析失敗時先把原檔改名為 `config.toml.bad`；PlantUML 預覽圖不可拖曳；`font_scale` 限制在 0.5–3。 | 未改的項目與理由：SVG 以 `<img>` 載入，blob 或 data URL 都不會執行 script；docx-preview 已渲染到未掛載的元素、只在沙箱內顯示；PlantUML URL 已在使用處驗證（H5）；字型名稱含換行只會讓該 CSS 值無效；`icons.rs`／`fonts.rs`／`officeTools` 屬效能建議；全面改寫 `.lock().unwrap()` 沒有對應的實際失敗案例。 |

### 多餘／重複程式碼

| 項目 | 判定 | Commit | 處理 |
| :--- | :--- | :--- | :--- |
| A-1 Linux 上的 `stream://` | 屬實 | `29b1895` | 隨 SEC-H3 移除。 |
| A-2 `--font-heading` | 屬實 | `4727ba7` | 移除定義與設定處。 |
| A-3 Dialog 未讀欄位 | 屬實 | `e06b74a` | 移除 find／compress 的 `side` 與 settings 的 `section`。 |
| A-4 未使用的 IPC 欄位 | 屬實 | `6afabc0` | `FindEvent::Done` 移除 `error` 與 `count`。`Config.window_size` 保留（對應後端的 Config 結構）。 |
| A-5 7 處不可達的 `_` 分支 | 屬實，未修改 | — | 要消除必須讓 `ask_conflict_with` 回傳新的 4 變體 enum，牽動所有衝突處理點；這些分支目前都是安全的預設行為（略過）。建議獨立成一次重構。 |
| A-6 多餘的 `pub`／`export` | 大部分屬實 | `90c488f` | 清單中的項目改為私有；`AppState::sftp` 與 `archive::Meter` 實際上有其他模組使用，維持原樣。 |
| A-7 零星冗餘 | 屬實 | `ec1c973`（doc 註解在 `90c488f`） | 合併重複 import、移除 `let custom`、內聯 `files_equal`、修正過期註解。`connect_with` 的外層大括號與 `!src.is_symlink()` 保留（前者只是重新縮排上百行，後者是零成本的防護）。 |
| B-4 SMB 密碼遮罩重複 | 屬實 | `1b10c3e` | 隨 SEC-H10 抽成共用函式。 |
| B-5 迴圈內配置 1 MiB buffer | 屬實 | `537fb8a` | `write_zip` 與 `extract_entries` 的 buffer 移到迴圈外。 |
| B 其餘重構建議 | 未處理 | — | `on_main_thread`、`useDismissable`、`useTauriListen`、合併 `Meter`／`TarMeter`、共用 Tokio runtime、重複 UI 片段等皆為不改變行為的重構，且多數需要實際操作 UI 才能驗證，未在這一輪進行。 |
| C 建置產物 | — | — | 僅為說明，無需修改。 |

另有一個後續 commit `e1d3ed9`（為 SEC-H6 新增的函式回傳型別取別名，消除 clippy 警告）。

---

## 總覽與整體評價

### 值得肯定的既有安全實作

本專案已經具備相當成熟的安全設計意識，許多常見陷阱皆已有專屬防護與單元測試：
- **HTML 與 `.docx` 預覽沙箱**：`HtmlPreview` 使用 `<iframe sandbox="" srcDoc={...}>`（無 script / same-origin / form / popup），並正確處理 BOM 與 HTML 空白字元後置入 `<meta http-equiv="Content-Security-Policy">`；`DocxPreview` 關閉 `renderAltChunks` 並將輸出導入 `HtmlPreview`。
- **導航防護與基礎 CSP**：`src-tauri/src/lib.rs` 透過 `navigation_guard()` 阻擋非 `tauri://localhost` / `about:srcdoc` / `about:blank` 導航；`default-src 'self'` 封鎖內嵌 script 與 `eval`。
- **SSH 主機金鑰驗證**：`src-tauri/src/fs/sftp.rs` 嚴格檢查 `~/.ssh/known_hosts`，防範不同金鑰型別降級 MITM，並僅寫入使用者明確確認的 SHA-256 指紋。
- **符號連結逃逸防護**：`src-tauri/src/ops/safe_path.rs` 的 `resolve_target` / `symlink_inside` / `O_NOFOLLOW` 與 macOS `smbutil` 透過 PTY 餵入密碼（避免出現在 `ps` argv）均屬高水準防護。
- **PlantUML 本機沙箱**：`render_via_jar` 加上 `-DPLANTUML_SECURITY_PROFILE=SANDBOX` 防止 `!include` 讀取本機檔案，前端改用 `<img src="data:image/svg+xml,...">` 而非內嵌 SVG。
- **程式碼整潔度高**：無未使用的 npm 套件、無未使用的 Cargo crate、無未註冊或未呼叫的 Tauri command / event、無未使用的 CSS class 與 i18n key。

### 問題統計摘要

| 類別 | 嚴重度 | 數量 | 重點說明 |
| :--- | :---: | :---: | :--- |
| 安全性（Security） | **High** | 10 | Markdown/Mermaid 主 DOM 渲染面、Keychain 憑證外洩與跨連線殘留、`stream://` 未限權、任意執行檔執行、SSRF、解壓炸彈、Quarantine 繞過、路徑安全與 0-byte 截斷、無上限 `read_preview`、`osascript` 錯誤未遮罩密碼 |
| 安全性（Security） | **Medium** | 12 | Bidi 偽裝漏網、CSP/asset 範圍強化、前端 ReDoS/OOM、`ffmpeg`/LibreOffice/子進程群組清理、`media_server` 加固、tar FIFO 與 `TarGz` 錯誤吞沒、Trash 還原路徑驗證、`batch_rename` 並發碰撞、SFTP 逾時、平台指令與 `mountinfo` UTF-8 panic |
| 安全性（Security） | **Low** | 6 | URL 正規化、Keychain key 碰撞、FFI TypeID 檢查、檔案操作邊界細節、前端表單驗證、全域鎖中毒與其他縱深防禦 |
| 多餘 / 重複程式碼 | **Cleanup** | 25+ 項 | 少數明確未使用的欄位 / CSS 變數 / Linux 無效註冊，以及跨模組重複程式碼與效能冗餘 |

---

## 一、安全性問題（Security Findings）

> **威脅模型說明**：在 Tauri 桌面應用中，主 WebView（`tauri://localhost`）直接擁有呼叫所有 IPC 指令的權限（包含檔案讀寫、Keychain 讀取、外部程式啟動、設定修改）。因此任何主 WebView 層級的 XSS 皆等同於 **本機任意程式碼執行（RCE）與憑證竊取**；同時，Quick View（F3）會在游標滑過檔案時自動觸發預覽與背景工具，屬 **0-click 攻擊面**，需特別嚴格防護。

### 高風險（High）

#### [SEC-H1] Markdown / Mermaid 直接渲染於主 WebView DOM（XSS 攻擊面與 0-Click 全視窗 UI Redressing）
- **位置**：
  - `src/components/MarkdownPreview.tsx:15-34, 99-109, 136-164, 183-192`
  - `src-tauri/tauri.conf.json:29`（`style-src` 允許 `'unsafe-inline'`）
  - `src/styles/base.css:911-942`
- **問題說明**：
  1. 不同於 `HtmlPreview` 與 `DocxPreview` 使用 `<iframe sandbox="">` 隔離，`MarkdownPreview` 將 `markdown-it` + `highlight.js` + `katex` + `mermaid` 產出的 HTML/SVG 直接透過 `dangerouslySetInnerHTML` 與 `mermaid.run()` 寫入具完整 Tauri IPC 權限的主視窗 DOM，且未經 DOMPurify 二次淨化。
  2. 驗證 `mermaid@12.2.0` 內部實作（`mermaid.core.mjs:1319-1396`）：在 `securityLevel: "strict"` 下，`renderDiagram()` 會先將 SVG 掛入主文件 DOM 並執行 `diag.renderer.draw(...)`，之後才於 `serializeSvg()` 呼叫 DOMPurify（且允許 `foreignobject`）。雖然目前 CSP `default-src 'self'` 阻擋內嵌 `<script>` 與事件處理器，但一旦出現 DOM clobbering 或 `<base>` 注入（參見 SEC-M2）等繞過，即可直接呼叫 `window.__TAURI_INTERNALS__.invoke` 達成 RCE。
  3. **0-Click 全視窗 UI 欺騙**：`mermaid` 的 `secure` 設定未鎖定 `themeCSS` 與 `fontFamily`，且主視窗 CSP 允許 `style-src 'unsafe-inline'`，加上 `.quickview` / `.qv-body` 缺乏 `contain: layout paint` 隔離。惡意 `.md` 透過 `%%{init: {"themeCSS": "..."}}%%` 可將元素設為 `position: fixed; inset: 0; z-index: 999999`，只要游標停在檔案上即可覆蓋整個應用視窗偽造介面。
  4. `md.use(katexFn)` 未傳入限制參數，KaTeX 預設 `maxSize: Infinity`（無上限 `\rule` 版面破壞）。
- **建議修復**：
  - 將整個 Markdown 預覽（或至少 Mermaid 輸出）改渲染於 `<iframe sandbox="" srcDoc={...}>` 中（沿用 `HtmlPreview` 的沙箱模式與 `baseDir={null}`），或將 Mermaid 渲染結果轉為 `<img src="data:image/svg+xml,...">`。
  - 傳入 KaTeX 安全參數：`md.use(katexFn, { trust: false, maxSize: 10, maxExpand: 200 })`。
  - 若短期仍保留於主 DOM，須補上 DOMPurify、將 `themeCSS`/`fontFamily`/`altFontFamily` 加入 Mermaid `secure` 清單，並在 `.qv-body` / `.markdown-body` 加上 `contain: layout paint; isolation: isolate;`。

#### [SEC-H2] Keychain 明文密碼暴露於 WebView IPC，且 `ConnectDialog` 存在跨主機憑證外洩與狀態競態
- **位置**：
  - `src-tauri/src/commands/config.rs:108-138`、`src-tauri/src/commands/smb.rs:90-117`
  - `src/components/ConnectDialog.tsx:108-126, 148-169, 262-270, 316-320, 364, 378-391, 416-441, 502-521`
  - `src/store/ops.ts:19-38, 151`
- **問題說明**：
  1. **切換目標不清除已載入密碼**：點選已儲存連線（`pick()`）會從 Keychain 載入明文密碼到 `secret` state；但使用者之後修改 `host`/`server`/`username`、切換協定（`SFTP ↔ SMB`，line 364）、切換驗證模式（line 441）或改選 `~/.ssh/config` 主機（lines 378-391）時，**均未清除 `secret`**。按下 Test 或 Connect 會把前一台伺服器的密碼送到新主機。
  2. **非同步競態（Race Condition）**：`pick()`（lines 108-126）呼叫 `sshSavedPassword` / `smbSavedPassword` 時未檢查請求是否已過期；快速切換連線或點擊 `+ New` 時，前一個連線的密碼會覆寫到新表單中。
  3. **無視 `remember: false` 仍寫入 Keychain**：`save()`（lines 156-166）未檢查 `ssh.remember` / `smb.remember`，取消勾選「記住」後按儲存仍會把密碼寫進系統 Keychain。
  4. **明文密碼暴露於 WebView 與全域 Zustand store**：`ssh_saved_password` / `smb_saved_password` 直接回傳明文密碼給前端（一旦前端遭 XSS 即可配合 `get_config` 一次匯出所有伺服器密碼）；SMB 明文密碼也會被放進全域 `useOps.jobs[].request.password`，且 `submitPw()` 後未清空 `pwInput`。
- **建議修復**：
  - **架構面**：移除直接回傳明文密碼給前端的 IPC；改回傳 `hasSavedPassword: boolean`，並在 `sftp_connect`、`sftp_test`、`smb_shares`、`smbMount` 加上 `useSavedPassword` 旗標，由 Rust 後端直接向 Keychain 取密連線。
  - 在任何目標欄位（`protocol`、`authMode`、`host`/`server`/`port`/`share`/`username`、`ssh/config` 選擇）變更時立即清空 `secret`，為 `pick()` 加上遞增 `requestId` 過期檢查。
  - `save()` 僅在 `remember === true` 時寫入 Keychain，並在 `submitPw()` 立即清空 `pwInput`。

#### [SEC-H3] `stream://` 自訂協定未限制存取範圍（可讀本機任意檔案、FIFO 阻塞、Linux 誤註冊）
- **位置**：
  - `src-tauri/src/stream_protocol.rs:84-171`
  - `src-tauri/src/lib.rs:115-117`
  - `src-tauri/tauri.linux.conf.json:28`
- **問題說明**：
  1. 專案刻意將 `asset://` 預設 `scope` 設為 `[]` 並於 `asset_dir_with_home` 封鎖 `/`、`/Users`、`$HOME` 等目錄；但 `stream_protocol::build_response` 直接將 URL path 解碼後 `File::open(&path)`，**沒有任何 token、目錄範圍、絕對路徑或影音副檔名限制**，任意本機可讀檔案（含 `~/.ssh/id_rsa`、`config.toml`、相對路徑）皆可被請求。
  2. 只檢查 `method == Method::HEAD`，`POST`/`PUT` 等方法皆當作 `GET` 回傳內容。
  3. 先呼叫 `File::open(&path)`（line 109）才檢查 `meta.is_file()`（line 115）：遇到 FIFO（named pipe）會永久阻塞 Tokio `spawn_blocking` 執行緒。
  4. Linux 實際走 `media_server.rs`，但 `lib.rs:115-117` 仍在 Linux 上註冊了 `stream://`（且 `tauri.linux.conf.json:28` 仍保留 `stream:` 於 `media-src`）。
  5. `stream_protocol.rs:146-148`：`start + CHUNK_MAX - 1` 與 `end + 1 - start` 在極大檔案長度下有溢位風險。
- **建議修復**：
  - 僅在 `#[cfg(target_os = "macos")]` 註冊 `stream://`，並自 `tauri.linux.conf.json` 移除 `stream:` 與 `http://stream.localhost`。
  - 改採與 `media_server.rs` 相同的 **128-bit 隨機 token 註冊機制**（僅允許經 IPC 顯式註冊的檔案，並驗證 `require_absolute` 與一般檔案）。
  - 拒絕 `GET`/`HEAD` 以外的方法（回傳 405），使用 `O_NONBLOCK` 開檔後 `fstat` 確認 `is_file()`，並改用 `saturating_add` / `end - start + 1`。

#### [SEC-H4] `open_with` 任意執行檔執行 / `.desktop` 路徑遍歷，與 `update_config` 未驗證設定內可執行路徑
- **位置**：
  - `src-tauri/src/commands/system.rs:363-369`
  - `src-tauri/src/platform/linux/mod.rs:269-274, 315-341`
  - `src-tauri/src/platform/macos/mod.rs:184-197`
  - `src-tauri/src/commands/config.rs:25-68`
  - `src-tauri/src/commands/plantuml.rs:172`、`src-tauri/src/commands/office.rs:398-403`
- **問題說明**：
  1. `open_with(app, paths)` 不驗證 `app` 是否屬於系統回傳的 `apps_for` 或使用者設定的 `external_tools`。在 Linux 上，任何不以 `.desktop` 結尾的 `app` 皆直接進入 `Command::new(app).args(paths).spawn()`（例如 `app = "/bin/sh"`, `paths = ["/tmp/p.sh"]`），構成直接 RCE 原語；而以 `.desktop` 結尾時，可傳入任意絕對路徑（如 `/tmp/evil.desktop`）或透過相對路徑 `../../../tmp/evil.desktop` 逃出 `find_desktop_file` 的 `applications_dirs()`。
  2. `update_config` 接受任意 JSON patch 寫入 `office_soffice`、`plantuml_java`、`plantuml_jar` 與 `external_tools`，缺乏路徑合法性檢驗；且 `commands/plantuml.rs:172` 對 `jar` 只檢查 `.exists()`，未如 `find_java`（line 92）要求 `is_absolute() && is_file()`。
- **建議修復**：
  - 限制 `open_with` 的 `app` 必須對應系統已知 handler（`apps_for`）或已設定的 `external_tools`；Linux `find_desktop_file` 拒絕未通過 `is_listing_name` 的 id，且 `.desktop` 絕對路徑經 `canonicalize` 後必須位於 XDG `applications_dirs()` 內；macOS `open -a` 在 `paths` 前加上 `--`。
  - `plantuml.rs:172` 要求 `jar` 為絕對路徑的一般檔案（且副檔名為 `.jar`）；在 `merge_patch` 驗證可執行檔設定為空字串或既有絕對路徑。

#### [SEC-H5] `render_plantuml` 的 SSRF、明文 HTTP、未限重導向與無上限輸入
- **位置**：`src-tauri/src/commands/plantuml.rs:51-80, 225-263`
- **問題說明**：
  1. `render_via_server` 直接拼接 `format!("{}/svg/{}", server.trim_end_matches('/'), encoded)` 並用 `ureq::get(&url)` 呼叫，未驗證 `server` 的協定（允許明文 `http://` 外洩圖表內容與遭 MITM 竄改）或禁止 query/fragment/userinfo 注入。
  2. `ureq` 2.x 預設自動跟隨最多 5 次 HTTP 重導向，允許從公網伺服器重導向至內網 / `127.0.0.1` / `169.254.169.254`（SSRF）或從 HTTPS 降級為 HTTP。
  3. `render_plantuml(code: String)` 未限制 `code` 長度，超大字串會直接壓縮或另開執行緒寫入 `java` stdin。
- **建議修復**：
  - 以 `tauri::Url::parse` 解析 `server`，僅允許 `https://`（或僅對 `localhost` / `127.0.0.1` 允許 `http://`），拒絕 userinfo/query/fragment。
  - 建立 `ureq::AgentBuilder::new().redirects(0).build()` 關閉重導向。
  - 限制 `code.len()` 上限（例如 1 MiB）。

#### [SEC-H6] 壓縮檔與 Office 文件解壓炸彈（OOM 與磁碟耗盡）
- **位置**：
  - `src-tauri/src/ops/archive.rs:340-356, 386-388, 416-426, 439, 515-520, 548, 568-599`
  - `src-tauri/src/fs/archive_fs.rs:89-111, 134-135, 160-180, 268-290, 411, 436, 444-457, 511`
  - `src-tauri/src/commands/office.rs:133-251`
  - `src/components/OfficePreview.tsx:47, 141-146, 167, 215-218`
- **問題說明**：
  1. **ZIP Symlink 記憶體炸彈（`ops/archive.rs:416-426`）**：`entry.read_to_string(&mut target)` 沒有任何長度上限（Unix `PATH_MAX` 至多 4096 bytes），極小壓縮檔即可透過單一 symlink 條目解壓出數 GB 字串直接 OOM。
  2. **全格式解壓無磁碟寫入上限與 `u64` 總和溢位**：`ops/archive.rs`（zip/7z/tar/tar.gz/gz）與 `fs/archive_fs.rs`（`extract_entries`）均無總解壓位元組或壓縮比限制；`extract_zip:386-388`、`extract_7z:515-520`、`archive_fs:411` 對不受信任標頭大小直接 `.sum()` / `+=` 可能溢位；`archive_fs::scan` 雖限條目數（200 萬）與名稱總長（256 MiB），但未限 `.tar.gz` 跳過的解壓 payload 大小，單一 100 GB 零位元組條目會讓瀏覽卡死。
  3. **Office 試算表與 PPTX/DOCX OOM**：
     - `commands/office.rs` 僅限制壓縮前檔案大小（30 MiB），但 `calamine` 開啟 XLSX/ODS 時會將 `sharedStrings.xml` / `content.xml` 完整讀入記憶體；ODS 走 `worksheet_range`（未經 `Window`）；`Window` 未限制工作表數量與單一儲存格字串長度。
     - 前端 `OfficePreview.tsx`（`PptxPreview` 與 `DocxPreview`）使用 `JSZip` 解壓時未先檢查解壓後大小（`PptxPreview:215-218` 在整張圖解壓為 base64 後才扣除 `budget`，`readXml` 則完全無上限）。
- **建議修復**：
  - `ops/archive.rs:421` 改用 `entry.by_ref().take(4097).read_to_string(&mut target)` 並拒絕超過 4096 bytes 的目標；所有標頭加總改用 `saturating_add`。
  - 在 `Meter` / `archive_fs` 的解壓流程加入解壓位元組上限（瀏覽 `scan`/`read_entry` 嚴格限制如 512 MiB，解壓提供合理上限或壓縮比警示，且 `extract_entries` tar 分支改為可取消的分塊迴圈）。
  - `commands/office.rs` 先檢查 ZIP 內未壓縮總量再交給 `calamine`，並限制工作表數量（如 100）與儲存格字串長度（如 4 KiB）；前端 `PptxPreview` / `DocxPreview` 解壓前先驗證未壓縮大小或改由後端限流讀取。

#### [SEC-H7] macOS Gatekeeper Quarantine 繞過（唯讀檔案先設權限才寫 `com.apple.quarantine`）
- **位置**：
  - `src-tauri/src/ops/archive.rs:361, 441, 450-456, 472-493`
  - `src-tauri/src/ops/transfer.rs:245-253`
- **問題說明**：
  1. `extract_op` 在整個解壓結束後才呼叫 `quarantine.mark_tree(&target)`（line 361），而此時 `extract_zip`（`apply_mode`）與 `extract_tar`（`unpack_in`）已套用壓縮檔內的權限位元。若惡意下載的 `.app` 或腳本在壓縮檔內標記為唯讀（`0o555` / `0o444`），macOS 的 `setxattr()` 會因 `EACCES` 失敗，而 `Quarantine::mark` 以 `let _ = xattr::set(...)` 靜默忽略錯誤；若是 `0o111` / `0o000` 目錄，`mark_tree` 更無法 `read_dir` 進入，導致解壓出來的執行檔**完全沒有 `com.apple.quarantine` 屬性而繞過 Gatekeeper**（對比之下，`fs/archive_fs.rs:442, 458` 正確地先標記才 `apply_mode`）。
  2. 同樣的問題也出現在 `ops/transfer.rs:247-252`：`copy_file` 先 `set_permissions(&dest, meta.permissions())` 後才 `copy_xattrs(src, &dest)`，複製唯讀受隔離檔案時會靜默遺失 `com.apple.quarantine` 與 Finder tags。
- **建議修復**：
  - 在 `extract_zip`、`extract_7z`、`Gz` 與 `transfer::copy_file` 中，一律在寫入權限之前（優先透過已開啟 fd 的 `xattr::FileExt::set_xattr`）先寫入 xattr / quarantine。
  - 在 `Quarantine::mark_tree` 與 `extract_tar` 中，若因權限不足無法 `setxattr` 或 `read_dir`，針對擁有者項目暫時加上 `0o200`（或目錄 `0o700`），寫完 quarantine 後再還原原本權限。

#### [SEC-H8] 檔案操作路徑安全與資料毀損缺陷（`create_dir_inside`、空/點號檔名解壓逃逸、`copy_op` 覆寫來源歸零）
- **位置**：
  - `src-tauri/src/ops/safe_path.rs:89-110`（`create_dir_inside`）
  - `src-tauri/src/ops/archive.rs:47-53, 308-315, 341-342`（`classify` / `extract_op`）
  - `src-tauri/src/ops/transfer.rs:62-70, 220-224`（`copy_op` / `copy_file`）
- **問題說明**：
  1. **`create_dir_inside` 先建目錄才驗證**：若傳入路徑在尚不存在的尾端包含 `..`（例如 `<root>/missing/../../outside`），`existing` 會回溯到 `<root>` 並通過前檢，接著 `std::fs::create_dir_all(dir)`（line 102）就會**在 `root` 外部實際建立目錄**，之後 line 106 才報錯。此函式作為核心安全原語（且 `extract_7z` 直接依賴它）應自帶完整防護。
  2. **空或點號壓縮檔名逃出 `dest_dir`**：檔案名為 `.zip` 或 `.tar.gz` 時，`classify` 回傳 `stem = ""`，`dest_dir.join("")` 等於 `dest_dir` 本身；因 `dest_dir.exists()` 為真，`keep_both_path(&dest_dir)` 會計算出 `<dest_dir 上層>/<dest_dir 名稱> 2`，將檔案**解壓到目標資料夾之外的兄弟目錄**。`..zip`（`stem = "."`）與 `...zip`（`stem = ".."`）亦同（且 `.gz` 分支 `target.join(stem)` 會寫出 `target`）。另 line 312 僅查 `target.exists()` 漏檢 dangling symlink。
  3. **`copy_op` 來源檔案歸零（0-byte truncation）與無限遞迴**：`copy_op` 在 move 分支（line 47）使用正規化的 `is_inside`，但 copy 分支（lines 62, 67）卻直接比較未正規化的 `dest == *src` 與 `dest.starts_with(src)`。若 `dest_dir` 經由 symlink 指向同一資料夾，或在預設大小寫不敏感的 macOS APFS 上僅大小寫不同，覆寫時 `replace_file_nofollow(&dest)` 會先將檔案 truncate 為 0 bytes，**直接毀損來源檔案**；若目標資料夾經由 symlink 指向來源資料夾內部，則會無限遞迴複製直到塞滿磁碟。
- **建議修復**：
  - `create_dir_inside` 應在回溯時收集尾端 component，拒絕任何非 `Component::Normal` 元件，再從 `canon` 逐層 `create_dir` 建立。
  - `extract_op` 對 `stem` 檢查 `is_listing_name(stem)`（不合法時退回安全名稱如 `"archive"`），`classify` 改用 ASCII 後綴比對避免 Unicode 大小寫長度差異，line 312 改為 `target.symlink_metadata().is_ok()`。
  - `copy_op` 改用 `is_inside(src, &dest_dir)` 與正規化路徑比對目錄自含關係；`copy_file` 覆寫前另以 `(dev, ino)` 確認 `src` 與 `dest` 非同一檔案。

#### [SEC-H9] `read_preview` 未在後端限制 `max_bytes`、未驗絕對路徑且可能遭 FIFO 阻塞
- **位置**：
  - `src-tauri/src/commands/preview.rs:68-102`
  - `src-tauri/src/fs/sftp.rs:582-604`、`src-tauri/src/fs/archive_fs.rs:252-292`
- **問題說明**：
  - `read_preview` 直接信任前端傳入的 `max_bytes: u64`，後端無任何上限，且本機分支未呼叫 `require_absolute(&path)`、未驗證 `is_file()`。呼叫端（或遭入侵 WebView）傳入超大 `max_bytes` 讀取 `/dev/zero` 或大檔會直接 OOM；開啟 FIFO 則永久卡死工作執行緒。
- **建議修復**：
  - 後端以常數硬限 `max_bytes = max_bytes.min(PREVIEW_MAX)`（如 64 MiB，或依用途設限），本機路徑加上 `require_absolute(&path)?` 並以 `O_NONBLOCK` + `fstat().is_file()` 防範 FIFO。

#### [SEC-H10] macOS `osascript` 掛載 SMB 錯誤訊息未遮罩密碼
- **位置**：`src-tauri/src/platform/macos/smb.rs:58-98`
- **問題說明**：
  - `mount()` 將密碼編碼進 `smb://user:pass@server/share` 後寫入 `osascript -` 的 stdin，但失敗時（lines 90-98）**直接回傳原始 `stderr` 給 UI**，並未像同檔 `list_shares`（lines 141-145）或 `linux/smb.rs:77-84` 一樣將明文與 URL-encoded 密碼替換為 `•••`。此外把密碼嵌在 URL 中經過 AppleEvent / NetFS 也有被系統日誌記錄的風險。
- **建議修復**：
  - 抽取共用 `scrub_password()` 並於 `macos/smb.rs::mount` 錯誤路徑強制呼叫。
  - 建議改用 macOS `NetFSMountURLSync` C API（將帳號密碼以獨立參數傳遞，不經 `osascript` 也不進 URL），或至少改用 AppleScript 的 `mount volume "smb://server/share" as user name "..." with password "..."` 語法。

---

### 中風險（Medium）

#### [SEC-M1] Bidi / RTLO（`U+202E`）檔名偽裝防護不完整
- **位置**：
  - `src/lib/format.ts:31-36`（僅用於 `FileTable.tsx` 與 `Dialogs.tsx:64`）
  - **漏用處**：`src/components/QuickView.tsx:704, 840, 890`（緊鄰 `Open ↵` 按鈕）、`src/components/Dialogs.tsx:317, 477-480`（`RepairVideoDialog` 與 `ConflictDialog` 覆寫確認）、`FindDialog.tsx:253-256`、`SyncDialog.tsx:182, 198`、`MultiRenameDialog.tsx:179, 182`、`CompareDialog.tsx:196-201`、`Panel.tsx:96, 247-268`、`OpsUI.tsx:42, 54`。
- **問題說明**：壓縮檔或遠端目錄中的惡意執行檔可藉 `U+202E` 偽裝成 `.pdf` / `.txt`；雖然檔案列表有遮蔽，但使用者在 Quick View（旁有「開啟」按鈕）或覆寫衝突對話框看到的仍是未過濾的偽裝名稱。另 `BIDI_CONTROLS` 亦遺漏 `U+200E`、`U+200F`、`U+061C` 與控制字元。
- **建議修復**：在所有顯示檔名／路徑的元件全面套用 `displayName()`，並擴充 `BIDI_CONTROLS` 涵蓋 `U+200E`、`U+200F`、`U+061C` 與 C0/C1 控制字元。

#### [SEC-M2] CSP 指令缺口與 `allow_asset_dir` 永久授權／未封鎖敏感家目錄子資料夾
- **位置**：
  - `src-tauri/tauri.conf.json:27-39`、`src-tauri/tauri.linux.conf.json:28`
  - `src-tauri/src/commands/preview.rs:127-161`
  - `src/components/HtmlPreview.tsx:76-86`、`src/components/QuickView.tsx:360`
- **問題說明**：
  1. `tauri.conf.json` 未設定 `base-uri`、`form-action`（兩者皆**不繼承** `default-src`）與 `object-src: ["'none'"]`。缺少 `base-uri 'none'` 使 HTML 注入可藉 `<base href>` 劫持 Vite 動態載入的 JS chunk。
  2. `asset_dir_with_home` 雖阻擋 `/`、`/Users`、`$HOME`，但**允許 `~/.ssh`、`~/.gnupg`、`~/.aws`、`~/.config`、`~/Library`** 等敏感目錄；且授權為遞迴、整個 session 永不撤銷，加上 `QuickView` 的 `htmlView` 為 sticky 狀態，瀏覽過 HTML 檔案就會逐步擴大 `asset://` 範圍。
  3. Linux `tauri.linux.conf.json:28` 的 `media-src` 允許 `http://127.0.0.1:*`（全本機 port）。
- **建議修復**：
  - CSP 補上 `"base-uri": ["'none'"]`、`"form-action": ["'none'"]`、`"object-src": ["'none'"]`（並驗證 `worker-src` 是否可移除 `blob:`）。
  - `asset_dir_with_home` 明確拒絕 `$HOME` 下常見敏感目錄（`.ssh`、`.gnupg`、`.aws`、`.kube`、`.config`、`Library/Keychains` 等），並於切換目錄時重置 `htmlView` 或改採可撤銷之單目錄授權機制。

#### [SEC-M3] 前端主執行緒 ReDoS 與未上限配置崩潰（DoS），且缺乏 ErrorBoundary
- **位置**：
  - `src/lib/wildcard.ts:10-21`：未合併連續 `*`，如 `******z` 對長檔名觸發指數級正規回溯（每次按鍵同步執行）。
  - `src/lib/renameMask.ts:36-41, 64-85`：`[C:99999999]` 觸發 `padStart(99999999)` 直接拋 `RangeError` 讓整個 App 白畫面；勾選 regex 時於主執行緒同步執行任意使用者正規表示式。
  - `src/lib/json.ts:5-54` + `QuickView.tsx:643` + `JsonPreview.tsx:15`：超過 256 KB 的 `.json` 略過 `isJson` 驗證，深層巢狀 `[` 導致 `indent.repeat(depth)` 產生平方級（數十 GB）字串與 DOM 節點 OOM。
  - `src/components/PdfPreview.tsx:59-67`：`scale` 與 `canvas.width/height` 無上限，窄寬超高 PDF 可撐爆 GPU/WebView 記憶體。
  - `src/App.tsx` / `src/main.tsx` 無 React ErrorBoundary。
- **建議修復**：
  - `wildcard.ts` 改為線性雙指標 glob 比對；`renameMask.ts` 限制 pad 寬度上限（如 32）並對 preview 去抖／限時；`prettyJson` 與 `JsonPreview` 限制縮排深度（如 64）與總輸出長度；`PdfPreview` 限制 `scale` 與 canvas 最大邊長（如 4096–8192 px）；於根節點補上 React ErrorBoundary。

#### [SEC-M4] `ffmpeg` 欠缺 `-protocol_whitelist`、`repair_video` TOCTOU 與無上限記憶體/時間
- **位置**：
  - `src-tauri/src/commands/preview.rs:25-48`（`video_poster`）
  - `src-tauri/src/commands/system.rs:174-289, 294-347`（`probe_duration`、`repair_video`、`run_with_progress`、`finish_repair`）
- **問題說明**：
  1. `video_poster`、`probe_duration` 與 `repair_video` 皆對未受信任檔案呼叫 `ffmpeg -i`，未帶 `-protocol_whitelist file`。
  2. `repair_video` 使用可預測暫存檔名 `.{stem}_fix.part.mp4`：於 line 216 呼叫 `remove_if_link` 後，接著執行最長 30 秒的 `probe_duration`（line 217），才以 `ffmpeg -y` 寫入（會跟隨 symlink），在共享目錄具明顯 TOCTOU 視窗。
  3. `run_with_progress`（lines 273, 281）以無上限 `read_to_end` 讀取 `stderr`、無上限 `read_until` 讀取 `stdout`，且無逾時或取消機制；損毀影片噴出大量錯誤訊息會導致 OOM 或永久卡住。
  4. `finish_repair` 未呼叫 `require_absolute(&original)`。
- **建議修復**：為所有 `ffmpeg` 呼叫加上 `-protocol_whitelist file`；將轉檔輸出寫入 `0o700` 隨機暫存目錄或 `O_CREAT|O_EXCL|O_NOFOLLOW` 唯一檔名；限制 `run_with_progress` 的 `stderr`/行長度並加入取消與逾時；`finish_repair` 補上 `require_absolute(&original)?`。

#### [SEC-M5] LibreOffice 無頭轉檔安全與 `proc::wait_bounded` 子進程群組遺漏
- **位置**：
  - `src-tauri/src/commands/office.rs:286-340, 353, 391-444`
  - `src-tauri/src/proc.rs:28-65`
- **問題說明**：
  1. `Scratch::new()` 使用可預測名稱 `/tmp/yc-office-<pid>-<counter>`（多人 Linux 主機易遭預佔 DoS）；`profile_dir()` 當 `dirs::cache_dir()` 為 `None` 時退回共享 `/tmp/your-commander/lo-profile`（可被預植惡意 profile）。
  2. `office_to_pdf` 未驗證本機檔案副檔名白名單（可傳入 `.html`/`.svg` 等），且 headless LibreOffice 轉 PDF 時可能載入文件內外部連結。
  3. `proc::wait_bounded` 逾時僅 `child.kill()` 直接子進程：Linux 上 `/usr/bin/soffice` 為 wrapper script，逾時後孫進程 `soffice.bin` 仍存活並鎖住 `lo-profile`，同時持續佔住 pipe 寫端導致兩個 `drain` 執行緒永久洩漏。
- **建議修復**：
  - `Scratch` 與 `lo-profile` 一律置於私有 `0o700` 快取目錄並使用隨機亂數名稱；`office_to_pdf` 驗證副檔名白名單；預置 `registrymodifications.xcu` 關閉巨集與外部連結更新。
  - `proc::wait_bounded` 建立新 process group（`setpgid(0, 0)`），逾時時以 `libc::kill(-(pid as i32), SIGKILL)` 清理整個子進程群組。

#### [SEC-M6] `media_server.rs` 缺 `Host` 驗證、`nosniff`、絕對路徑檢查與 FIFO 防護
- **位置**：`src-tauri/src/media_server.rs:119-240, 257-260`
- **問題說明**：未驗證 `Host` header 為 `127.0.0.1:<port>`（DNS rebinding 縱深防禦）、回應缺少 `X-Content-Type-Options: nosniff`、`media_url` 未檢 `require_absolute`、`handle` 先 `File::open` 才檢 `is_file()`（FIFO 會永久佔用 `MAX_CONNS = 32` 名額），且串流寫入無寫入逾時；另 416 回應重複送出兩次 `Connection: close`（lines 203, 252）。
- **建議修復**：強制驗證 `Host` header、補上 `X-Content-Type-Options: nosniff`、於 `url_for` 驗證 `require_absolute` 與一般檔案、以 `O_NONBLOCK` 開檔後 `fstat`，並修正 416 重複標頭。

#### [SEC-M7] `extract_tar` 未過濾特殊檔案、`compress_op` TOCTOU 誤刪 symlink 與 `TarGz` 吞沒結束錯誤
- **位置**：`src-tauri/src/ops/archive.rs:100-136, 216-218, 221-247, 573-597`
- **問題說明**：
  1. `extract_tar` 只檢 `is_symlink()`，未過濾 `EntryType::Fifo` 等特殊檔（非 root 即可建立 FIFO，後續預覽/搜尋/壓縮將卡死），亦未顯式檢查 hardlink（`EntryType::Link`）。
  2. `compress_op` 先檢 `exists() || is_symlink()` 再以非 `O_EXCL` 開檔，競態時不僅可能截斷新檔，失敗清除 `remove_file(&dest)`（line 134）更會誤刪原本存在的 symlink。
  3. `write_tar`（line 217）直接 `drop(inner)` 而非呼叫 `GzEncoder::finish()?`，磁碟滿導致 gzip trailer 寫入失敗時會**靜默回報壓縮成功**並留下損毀檔；`gz_file` 遇 I/O 錯誤未清理半成品。
- **建議修復**：`extract_tar` 改採白名單（`Regular` / `Directory` / 經驗證之 `Symlink`）；`compress_op` 改用 `O_CREAT | O_EXCL | O_NOFOLLOW` 且僅於建立成功後才於錯誤時刪除；`write_tar` 回傳 `W` 並對 `TarGz` 顯式 `enc.finish()?`。

#### [SEC-M8] 垃圾筒還原路徑驗證不足與 `.DS_Store` 解析風險
- **位置**：
  - `src-tauri/src/platform/macos/trash.rs:38-89, 141-155`
  - `src-tauri/src/platform/linux/mod.rs:31-47`
  - `src-tauri/src/ops/delete.rs:54-69`
- **問題說明**：
  - macOS `original_location` 未驗證 `.DS_Store` 的 `ptbN`（若以 `/` 開頭會透過 `PathBuf::join` 覆寫整個目的路徑）與 `ptbL`（未擋 `..` 與 NUL）；讀取 `.DS_Store` 無大小上限且跟隨 symlink，`record()` 中 `blob` 長度與位移運算未做邊界與溢位檢查。
  - Linux `parse_trash_info` 未拒絕 `Path=` 中的 `..` 元件；`delete.rs:56` 未驗證 `item.file_name().is_some()`，且 `symlink_metadata` 與 `rename` 間存在覆寫競態。
- **建議修復**：對 `ptbN` 驗 `is_listing_name`、拒絕 `Component::ParentDir` 與 NUL、以 `O_NOFOLLOW` + 上限（如 32 MiB）讀取 `.DS_Store`、位移改用 `checked_add`/`checked_mul` 並檢查 `blob` 邊界。

#### [SEC-M9] 並發 `batch_rename` 暫存檔名碰撞可能靜默覆寫使用者檔案
- **位置**：`src-tauri/src/commands/fs.rs:127-147`
- **問題說明**：兩階段改名使用 `format!(".ycrn_{}_{i}", std::process::id())`；因 `batch_rename` 為 `async` 且無鎖，同一目錄同時執行兩次批次改名會算出相同暫存檔名，`std::fs::rename` 在 POSIX 上會靜默覆蓋另一個檔案。另 `commands/fs.rs` 多個本機指令缺少 `require_absolute`，且未拒 `file_name().is_none()` 路徑。
- **建議修復**：暫存檔名加入遞增 `AtomicU64` 或隨機 token，補上 `require_absolute` 與 `file_name().is_some()` 檢查。

#### [SEC-M10] SFTP 缺少逾時保護、未驗證 `host`/`identity_file`，與 `download_item` 內部 symlink 合併
- **位置**：
  - `src-tauri/src/fs/sftp.rs:273-381`、`src-tauri/src/commands/sftp.rs:59-153`
  - `src-tauri/src/ops/remote.rs:282-308`
- **問題說明**：
  1. `client::Config::default()` 未設 `inactivity_timeout`，且連線、驗證、SFTP 請求均無 `tokio::time::timeout`，遇無回應伺服器會永久占用工作執行緒；`params.host` 與 `params.identity_file` 未做格式與絕對路徑驗證。
  2. `remote::download_item` 丟棄 `create_dir_inside(root, dest)?` 回傳的正規化路徑，且未如 `transfer::copy_dir` / `upload_item` 將既有目錄 symlink 視為衝突，會直接穿過 `root` 內既有 symlink 寫入目標資料夾。
- **建議修復**：為 SFTP 連線與操作加上 `tokio::time::timeout` 與 keepalive/inactivity timeout；驗證 `host` 與 `identity_file`；`download_item` 檢查既有 symlink 衝突並沿用 `create_dir_inside` 回傳路徑。

#### [SEC-M11] 平台整合安全缺口（macOS AppleScript / PTY、Linux SMB / PATH）
- **位置**：
  - `src-tauri/src/platform/macos/mod.rs:201-212`（`get_info`）
  - `src-tauri/src/platform/macos/smb.rs:168-252`（`run_with_password_prompt`）
  - `src-tauri/src/platform/linux/smb.rs:23-26, 31-47, 116, 149, 199-215`
  - `src-tauri/src/platform/linux/mod.rs:174, 194, 322, 351, 445`
- **問題說明**：
  1. macOS `get_info` 直接將檔名拼接進 AppleScript 字串（未處理合法的 APFS `\r`/`\n` 檔名），建議改由 `osascript -e 'on run argv' ... -- <paths>` 以 argv 傳遞。
  2. macOS `run_with_password_prompt` 未對 `slave_fd` 設 `FD_CLOEXEC`（多執行緒併發 spawn 可能繼承 fd 導致 PTY 讀取執行緒永久卡死）；`seen` 緩衝無上限且每次 `O(N²)` 掃描；寫入密碼失敗提前返回時未 `kill`/`wait` 子進程（產生 zombie）。
  3. Linux `smbclient --user={username}` 中 `%` 會被解析為 `USER%PASS`（外洩於 argv/`ps`）；`smb_unmount` 未限制僅能卸載掛載點（亦缺 `require_absolute` 與逾時），且 `parse_smb_dir` 解碼後的 `server` 直接拼回 `smb_url` 未驗證；`gvfs_dir` 未過濾空字串 `XDG_RUNTIME_DIR=""`；Linux 多處裸名呼叫 `"gio"`/`"gdbus"`/`"gsettings"`/`"smbclient"` 而非使用標準絕對路徑。
- **建議修復**：macOS `get_info` 改 `on run argv`；PTY 補上 `slave_fd` `FD_CLOEXEC`、限制 `seen` 大小與 RAII 子進程清理；Linux 改由 `find_cli` 優先解析 `/usr/bin/...`、驗證 `unmount` 掛載路徑與 `server`、以 `-A` 認證檔或拒 `%` 避免 `smbclient` 密碼解析問題。

#### [SEC-M12] 解析 Panic / 整數溢位、FIFO 搜尋卡死與並發競態
- **位置**：
  - `src-tauri/src/fs/local.rs:95, 242-259`（Linux `mountinfo` `unescape` 與 `statfs`）
  - `src-tauri/src/fs/archive_fs.rs:44, 103, 501`
  - `src-tauri/src/commands/find.rs:86-104, 150-182`
  - `src-tauri/src/watch.rs:20-68`、`src-tauri/src/lib.rs:174-187`
  - `src-tauri/src/commands/ops.rs:68-78, 315-324`
- **問題說明**：
  1. **Linux `mountinfo` UTF-8 Panic 與亂碼（`fs/local.rs:247-256`）**：`&s[i + 1..i + 4]` 在 `\` 後鄰接多位元組 UTF-8 字元時直接 panic，且 `out.push(bytes[i] as char)` 把非 ASCII UTF-8 位元組當成 Latin-1，導致中文等非 ASCII 掛載點全變亂碼。
  2. **整數溢位**：`archive_fs.rs:103, 501`（`mtime * 1000`）、`line 44`（`d == 0` 下溢）、`local.rs:95`（`f_bavail * block`）。
  3. **`start_find` FIFO 阻塞與無遞迴深度上限**：內容搜尋直接 `File::open` 未檢 `is_file()`（遇 FIFO 永久卡死且無法取消）；本機遞迴無深度上限（SFTP 有 128 層上限）。
  4. **`watch.rs` 視窗關閉競態洩漏**：`watch_dir` 無每視窗數量上限且無 `require_absolute`；若在 `spawn_blocking` 建立 watcher 期間視窗關閉，watcher 與 debounce 執行緒會永久洩漏。
  5. **`ops.rs` 取消／衝突死鎖競態**：若 `cancel_op` 在 `TauriSink::conflict` 寫入 `conflict_tx` 前一瞬執行，`rx.recv()` 會永久阻塞。
- **建議修復**：`unescape` 改收集 `Vec<u8>` 再 `from_utf8_lossy`；整數運算改 `checked_*`/`saturating_*`；`find` 加 `is_file()` 與深度上限；`watch_dir` 設上限並在插入前確認視窗仍存在；`TauriSink::conflict` 在持有 `conflict_tx` 鎖時先檢查 `cancel` 旗標。

---

### 低風險與縱深防禦（Low）

| 編號 | 位置 | 問題說明與建議 |
| :--- | :--- | :--- |
| **SEC-L1** | `src-tauri/src/commands/system.rs:397-415` | `open_url` 驗證 `tauri::Url::parse(&url)` 後卻把原始未正規化 `url` 交給 `open::that`（可能殘留控制字元或空白）。應改傳 `parsed.as_str()`。 |
| **SEC-L2** | `src-tauri/src/keychain.rs:31-46`、`fs/sftp.rs:44-52` | `smb_account` / `ssh_account` 未對 `username`（可含 `@`）與 `share` 做 URL 編碼，可能鍵名碰撞；記憶體中密碼為普通 `String` 未於 drop 時 zeroize。 |
| **SEC-L3** | `src-tauri/src/fnkeys.rs:114-125` | `IOHIDCopyCFTypeParameter` 回傳之 `CFTypeRef` 未檢查 `CFGetTypeID` 即強制轉型為 `CFNumber`（潛在 FFI UB）。改用 `CFType::wrap_under_create_rule(value).downcast::<CFNumber>()`。 |
| **SEC-L4** | `ops/queue.rs:301-319`、`ops/sync.rs:280-296`、`ops/archive.rs:454`、`ops/transfer.rs:247`、`platform/{macos,linux}/mod.rs` | `keep_both_with` 達 9,998 次碰撞後靜默回傳原 `dest` 造成覆寫；`sync_op` 遠端來源缺 `real_path` 逃逸檢查；`set_permissions` 以路徑而非 open fd 執行（TOCTOU）且 `transfer` 未濾掉 setuid/setgid；多處外部指令參數前可加 `--` 縱深防禦；`extract_7z` 對 0-byte 條目未檢查 `ctx.cancelled()`；`commands/smb.rs:57` 對含 `:port` 的 `server` 會拼成 `nas:445:445`。 |
| **SEC-L5** | `src/components/Dialogs.tsx:97-100, 427`、`src/lib/renameMask.ts:91`、`ConnectDialog.tsx:274-275`、`Panel.tsx:27` | `CopyMoveDialog` 對側為 SFTP 時只要稍微編輯路徑就會靜默切換成 `local`；`CompressDialog` 未拒檔名中的 `/` 與 `..`；`renameMask.ts` 未拒控制字元與 Bidi 字元；`smb.start_in` 未擋 `..`；`Panel.tsx:27` `startsWith(volPath)` 缺路徑邊界比對（`/Volumes/Data2` 誤中 `/Volumes/Data`）。 |
| **SEC-L6** | `QuickView.tsx:605`、`MarkdownPreview.tsx:81`、`OfficePreview.tsx:45`、`SettingsDialog.tsx`、`store/settings.ts:26,40`、`src-tauri/src/config.rs:193,203,216`、`icons.rs:55`、`fonts.rs:76`、全域 `.lock().unwrap()` | SVG 預覽建議改 `data:image/svg+xml;base64,...` 取代同源 `blob:`；`PlantUmlPreview` 補上 `img.draggable = false`；`docx-preview` 可改用獨立 `createHTMLDocument`；`SettingsDialog` 建議驗 URL 協定、避免每鍵呼叫 `officeTools`、`settings.ts` 過濾 `\r\n` 並夾限 `font_scale`；`config.rs` TOML 解析失敗應備份而非靜默覆蓋、`write_replacing` 改隨機 temp 名並回傳儲存錯誤；`icons.rs` 避免在 macOS 主執行緒對網路磁碟做 `exists()`；`fonts.rs` `fc-list` 改用絕對路徑與 `output_bounded`；將遍佈後端的 `.lock().unwrap()` 改為毒鎖復原或錯誤回傳。 |

---

## 二、多餘、未使用與重複程式碼（Redundant, Dead & Duplicated Code）

### 整體乾淨度確認（已驗證無問題項目）
- **npm 相依套件（`package.json`）**：19 個 runtime + 7 個 dev 套件**全部有使用**（0 未使用）。
- **Cargo 相依套件（`src-tauri/Cargo.toml`）**：26 個 crate **全部有使用**（0 未使用）。
- **Tauri IPC 與事件**：63 個 `#[tauri::command]` 與 3 個事件（`menu:settings`、`fs:changed`、`config-patched`）**前後端 100% 對應**（0 孤立指令／事件）。
- **CSS Classes（`src/styles/base.css`）**：全部 268 個 class 皆有使用（0 未使用 class、0 無效重複規則）。
- **i18n 語系檔（`en.json` / `zh-TW.json`）**：0 未使用 key（`zh-TW.json:425` 獨有的 `quickview.reveal_linux` 為刻意平台差異設計）。
- **Rust `#[allow(...)]`**：共 4 處（`lib.rs:14`、`fonts.rs:40` 為跨平台測試所需；`ops/remote.rs:237`、`ops/sync.rs:230` 為 `clippy::too_many_arguments`），皆合理。

---

### A. 明確多餘／未使用的程式碼與欄位

1. **Linux 上多餘註冊的 `stream://` 協定**
   - `src-tauri/src/lib.rs:115-117` 與 `src-tauri/tauri.linux.conf.json:28`：前端僅在 macOS 使用 `stream://`（`src/ipc/commands.ts:224-225`），Linux 上註冊此協定既未使用又擴大攻擊面（參見 SEC-H3）。
2. **未曾被讀取的 CSS 變數 `--font-heading`**
   - 定義於 `src/styles/tokens.css:66`，並在 `src/store/settings.ts:35` 動態設定，但全專案從未使用 `var(--font-heading)`。
3. **`Dialog` 狀態中未讀取的欄位**
   - `src/store/dialogs.ts:11`：`{ type: "find"; side: Side; ... }` 的 `side` 在 `actions.ts:357`、`Toolbar.tsx:299` 有傳入，但 `FindDialog.tsx` 從未讀取（line 157 改讀 `usePanels.getState().activeSide`）。
   - `src/store/dialogs.ts:26`：`{ type: "compress"; side: Side; ... }` 的 `side` 在 `actions.ts:425` 有傳入，但 `CompressDialog`（`Dialogs.tsx:411-466`）從未讀取。
   - `src/store/dialogs.ts:12`：`section?: "general" | "hotkeys"` 僅有單一呼叫點（`actions.ts:490`）傳入預設值 `"general"`，`"hotkeys"` 從未使用。
4. **未實際使用的 IPC 欄位**
   - `src-tauri/src/commands/find.rs:27, 218`：`FindEvent::Done.error` 永遠填入 `None`。
   - `src/ipc/types.ts:153`：`FindEvent` `"done"` 的 `count` 欄位由後端送出，但 `FindDialog.tsx:123-133` 未使用（改讀 `results.length`）。
   - `src/ipc/types.ts:122`：`Config.window_size` 僅供型別對齊 Rust，前端完全未讀寫。
5. **7 處不可達的 `_` 衝突分支**
   - `src-tauri/src/ops/transfer.rs:144-154, 194-204` 與 `src-tauri/src/ops/remote.rs:114-127, 142-148, 178-187, 260-273, 320-326`：因 `OpCtx::ask_conflict_with`（`ops/queue.rs:220-230`）已將 `OverwriteAll`/`SkipAll` 正規化，7 處 `_ => ...` 永遠不會執行。改回傳 4 變體 enum 即可消除。
6. **多餘的匯出（`export` / `pub` / `pub use`）**
   - 前端：`src/keymap/actions.ts:69` `export interface Action` 僅在檔內使用。
   - Rust 未使用的 re-export：`src-tauri/src/commands/preview.rs:9`（`pub use crate::platform::MediaInfo;`）、`src-tauri/src/commands/system.rs:9`（`pub use crate::platform::OpenApp;`）。
   - Rust 僅限同檔或單元測試使用卻宣告為 `pub` / `pub(crate)` 的項目：
     - `watch.rs:9`（`watch_key`）、`state.rs:80`（`sftp`）、`media_server.rs:119`（`url_for`）、`commands/config.rs:10,13,25`（`CONFIG_PATCHED`, `ConfigPatched`, `merge_patch`）、`fs/sftp.rs:240`（`connect_checked`）、`ops/archive.rs:19,256`（`excluded`, `Meter` 及其欄位）、`ops/queue.rs:114-115,237`（`cancel`, `pause`, `TestSink`）、`platform/macos/smb.rs:14`（`find_mount`）、`platform/linux/smb.rs:31,57`（`parse_smb_dir`, `find_mount`）、`platform/linux/mod.rs:223,278,404,422`（`parse_gio_mime`, `desktop_entry_name`, `parse_ffprobe`, `icon_names`）。
7. **零星語法與註解冗餘**
   - `src/components/FindDialog.tsx:9, 16`：同一模組 `../lib/path` 重複 import 兩次。
   - `src-tauri/src/commands/office.rs:275`：`let custom = soffice_path;` 無意義搬移。
   - `src-tauri/src/fs/sftp.rs:272, 381`：`connect_with` 整段函式體被多餘大括號 `{ ... }` 包裹；line 59 文件註解仍提及舊名稱 `trust_host_key`（現為 `trusted_fingerprint`）。
   - `src-tauri/src/ops/remote.rs:136`：`!src.is_symlink()` 為多餘條件（lines 107-135 已排除所有未跟隨的 symlink）。
   - `src-tauri/src/ops/sync.rs:93-95`：單次呼叫的單行包裝函式 `files_equal`（且忽略已 import 的 `transfer` 改寫 `super::transfer`）。

---

### B. 重複程式碼與效能冗餘（重構建議）

#### Rust 後端
1. **主執行緒橋接重複 3 次**：`icons.rs:162-171`、`fonts.rs:60-69`、`commands/drag.rs:56-86` 皆重複 `run_on_main_thread` + `mpsc::channel` + `spawn_blocking` 樣板，建議抽成共用 helper `on_main_thread(app, f)`。
2. **`icons.rs` 平台快取流程重複**：`icons.rs:151-205` 在 macOS 與非 macOS 兩個 `#[cfg]` 版本的 `file_icon` 重複快取讀寫，可只針對 cache miss 抽取平台函式。
3. **未複用既有 `commands::blocking`**：`commands/preview.rs:28-47`、`watch.rs:28-67`、`commands/system.rs:121,164,356,430` 手寫 `spawn_blocking` 樣板，可統一複用 `commands::blocking`。
4. **SMB 平台輔助重複**：`platform/macos/smb.rs` 與 `platform/linux/smb.rs` 重複定義 `OUTPUT_MAX`/`MOUNT_TIMEOUT`/`LIST_TIMEOUT`、密碼遮罩邏輯與掛載輪詢迴圈（且 macOS `mount` 因此漏了遮罩），建議集中於 `platform/` 共用模組。
5. **1 MiB 分塊複製迴圈重複與迴圈內重複配置**：`ops/transfer.rs:225-243`、`ops/archive.rs:176-188`、`ops/archive.rs:226-240` 重複相同複製迴圈；且 `ops/archive.rs:176`（`write_zip`）與 `fs/archive_fs.rs:444`（`extract_entries`）皆在 `for` 迴圈內對**每個檔案重新配置 1 MiB `Vec`**，移至迴圈外即可大幅減少配置。
6. **進度計量器與 Tokio Runtime 重複**：`fs/archive_fs.rs:333-353`（`TarMeter`）與 `ops/archive.rs:256-283`（`Meter`）高度重複；`fs/sftp.rs:27-30` 另建第二套全域多執行緒 Tokio runtime，而非複用 Tauri 既有的 `tauri::async_runtime`。
7. **其他後端重複與多餘系統呼叫**：
   - `commands/plantuml.rs:146-154` 手寫 `/usr/bin/which java`，重複 `platform::find_cli`。
   - `lib.rs:58-66` 與 `dock.rs:47-58` 重複「開新視窗 / New Window」語系判斷；`macos/mod.rs:175-177` 與 `linux/mod.rs:215-217` 重複 `OpenApp` 排序。
   - 多處 `p.exists() || p.is_symlink()`（`transfer.rs:142,193,212`、`remote.rs:259,319`、`archive.rs:100`、`commands/fs.rs:115`）發起兩次 syscall，可統一改為單次 `lstat`（`p.symlink_metadata().is_ok()`）。
   - `commands/office.rs:158` `cell_text(&value.clone().into())` 對每格字串做兩次 clone；`ops/sync.rs:191,201-228` 對每個檔案重複 `canonicalize` / SFTP `real_path`；`ops/remote.rs:113,118` 對同一遠端路徑連續多次 SFTP stat/exists RPC；`ops/archive.rs:63,115,211` 重複 `stat` 檔案。
   - 憑證管理模組歸屬不一致：SSH Keychain 指令在 `commands/config.rs:108-138`，SMB 則在 `commands/smb.rs:90-117`（可移至 `commands/sftp.rs` 或集中於 `commands/credentials.rs`）。

#### 前端 React / TypeScript
1. **Popover 外部點擊 + Escape 關閉重複 4 次**：`Panel.tsx:145-164`、`Toolbar.tsx:159-177`、`QuickView.tsx:174-192`、`ContextMenu.tsx:279-308`（且 `mousedown` 註冊於 capture/bubble 階段不一致），建議抽為共用 `useDismissable()` hook。
2. **非同步事件監聽清理重複 3 次**：`src/App.tsx:79-99, 102-112, 115-123` 的 `gone` + `unlisten` 模式可抽為小 helper `useTauriListen`。
3. **重複 UI 片段**：`ConnectDialog.tsx:583-675` 三處巢狀子對話框與兩處 Remember checkbox（lines 541-548, 657-664）；`CompareDialog.tsx:243-250` 與 `SyncDialog.tsx:151-158` 重複 `diffOnly` checkbox；`keymap/actions.ts:365-369, 403-407` 重複 archive 唯讀與空選取檢查。

---

### C. 專案目錄與本機建置產物說明

- **已納入 git 追蹤的文件檔**：根目錄 `File Manager Screens.html`（752 KB）與 `PLAN.md`（16 KB）為 `AGENTS.md` 明列之設計與架構參考文件，非孤立檔案。
- **未追蹤／本機殘留產物（已被 `.gitignore` 忽略）**：
  - `src-tauri/target/`：約 **88 GB** 本機 Rust 編譯快取，建議定期執行 `cargo clean` 釋放空間。
  - `target/flycheck0/`：專案根目錄殘留之舊版 rust-analyzer 檢查快取，可直接移除。

---

## 三、修復優先順序建議（Remediation Roadmap）

### P0 — 優先修復（直接影響 RCE 風險、憑證安全或資料完整性）
1. **隔離 Markdown / Mermaid 預覽（SEC-H1）** 並補齊 CSP `base-uri 'none'`、`form-action 'none'`、`object-src 'none'`（SEC-M2）。
2. **重構 Keychain 憑證流向與修復 `ConnectDialog` 密碼殘留／競態（SEC-H2）**，並遮罩 macOS `osascript` 錯誤中的密碼（SEC-H10）。
3. **加固 `stream://` 為隨機 token 授權並僅於 macOS 註冊（SEC-H3）**，同時限制 `read_preview` 的 `max_bytes` 與 FIFO 防護（SEC-H9）。
4. **修復檔案安全與資料毀損缺陷**：`copy_op` 正規化比對避免 0-byte 截斷、`create_dir_inside` 拒絕尾端 `..`、`extract_op` 處理空/點號檔名（SEC-H8）、`batch_rename` 並發暫存檔碰撞（SEC-M9）、macOS Gatekeeper Quarantine 寫入順序（SEC-H7）與 Linux `mountinfo` UTF-8 panic（SEC-M12）。
5. **收斂 `open_with` 執行權限與驗證 `update_config` 可執行路徑（SEC-H4）**。

### P1 — 近期修復（DoS、SSRF 與平台健壯性）
1. **補齊壓縮與 Office 解壓上限**（ZIP symlink 4 KiB 上限、解壓總量上限、Office/JSZip 未壓縮檢查、標頭 `saturating_add`）（SEC-H6）。
2. **加固 PlantUML Server 請求**（限制 `https://`、停用重導向、輸入上限）（SEC-H5）與 **`ffmpeg` / LibreOffice**（`-protocol_whitelist file`、私有隨機暫存目錄、子進程群組清理、`run_with_progress` 上限）（SEC-M4、SEC-M5）。
3. **補齊全介面 Bidi 檔名淨化（SEC-M1）** 與 **前端 ReDoS/OOM/ErrorBoundary（SEC-M3）**。
4. **修復 `media_server`、`extract_tar`、`compress_op`（`TarGz::finish`）、Trash 還原驗證、SFTP 逾時與平台整合缺口（SEC-M6 ~ SEC-M12）**。

### P2 — 程式碼整理與縱深防禦
1. 清理所有明確多餘程式碼（`--font-heading`、`Dialog` 無用 `side` 欄位、`FindEvent::Done.error`、重複 import、多餘 `pub`/`export` 與 7 處不可達衝突分支）。
2. 抽取前後端重複邏輯（`on_main_thread`、`useDismissable`、共用 SMB helpers、共用 chunked copy 並將 1 MiB buffer 移出迴圈），以及處理 SEC-L1 ~ SEC-L6 細項。
