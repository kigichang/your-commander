# Your Commander — Code Review 報告 (`argon-code-review-2`)

- **審查日期**：2026-11-07
- **審查範圍**：前端（`src/`、`package.json`、`vite.config.ts`、`tsconfig.json`）與後端（`src-tauri/src/`、`Cargo.toml`、`tauri.conf.json`、`tauri.linux.conf.json`、`capabilities/default.json`）
- **審查面向**：資安問題（Security）、效能與記憶體洩漏問題（Performance）、多餘／重複程式碼（Redundant & Duplicate Code）

---

## 一、資安問題（Security Issues）

### 1.1 `[High]` 壓縮檔選擇性複製（F5）與 `.gz`／`.7z` 解壓縮缺少目錄 Symlink 穿越防護與 `.` 路徑檢查
- **檔案位置**：
  - `src-tauri/src/fs/archive_fs.rs:234-350`（`extract_entries`）
  - `src-tauri/src/ops/archive.rs:347-348`（`Extractable::Gz`）
  - `src-tauri/src/ops/archive.rs:455-469`（`extract_7z`）
- **問題說明**：
  1. 在 `archive_fs::extract_entries`（第 239 行）中，`rel.split('/').any(|c| c == ".." || c.is_empty())` 未排除 `c == "."` 與反斜線 `\`。若 `rel == "."`，`dest_dir.join(".")` 會指向 `dest_dir` 本身；若 `dest_dir` 為 symlink，`replace_file_nofollow(&out)` 會先執行 `std::fs::remove_file(&dest_dir)`。
  2. 在 `archive_fs.rs` 第 278、282、337、344 行直接呼叫 `std::fs::create_dir_all`，而未如同 `ops/archive.rs::extract_zip`／`extract_tar` 使用 `safe_path::create_dir_inside(&root, ...)`。由於 `replace_file_nofollow(&out)` 僅對最後一層檔名設定 `O_NOFOLLOW`，若 `dest_dir` 內部中繼目錄是或被建立成指向外部的 symlink，解壓縮仍會穿越 symlink 並覆寫外部目錄的檔案。
  3. `ops/archive.rs` 第 347 行（`Extractable::Gz`）直接使用 `std::fs::File::create(&out)` 而非 `create_file_nofollow(&out)`；第 455–469 行（`extract_7z`）使用 `sevenz_rust2::default_entry_extract_fn`，其內部同樣直接呼叫 `std::fs::create_dir_all` 與 `std::fs::File::create`，未經過 `create_dir_inside` 與 `create_file_nofollow`。
- **修正建議**：
  - 在 `archive_fs::extract_entries` 開頭將 `dest_dir` canonicalize 為 `root`，使用 `safe_path::is_plain_name` 驗證每個路徑片段，並在寫檔前以 `create_dir_inside(&root, parent)` 建立父目錄。
  - 在 `Extractable::Gz` 與 `extract_7z` 中統一改用 `create_dir_inside` + `create_file_nofollow`。
- **處理結果**（Claude，2026-11-07）：部分採納，嚴重度應下修——三點皆無法讓壓縮檔內容寫到目的地之外。
  1. **`.` 片段：已修（加固）。** `a/.` 這類 entry 不會逃逸，但 `dest/a/.` 會指向資料夾本身，寫檔時 EISDIR 讓整個 F5 中斷。`extract_entries` 的檢查加上 `c == "."`，改為跳過該 entry；新增測試 `dot_pieces_are_skipped_not_written_onto_the_folder`（拿掉修正會失敗）。
     - 「`dest_dir` 為 symlink 時 `replace_file_nofollow` 會先 `remove_file(&dest_dir)`」**不成立**：路徑是 `dest_dir/.`，`lstat("link/.")` 會穿過 link 解析到真正的目錄，`is_symlink()` 為 false，不會刪除任何東西（已實測）。
     - **反斜線 `\`：不修。** macOS/Linux 上 `\` 是一般檔名字元，不是分隔符，無法逃逸；擋掉反而讓 Windows 製作的 `dir\file.txt` 無法用 F5 複製出來。
  2. **中繼目錄改用 `create_dir_inside`：不採納。** 此為 `9c4567d` 的刻意設計：F5 複製這條路徑從不建立 symlink（tar 非一般檔案略過、zip 的 link 寫成一般檔案），entry 路徑也已排除 `..`，所以路徑上的 symlink 只可能是使用者目錄中原本就有的。目的地是使用者既有的資料夾（不像完整解壓縮是全新資料夾），改用 `create_dir_inside` 會在使用者自己放了同名 symlink 資料夾時拒絕複製，屬行為退步。
  3. **`.gz`：已修（一致性）。** 輸出檔名來自使用者自己的檔名而非壓縮檔內容，且寫入全新資料夾，原本即無實際風險；仍將 `File::create` 改為 `create_file_nofollow`，與其他解壓縮路徑一致。
     **`.7z`：不需修。** sevenz-rust2 0.24.1 在呼叫 `default_entry_extract_fn` 前先以 `safe_join` 拒絕 `..`、絕對路徑與反斜線；它不建立 symlink（link entry 寫成一般檔案）；輸出目錄為全新資料夾（已存在則 keep-both），裡面不會有預先存在的 link。

### 1.2 `[High]` SFTP 目錄同步與遞迴刪除未驗證遠端回傳檔名（Remote Path Traversal）
- **檔案位置**：
  - `src-tauri/src/ops/sync.rs:59-89`（`collect_remote`）
  - `src-tauri/src/ops/sync.rs:226-248`（`sync_op`）
  - `src-tauri/src/fs/sftp.rs:347-385`（`SftpConnection::list`）
  - `src-tauri/src/fs/sftp.rs:443-470`（`SftpConnection::remove_recursive`）
- **問題說明**：
  - 雖然 `ops/remote.rs:261`（`download_item`）有針對惡意或異常 SFTP 伺服器回傳的 `entry.name` 檢查 `if !is_plain_name(&entry.name)`，但其他 SFTP 目錄消費者並未檢查：
    1. `ops/sync.rs::collect_remote`（第 66–87 行）直接將 `entry.name` 串接至 `rel`。後續執行 `sync_op`（第 226–248 行）時，`dest_base.join(&item.rel)` 搭配 `std::fs::create_dir_all(parent)` + `conn.download_file(ctx, &src, &dest)` 可將檔案寫入 `dest_base` 之外的任意本機路徑。
    2. `ops/sync.rs::sync_op`（第 226–248 行）本身亦未驗證 `item.rel` 的 `Component::Normal`，且未對本機端使用 `create_dir_inside`。
    3. `fs/sftp.rs::SftpConnection::remove_recursive`（第 450–455 行）直接對 `self.list(path)` 的結果遞迴刪除 `&entry.path`。若伺服器在 `READDIR` 回傳 `".."` 或 `"../x"`，會導致往父目錄逃逸刪除遠端檔案。
- **修正建議**：
  - 直接在 `SftpConnection::list`（`src-tauri/src/fs/sftp.rs:348`）統一以 `crate::ops::safe_path::is_plain_name(&name)` 過濾所有遠端 `READDIR` 項目。
  - 在 `ops/sync.rs::sync_op` 驗證 `item.rel` 僅包含 `Component::Normal`，並於本機寫入時使用 `create_dir_inside` + `replace_file_nofollow`。
- **處理結果**（Claude，2026-11-07）：採納，「遠端→本機」同步確為真漏洞；遞迴刪除風險較低。
  - **查證**：russh-sftp 3.0.1 的 `ReadDir` 只濾掉名稱恰為 `.` 與 `..` 的項目，`""`、`../x`、`/abs/path`、`a/b` 都會原樣回傳。
    - 同步：惡意伺服器回傳 `../../.zshrc` 或 `/Users/x/.zshrc`（`Path::join` 遇絕對路徑會整個取代 base），「遠端→本機」同步就會寫到 `dest_base` 之外——**成立**。
    - 遞迴刪除：逃逸只會刪到伺服器自己的檔案，伺服器本來就能刪；但空字串名稱會讓 `path.join("")` 指回自己，`remove_recursive` 無限遞迴到 stack overflow——屬穩健性問題。
  - **已修**：
    1. `SftpConnection::list` 統一過濾，涵蓋所有使用者（面板、搜尋、同步、下載、遞迴刪除、衝突偵測）。採新的 `safe_path::is_listing_name`（非空、非 `.`/`..`、不含 `/` 與 NUL），而非建議的 `is_plain_name`：反斜線在 macOS/Linux 是一般檔名字元，用 `is_plain_name` 會讓遠端 `a\b` 這類合法檔案從面板消失。`is_plain_name` 改為建立在 `is_listing_name` 之上（再排除 `\`），`download_item` 原有的較嚴格檢查保留。
    2. `sync_op` 開頭驗證每個 `item.rel` 的每一段都通過 `is_listing_name`；任一列不合格就整批拒絕，不會先複製一部分（`rel` 來自前端 IPC，縱深防禦）。
  - **不採納**：本機端改用 `create_dir_inside`。`rel` 驗證後，路徑上的 symlink 只可能是使用者目錄中原有的（`collect_local`／`collect_remote` 跳過 symlink，同步從不建立 link），理由同 1.1 第 2 點。最後一層檔案：下載走 `download_file` 已是 `O_NOFOLLOW`；本機→本機走 `transfer::copy_file`，歸入 1.4 處理。
  - **測試**：新增 `sync_refuses_paths_that_leave_the_folders`（`../`、中段 `..`、絕對路徑、空字串、`//`、`./`；拿掉檢查會失敗）與 `listing_names`。`list` 的過濾需要惡意 SFTP 伺服器才能端對端測試，目前以 `is_listing_name` 單元測試涵蓋。

### 1.3 `[High]` `HtmlPreview.tsx` 的 CSP `<meta>` 注入可被繞過，結合 `assetProtocol` 全域開放（`["**"]`）有讀取本機檔案風險
- **檔案位置**：
  - `src/components/HtmlPreview.tsx:28-58`
  - `src-tauri/tauri.conf.json:26-32`
- **問題說明**：
  - `tauri.conf.json` 開啟了 `assetProtocol` 且 `"scope": ["**"]`，並在主視窗 CSP 允許 `asset: http://asset.localhost`。
  - `HtmlPreview.tsx` 的 `withHead`（第 47–58 行）試圖在 `<!doctype ...>` 之後插入 `<meta http-equiv="Content-Security-Policy">` 以限制 iframe 僅能載入當前目錄下的資源：
    ```ts
    const doctype = /^\s*<!doctype[^>]*>/i.exec(text);
    const end = doctype ? doctype[0].length : 0;
    return text.slice(0, end) + tags + text.slice(end);
    ```
  - **繞過方式**：
    1. 若 HTML 檔案開頭為包含 `>` 的 DOCTYPE 字串（例如 `<!DOCTYPE html SYSTEM "foo>bar">`），`/^\s*<!doctype[^>]*>/i` 會在雙引號內的第一個 `>` 提前結束，導致 `<meta http-equiv="Content-Security-Policy" ...>` 被插入在 DOCTYPE 標籤**內部**並被 HTML parser 忽略。
    2. 若 HTML 開頭有註解（`<!-- ... -->\n<!DOCTYPE html>`），`<meta>` 會被插在 `<!DOCTYPE html>` 之前而觸發 quirks mode，或若開頭含特殊控制字元使 HTML5 parser 隱式開啟 `<body>`，`<meta>` CSP 同樣會失效。
    3. 一旦 `<meta>` CSP 失效，`iframe srcDoc` 將直接繼承主視窗 CSP，可透過 `asset://localhost/...` 讀取整個檔案系統的任意檔案。
- **修正建議**：
  - 在 `withHead` 中先剝除開頭的 BOM、空白、所有 `<!-- ... -->` 註解與既有的 `<!doctype ...>`，再由程式固定輸出乾淨的 `<!DOCTYPE html><head><meta http-equiv="Content-Security-Policy" ...><base ...></head>`。
  - 為 `<iframe>` 加上 `referrerPolicy="no-referrer"`。
- **處理結果**（Claude，2026-11-07）：漏洞確實存在，但 Argon 舉的兩個例子都不成立；真正的繞過是它只模糊帶過的「特殊字元」那一項。已修。
  - **查證方式**：以 parse5 7.1.2（WHATWG 規格實作）解析 `withHead` 的輸出，檢查 CSP `<meta>` 的父節點是否為 `<head>`（`<meta>` CSP 不在 `<head>` 內即被忽略）。
    1. DOCTYPE 引號內含 `>`（`<!DOCTYPE html SYSTEM "foo>bar">`）：**不成立**。規格的 DOCTYPE 狀態在引號內遇到 `>` 也會結束（abrupt-doctype-system-identifier），與 `[^>]*` 一致；`<meta>` 落在 `<head>`。
    2. DOCTYPE 前有註解：**不成立（非繞過）**。此時不比對 DOCTYPE，`<meta>` 插在最前面，仍落在 `<head>`；代價只是頁面進入 quirks mode（排版差異）。
    3. 「特殊字元使 parser 隱式開啟 `<body>`」：**成立**。`/^\s*<!doctype/` 的 JS `\s` 也比對 NBSP、VT、U+1680、U+2000–U+200A、U+2028/2029、U+202F、U+205F、U+3000，但 HTML parser 只把 TAB/LF/FF/CR/SPACE 視為空白；這些字元被當成文字而開啟 `<body>`，其後的 DOCTYPE 被丟棄，插在它後面的 `<meta>` 落入 `<body>` 而失效。實測 NBSP／VT／U+3000／U+2028 開頭皆可繞過。
  - **實際影響**：iframe 為 `sandbox=""`（無 script），且主視窗 CSP 不允許任何遠端來源，所以無法外送資料；但頁面可透過 `asset://` 顯示資料夾以外的本機圖片／樣式，違背預覽 CSP「只能載入同資料夾檔案」的保證。
  - **已修**：`HtmlPreview.tsx` 的 DOCTYPE 前導空白改為 HTML 規格的 `[\t\n\f\r ]*`。從原始碼抽出 regex，以 parse5 跑 50,220 個案例（開頭字元 U+0000–U+30FF 逐一，加上 BOM、註解等組合，各搭配四種 DOCTYPE 寫法），CSP 全部落在 `<head>`；舊 regex 在上述字元下失敗。
  - **不採納**：
    - 剝除開頭註解與 DOCTYPE 後重新輸出 `<!DOCTYPE html>`：剝除註解必須完全複製規格的註解斷詞（`<!-->`、`<!--->`、`--!>` 等特例），一旦判斷錯誤，`<meta>` 會被包進註解裡，反而造成繞過；而它解決的只是「註解在 DOCTYPE 前時進入 quirks mode」這個排版問題。強制輸出 `<!DOCTYPE html>` 也會讓沒有 DOCTYPE 的舊頁面改以 standards mode 排版。
    - `referrerPolicy="no-referrer"`：iframe 無 script，CSP 只允許 `asset:` 與 `data:`，不會對任何遠端發出請求，referrer 無處可送。
    - `assetProtocol` 的 `scope: ["**"]`：檔案管理員需要預覽任意位置的檔案；限制改由預覽文件自身的 CSP 負責，即本項修正的對象。

### 1.4 `[Medium-High]` 本機 `transfer::copy_file` 在覆寫時會穿透既有 Symlink，且未偵測懸空 Symlink（Dangling Symlink）
- **檔案位置**：
  - `src-tauri/src/ops/transfer.rs:144-175`
- **問題說明**：
  1. 第 144 行 `if dest.exists()` 在 `dest` 為**懸空 symlink**（`dest.is_symlink() == true` 但目標不存在）時會回傳 `false`，導致不跳出衝突詢問，直接進入第 174 行 `std::fs::File::create(&dest)`，順著懸空 symlink 在外部目標路徑建立並寫入檔案。
  2. 當 `dest` 為指向既有檔案的 symlink 且使用者選擇 `Overwrite` 時，第 174 行 `std::fs::File::create(&dest)` 會截斷並覆寫 **symlink 指向的目標檔案**，而非取代 symlink 本身（與 `ops/remote.rs:285-293` 及 `fs/archive_fs.rs:286` 使用 `replace_file_nofollow` 的行為不一致）。
- **修正建議**：
  - 將第 144 行改為 `if dest.exists() || dest.is_symlink()`，並將第 174 行的 `std::fs::File::create(&dest)` 改為 `super::safe_path::replace_file_nofollow(&dest)`。
- **處理結果**（Claude，2026-11-07）：兩點皆成立，已依建議修正，另補上兩處同類問題。
  1. **懸空 symlink**：`dest.exists()` 跟隨 link，懸空時為 false，不詢問就以 `File::create` 在 link 指向處建立檔案。改為 `dest.exists() || dest.is_symlink()`，懸空 link 也算衝突。
  2. **覆寫穿透**：選 Overwrite 時 `File::create` 會截斷 link 指向的目標檔。改用 `replace_file_nofollow`：link 本身被取代，期間才出現的 link 則讓 `O_NOFOLLOW` 開檔失敗；與下載、壓縮檔 F5 的行為一致。
  - **另補（同類）**：
    - 同裝置移動的 `rename` 快速路徑原本只看 `!dest.exists()`，遇懸空 link 會不詢問就以 rename 蓋掉它；改為同時排除 `is_symlink()`，交由一般流程詢問。
    - 重建 symlink 的分支原本以 `dest.exists()` 決定是否先刪除目的地；懸空 link 不會被刪，選 Overwrite 後 symlink 以 EEXIST 失敗。改為同樣判斷。
  - **連帶受益**：本機→本機同步（1.2 提到的 `sync_op`）走 `copy_file`，同樣不再寫穿 link。衝突對話框只顯示路徑、不讀 metadata，懸空 link 可正常顯示；「取代較舊」讀不到懸空 link 的時間時會改為詢問。
  - **未變更**：複製資料夾時，目的地若是指向資料夾的 symlink，仍會合併進 link 指向的資料夾——那是使用者自己放的 link，理由同 1.1 第 2 點。
  - **測試**：新增 `dangling_link_at_dest_is_a_conflict_not_a_way_out`（詢問時取消不寫任何檔、Overwrite 取代 link、同裝置移動也會詢問）與 `overwrite_replaces_a_link_instead_of_writing_through_it`（link 指向的檔案不被改寫、link 覆蓋懸空 link）；兩者在舊程式碼上皆失敗。

### 1.5 `[Medium-High]` PlantUML 預覽未啟用 `SANDBOX` 安全模式（`!include` 本機敏感檔案洩漏與 SSRF）
- **檔案位置**：
  - `src-tauri/src/commands/plantuml.rs:51-80`（`render_via_server`）
  - `src-tauri/src/commands/plantuml.rs:156-197`（`render_via_jar`）
- **問題說明**：
  - 當使用者在設定中啟用 PlantUML `jar` 模式並開啟 F3 Quick View 瀏覽目錄時，游標移過不可信的 `.puml` 或 `.md` 檔案即會自動觸發 `render_plantuml`。
  - `render_via_jar`（第 167–181 行）執行 `java -Djava.awt.headless=true -jar <jar> -pipe -tsvg -charset UTF-8`，未指定 `-DPLANTUML_SECURITY_PROFILE=SANDBOX`。PlantUML 預設的 `LEGACY` 模式允許 `!include /etc/passwd`、`!include ~/.ssh/id_rsa` 或 `!include http://169.254.169.254/...` 等前處理指令，會將本機機敏檔案內容直接渲染進預覽 SVG 中。
  - 此外，`render_via_server`（第 65–78 行）未驗證 `server` URL 是否為 `http://` 或 `https://`，且 `resp.into_string()` 未限制讀取上限。
- **修正建議**：
  - 在 `render_via_jar` 傳入 `"-DPLANTUML_SECURITY_PROFILE=SANDBOX"`。
  - 在 `render_via_server` 驗證 `server.starts_with("https://") || server.starts_with("http://")`，並使用 `.into_reader().take(4 * 1024 * 1024)` 限制回應大小。
- **處理結果**（Claude，2026-11-07）：jar 模式的部分成立，已修；server 模式的兩點已由 ureq 涵蓋，不需修。
  - **查證**（官方文件 plantuml.com/security，並以本機 `plantuml-mit-1.2026.9.jar` + Homebrew openjdk 實測）：
    - 未指定 profile 時預設為 `LEGACY`（文件：「full access to local files and full access to URL」）。實測 `!include <本機檔案>` 會把檔案內容原樣畫進 SVG——**成立**。Quick View 在游標移到 `.puml`／`.md` 時自動渲染，所以只要瀏覽到一個不可信的檔案就會觸發。
    - 同版本 LEGACY 下 `%getenv` 回傳空字串；`!include http://127.0.0.1:18765/...` 未發出請求（本機 HTTP 伺服器無紀錄）。文件說 LEGACY 可存取任意 URL；80/443 未實測（需對外連線）。
    - `SANDBOX`（文件：「cannot have any access to local files or URL」）：實測本機 include 被擋，一般圖與 jar 內建 stdlib（`!include <C4/C4_Context>`）照常渲染。
  - **已修**：`render_via_jar` 傳入 `-DPLANTUML_SECURITY_PROFILE=SANDBOX`（JVM system property，須在 `-jar` 之前）。
  - **代價**：圖中以絕對路徑 `!include` 本機檔案的寫法不再生效。相對路徑 include 原本就無法使用——`-pipe` 模式下相對路徑是以 app 行程的工作目錄（Finder 啟動時為 `/`）解析，而非檔案所在資料夾。
  - **不需修**（server 模式）：
    - URL scheme：ureq 2.12.1 在連線時只接受 `http`／`https`，其餘回傳 `UnknownScheme`；且 server URL 是使用者自己的設定。
    - 回應大小：ureq 2.12.1 的 `into_string` 內建 10 MB 上限（`INTO_STRING_LIMIT`），超過即回錯。
  - **測試**：新增 opt-in 測試 `jar_rendering_cannot_include_local_files`（設定 `YC_PLANTUML_JAR` 才執行，作法同既有的 `YC_TEST_SSHD_PORT` sshd 測試）：本機檔案內容不得出現在 SVG、一般圖與 C4 stdlib 仍可渲染。拿掉 SANDBOX 時失敗。

### 1.6 `[Medium]` IPC 路徑與外部工具參數驗證（`finish_repair`、`ffmpeg`、`ffprobe`、`stream_protocol`）
- **檔案位置**：
  - `src-tauri/src/commands/system.rs:124-241`（`probe_duration`、`repair_video`、`finish_repair`）
  - `src-tauri/src/commands/preview.rs:21-42`（`video_poster`）
  - `src-tauri/src/platform/linux/mod.rs:403-414`（`media_info`）
  - `src-tauri/src/stream_protocol.rs:102-117`
- **問題說明**：
  - `finish_repair(tmp, original, action)`（`commands/system.rs:208-241`）直接接受前端傳入的 `tmp` 與 `original` 執行 `remove_file` 或 `rename` 覆寫，未驗證 `tmp` 是否確實為同目錄下的 `.<stem>_fix.part.mp4` 暫存檔。
  - `ffmpeg` 與 `ffprobe` 直接將 `path` 傳入 `-i <path>`，未確認 `path.is_absolute()`（若傳入 `http://...`、`concat:...` 或 `-` 開頭字串，會被 `ffmpeg` 當成網路協定或旗標）。
  - `stream_protocol.rs`（macOS `stream://`）未如 Linux 的 `media_server.rs` 採用已註冊的單次隨機 token，而是直接依 URL 路徑讀取任意本機檔案。
- **修正建議**：
  - 在呼叫 `ffmpeg`/`ffprobe` 前檢查 `path.is_absolute()`。
  - 在 `finish_repair` 驗證 `tmp.parent() == original.parent()` 且檔名以 `_fix.part.mp4` 結尾。
- **處理結果**（Claude，2026-11-07）：前兩點以縱深防禦採納（成本極低）；`stream://` 不採納。嚴重度應下修——這些指令只由 app 自己的前端呼叫，而前端本來就能透過複製／移動／刪除操作處理任意路徑，因此三點都沒有讓攻擊者多得到能力。
  1. **`finish_repair`：已修。** 新增 `repair_work_file(original)`，`repair_video` 用它產生工作檔路徑；`finish_repair` 重新推導並要求 `tmp` 完全等於它，否則拒絕（比建議的「同資料夾＋後綴」更嚴格）。前端傳入的 `tmp` 一定是 `repair_video` 對同一 `original` 回傳的值，正常流程不受影響；能擋下前端 bug 把錯誤路徑 rename 蓋過原檔或刪除。
  2. **ffmpeg／ffprobe 路徑：已修。** 新增 `commands::media_input`，`media_info`、`video_poster`、`repair_video` 皆要求絕對路徑。以本機 ffmpeg 實測：相對路徑 `concat:a.txt|a.txt` 會被當成 concat protocol 讀取串接；同名絕對路徑則被當成一般檔案（No such file）。絕對路徑以 `/` 開頭，不可能是 protocol 名稱，也不可能是 ffprobe 的選項（ffprobe 的路徑是位置參數，前導 `-` 會被當成選項）。現行前端只會傳面板上的絕對路徑，此檢查屬保險。
  3. **`stream://` 不採用 token：不採納。** 自訂 scheme 只有 app 自己的 webview 能發請求；而 `asset://` 的 scope 本來就是 `**`（同樣能讀任意本機檔案），`stream://` 並未擴大能力，回應也刻意不帶 `Access-Control-Allow-Origin`。Linux 的 `media_server` 需要 token 是因為它是 `127.0.0.1` 上的 HTTP 伺服器，本機任何程式與使用者瀏覽器中的網頁都連得到——兩者的暴露面不同。
  - **測試**：新增 `finish_repair_only_touches_its_own_work_file`（其他檔案在三種 action 下皆被拒且未被動到、真正的工作檔可正常 save）與 `media_tools_get_absolute_paths_only`。

### 1.7 `[Medium]` SSH Keychain 密碼儲存鍵值未包含 `port`
- **檔案位置**：
  - `src-tauri/src/commands/config.rs:68-80`
- **問題說明**：
  - `ssh_saved_password`、`ssh_store_password`、`ssh_forget_password` 皆以 `format!("ssh://{username}@{host}")` 作為 Keychain / Secret Service 的帳號識別，忽略了 `port`。若同一台主機或 `127.0.0.1` 上有多個不同 port 的 SSH 服務（例如 `2222` 與 `2223`），密碼會互相覆蓋並送到錯誤的伺服器。
- **修正建議**：
  - 將 `port` 納入 key（例如 `port != 22` 時使用 `ssh://{username}@{host}:{port}`，查詢時若找不到可 fallback 至舊格式以維持向下相容）。
- **處理結果**（Claude，2026-11-07）：成立，且問題比描述的更大；已修。fallback 改為「有證據才轉移」。
  - **另一半問題**：前端 `ConnectDialog` 儲存與刪除 SSH 連線時同樣只以 (host, username) 判斷同一筆——存下 `:2223` 會把清單中的 `:2222` 換掉，刪除一筆會兩筆一起刪。也就是說原本根本存不了兩筆只差在 port 的連線。只改 Keychain 鍵值不夠。
  - **已修**：
    1. 新增 `keychain::ssh_account(user, host, port)`：port 22 維持舊格式 `ssh://user@host`（既有項目照常可用），其他 port 為 `ssh://user@host:port`；IPv6 主機加中括號（`[::1]:2222`），避免「`::1` + port 2222」與「主機 `::1:2222` + port 22」撞成同一鍵。三個 `ssh_*_password` 指令與前端呼叫皆帶入 port。
    2. 前端以 `sameSsh`（host + username + port）判斷同一筆連線，用於儲存與刪除。
    3. **不採用盲目 fallback**：舊鍵值不知道原屬哪個 port，找不到就讀舊鍵會讓「把 A 的密碼送給 B」再發生一次。改為：修正前清單每個 user@host 最多一筆，所以只有當目前清單中該 user@host **僅有這一筆**非 22 的連線時，舊鍵才確定屬於它——此時把密碼搬到新鍵並刪除舊鍵（一次性）。其餘情況（另有 port 22 或其他 port 的同 user@host 連線）不讀舊鍵，最多請使用者重新輸入一次密碼。
  - **測試**：新增 `ssh_keys_tell_ports_apart`（22 維持舊格式、不同 port 不同鍵、IPv6 不撞鍵）與 `legacy_key_is_adopted_only_by_its_sole_owner`（唯一擁有者才接手、多筆或 port 不符皆不接手）。實際讀寫 Keychain 的部分會碰到真實的系統鑰匙圈，未做自動化測試。

### 1.8 `[Medium]` `MarkdownPreview` 與 `QuickView` 的連結／SVG 拖放邊界防護
- **檔案位置**：
  - `src/components/MarkdownPreview.tsx:180-195`
  - `src/components/QuickView.tsx:465-470, 603, 629`
  - `src/components/Dialogs.tsx:204-212, 261-268, 418-428`
- **問題說明**：
  1. `MarkdownPreview.tsx` 僅攔截左鍵 `onClick`，未攔截中鍵點擊（`onAuxClick`）、拖曳連結（`onDragStart`）以及 Mermaid SVG 內的 `xlink:href`。
  2. `QuickView.tsx` 以 `URL.createObjectURL(new Blob([buf], { type: "image/svg+xml" }))` 預覽 SVG 時，第 603 與 629 行的 `<img>` 未設定 `draggable={false}`（對照 `FileIcon.tsx:104` 有設定），若使用者將圖片拖放至視窗可能觸發頂層導覽。
  3. `Dialogs.tsx` 中的 `NewFolderDialog`、`RenameDialog`、`CompressDialog` 未在前端驗證名稱是否包含 `/` 或 `..`（例如在 `CompressDialog` 輸入 `../out` 會將壓縮檔輸出至上層目錄）。
- **修正建議**：
  - 在 `MarkdownPreview.tsx` 攔截 `onAuxClick` 與 `onDragStart`，並同時檢查 `href` 與 `xlink:href`。
  - 在 `QuickView.tsx` 的預覽 `<img>` 加上 `draggable={false}`。
  - 在 `NewFolderDialog`、`RenameDialog`、`CompressDialog` 禁止名稱包含 `/`、`\0`、`.` 或 `..`。
- **處理結果**（Claude，2026-11-07）：三點描述的導覽都已被既有機制擋下；只做兩項低成本加固，嚴重度應下修為 Low。
  - **查證**（Tauri 2.12.5／tauri-runtime-wry 2.12.3／wry 0.57.1 原始碼；app 未設定 `on_navigation`、`on_new_window`，也未關閉 `dragDropEnabled`）：
    - **中鍵點擊**：中鍵開連結是「開新視窗」。macOS 上 wry 的 `createWebViewWithConfiguration` 在沒有 new-window handler 時回傳 `None`（拒絕）；Linux 上沒有 handler 就不連接 `create` 訊號，WebKitGTK 不建立視窗。主視窗不會被導覽。
    - **拖放連結／圖片**：macOS 上 Tauri 的 drag-drop handler 對每個拖放事件都回傳 `true`，WebKit 收不到 drop，不會導覽。Linux 上只有 uri-list 類型的拖放會被吃掉（連結與圖片拖曳屬於此類），其他類型會交還 WebKit——保證較弱。
    - **Mermaid 的 `xlink:href`**：`securityLevel: "strict"` 會停用 click／連結功能；就算出現 SVG `<a>`，`closest("a")` 也會找到它並 `preventDefault`（`xlink:href` 讀不到 `href`，因此不開啟任何東西）。
    - **對話框名稱**：新增資料夾與改名的後端指令都會先呼叫 `transfer::check_name`，已拒絕空字串、`.`、`..`、`/`、NUL——**不成立**。壓縮檔名是使用者自己輸入的目的地（輸入 `../out` 即是要放到上層），不是不可信輸入，不屬資安問題。
  - **為何仍加固**：Quick View 的 SVG 以 `blob:` URL 顯示，而 blob URL 與 app 同源；若主視窗被導覽到它，SVG 會成為 app 同源文件。目前有兩層保護擋住（上述 Tauri 預設行為，加上主視窗 CSP 無 `'unsafe-inline'`，inline script 不會執行），但這條鏈後果嚴重，移除拖曳來源只需幾行。
  - **已改**：
    1. `QuickView.tsx` 的圖片與影片截圖 `<img>` 加上 `draggable={false}`。
    2. `MarkdownPreview.tsx`：中鍵點擊連結與左鍵相同處理（`https?` 交給系統瀏覽器，其餘不動作），不再交由 webview 開新視窗；連結與圖片不能拖曳，選取的文字仍可拖曳。
  - **未驗證**：這兩項是前端事件處理，專案沒有前端測試框架，也未在 app 中實際操作驗證；`npx tsc -b` 通過。

---

## 二、效能與記憶體洩漏問題（Performance & Memory Leak Issues）

### 2.1 `[High]` `fs/local.rs` 列出目錄時每個項目多執行一次 `stat(2)` 系統呼叫
- **檔案位置**：
  - `src-tauri/src/fs/local.rs:18-26`
  - `src-tauri/src/ops/sync.rs:41-47`
  - `src-tauri/src/ops/transfer.rs:10-22`
  - `src-tauri/src/ops/archive.rs:61, 111-113`
- **問題說明**：
  - 在 `src-tauri/src/fs/local.rs:18-26` 中：
    ```rust
    let meta = if is_symlink {
        std::fs::metadata(&path).or_else(|_| entry.metadata())
    } else {
        entry.metadata()
    };
    let (is_dir, size, modified) = match meta {
        Ok(m) => (path.is_dir(), m.len(), m.modified().ok().map(epoch_millis)),
        Err(_) => (false, 0, None),
    };
    ```
    第 24 行寫成了 `path.is_dir()` 而非 `m.is_dir()`。因為 `m` 已經是目標檔案的 `Metadata`，呼叫 `path.is_dir()` 會對目錄中**每一個檔案再發起第二次 `stat(2)` 系統呼叫**。在大型目錄或掛載的 SMB/GVfs 網路資料夾上，這會使 metadata I/O 時間直接翻倍。
  - 同樣地，`ops/sync.rs:41-47`、`ops/transfer.rs:11-20` 與 `ops/archive.rs:61, 111-112` 在遞迴走訪時分別呼叫了 `path.is_symlink()`、`path.is_dir()` 與 `path.metadata()`（每個檔案最多 3 次 syscall），而未利用 `entry.file_type()`（在 macOS/Linux 上直接讀取 `readdir` 的 `d_type`，0 次額外 syscall）。
- **修正建議**：
  - 將 `src-tauri/src/fs/local.rs:24` 的 `path.is_dir()` 改為 `m.is_dir()`。
  - 在 `ops/` 各遞迴走訪函式中改用 `entry.file_type()` 與單次 `entry.metadata()`。
- **處理結果**（Claude，2026-11-07）：成立，已修。
  - **查證**：`local.rs` 中 `m` 對一般項目與有效 symlink 即為目標的 metadata，對懸空 symlink 為 link 本身（兩者 `is_dir()` 皆為 false），因此 `m.is_dir()` 與 `path.is_dir()` 結果完全相同，後者只是多一次 `stat`。`ops/` 的走訪函式也確實對每個檔案重複查詢。
  - **已修**（每個一般檔案的 syscall 數）：
    - `fs/local.rs::list_dir`：`path.is_dir()` → `m.is_dir()`（2 → 1；有效 symlink 3 → 2）。
    - `ops/sync.rs::collect_local`：改用 `entry.file_type()`（macOS/Linux 上多半直接取自 readdir 的 `d_type`）與 `entry.metadata()`（3 → 1）。
    - `ops/transfer.rs::total_bytes`：單次 `symlink_metadata()`，只有在要跟隨 symlink 時才再 `stat` 目標（3 → 1）。
    - `ops/transfer.rs::copy_dir`：判斷子項是否為資料夾改用 `entry.file_type()`（2 → 0）。
    - `ops/archive.rs::collect_tree` 與 `compress_op` 的總量計算：各改為單次 `symlink_metadata()`（2 → 1）。
  - **語意保持不變**：symlink 一律不走進去、只有開啟「跟隨 symlink」時才計入檔案 link 的大小、懸空 link 不計。新增 `listing_follows_links_for_type_and_size`、`local_listing_skips_links`、`totals_and_folder_copies_treat_links_as_before` 三個測試涵蓋這些規則，並在**修改前**與**修改後**的程式碼上都執行過，兩者皆通過（全部 98 個測試）。
  - **未量測**：syscall 減少是依程式碼逐一計算，未以 dtruss／strace 實測，也未在網路磁碟上量測實際耗時。

### 2.2 `[High]` SFTP 目錄同步上傳時的嚴重 N+1 網路來回（`ensure_remote_parents`）
- **檔案位置**：
  - `src-tauri/src/ops/sync.rs:176-186, 204-216, 238-241`
  - `src-tauri/src/ops/remote.rs:11-29`
- **問題說明**：
  1. `ops/sync.rs::ensure_remote_parents`（第 176–186 行）在同步上傳**每一個檔案**時都會被呼叫（第 239 行），並從根部逐層對每個父目錄片段呼叫 `conn.exists(&dir)`，完全沒有記憶已確認存在的目錄。若同步一個含 500 個檔案、深度 4 層的專案目錄，會產生 **2,000 次以上重複的 SFTP `LSTAT` 網路來回**。
  2. `ops/sync.rs:204-216` 在計算 `ctx.total_bytes` 時，若來源為遠端（`Some(_) => 0`）直接給 `0`，導致遠端對本機（`Rtl`）同步時沒有位元組進度條。
  3. `ops/remote.rs::remote_keep_both`（第 11–29 行）以 `for n in 2..10_000` 逐一透過網路呼叫 `conn.exists(&candidate)`。
- **修正建議**：
  - 在 `sync_op` 維護一個 `HashSet<PathBuf>` 傳入 `ensure_remote_parents`，確保每個遠端父目錄最多只檢查／建立一次。
  - 在 `SyncItem` 結構中加入 `size: u64` 欄位以支援遠端同步進度計算。
- **處理結果**（Claude，2026-11-07）：前兩點成立，已修；第三點不採納。
  1. **`ensure_remote_parents` 重複檢查：已修。** 新增純函式 `unchecked_parents`，以整次 sync 共用的 `HashSet<PathBuf>` 記錄已確認的遠端資料夾，只回傳尚未確認者；每個遠端資料夾在一次同步中最多檢查／建立一次。原本 500 個檔案、深 4 層會做 2,000 次 `exists`（每次為 LSTAT，遇 symlink 再加 STAT），現在為 4 次——測試 `each_remote_folder_is_checked_once_per_sync` 直接驗證此數字。
  2. **遠端來源沒有進度：已修。** 比對結果的每一列本來就帶有兩側的大小，因此 `SyncItem` 新增選填的 `size`（`#[serde(default)]`），前端 `SyncDialog` 傳入來源側的大小；遠端來源的總量改用它，不需額外的網路往返。本機來源維持當場 `stat`（資料較新）。
  3. **`remote_keep_both` 逐一探測：不採納。** 只在使用者對衝突選「保留兩者」時呼叫，從 `name 2` 起探測，通常第一個就空著，只花一次往返；改成先列出父資料夾需要一次 READDIR 加上 `list()` 對 symlink 的 STAT，大資料夾反而更慢。
  - **未驗證**：未對真實 SFTP 伺服器實測（既有的 sshd 測試為 opt-in，本次未執行）；`ensure_remote_parents` 的邏輯以純函式的單元測試涵蓋，進度總量的前端傳遞以 `tsc` 型別檢查涵蓋。

### 2.3 `[High]` 瀏覽壓縮檔內部時，每次切換子目錄都從頭完整解壓／掃描整個壓縮檔
- **檔案位置**：
  - `src-tauri/src/fs/archive_fs.rs:51-173, 176-216, 228-266`
- **問題說明**：
  - 每次在壓縮檔內進入子目錄呼叫 `list_dir` 時，`for_each_entry`（第 51–105 行）都會重新開啟檔案並從頭掃描：
    - 對 `.tar.gz` / `.tgz`，每進一層目錄就會**從第 0 byte 重新 Gzip 解壓縮整個壓縮檔**。
    - 對 `.zip`，`for i in 0..zip.len() { let entry = zip.by_index_raw(i)?; ... }` 會對每個項目執行一次 `seek` + 讀取 local header（10,000 個檔案即 10,000 次 I/O）。
  - 另外在 `extract_entries`（第 235 行）的內層迴圈中，每次比對都執行 `name.starts_with(&format!("{sel}/"))`，對每個 `(entry, selected)` 配對重複配置 `String`。
- **修正建議**：
  - 在 `AppState` 中使用小型 LRU（例如容量 4，以 `(PathBuf, modified, len)` 為 key）快取壓縮檔的扁平項目索引 `Vec<ArchiveItem>`，讓壓縮檔內的目錄切換直接在記憶體中完成。
  - 在 `extract_entries` 迴圈外預先計算 `let prefix = format!("{sel}/");`。
- **處理結果**（Claude，2026-11-07）：成立，已依建議修正。
  - **查證**：
    - tar／tar.gz：`tar` crate 只能循序讀取，每列一層目錄就把整個檔案從頭讀一遍；`.tar.gz` 還要從頭解壓縮——成立。
    - zip：在鎖定的 zip 9.0.5 中，`by_index_raw` → `find_content` → `data_start` 會對每個項目 seek 並讀取 local header——成立。同版本另有 `by_index_data(&self, i)`，只讀已載入記憶體的中央目錄，並提供 `name`／`size`／`last_modified`／`is_dir`。
  - **已修**：
    1. zip 列目錄與 F5 複製的總量預掃改用 `by_index_data`，不再逐項讀 local header。
    2. 新增扁平索引快取：以現有的 `lru::Lru`（容量 4）快取每個壓縮檔的 `Vec<Item>`，鍵值為（路徑、大小、修改時間）——檔案一改寫即換鍵，不會讀到過期索引。`list` 只在第一次掃描，之後切換目錄只需一次 `stat` 比對鍵值。超過 50 萬個項目的壓縮檔不放入快取，避免記憶體無上限成長。
    3. `extract_entries` 的 `format!("{sel}/")` 移到迴圈外預先計算。
  - **測試**：新增 `listing_reuses_the_index_until_the_archive_changes`——把壓縮檔換成大小與修改時間相同、內容不同的 tar，列出的仍是舊內容（證明未重新掃描）；修改時間改變後列出新內容（證明會失效）。全部 100 個測試通過。
  - **未處理／未量測**：Quick View 預覽 tar 內的檔案（`read_entry`）仍需循序讀到該項目為止，tar 格式本身沒有索引；未對大型壓縮檔量測實際耗時。

### 2.4 `[High]` `Panel.tsx` 與 `QuickView.tsx` 的 Zustand 訂閱粒度過粗，每次移動游標都重繪整個面板樹
- **檔案位置**：
  - `src/components/Panel.tsx:39, 78, 134, 137, 311, 378`
  - `src/components/QuickView.tsx:285`
  - `src/store/panels.ts:117-132, 378-422`
- **問題說明**：
  - 在 `panels.ts` 中，每次方向鍵移動游標（`moveCursor`）、空白鍵標記（`toggleMark`）或拖曳欄寬（`setColWidth`），都會複製並產生新的 `TabState` 與 `s.panels[side]`。
  - 在 `Panel.tsx` 中，`TabStrip`（第 78 行）直接訂閱 `usePanels((s) => s.panels[side])`，而 `PanelBar`（第 134 行）、`PathBar`（第 311 行）、`StatusLine`（第 378 行）都接收整個 `tab: TabState` 且未使用 `React.memo`。
  - 結果：**使用者每按一下 `ArrowDown`/`ArrowUp`，`Panel`、`TabStrip`、`PanelBar`（包含磁碟列表 `.filter().sort()` 與最愛目錄比對）、`PathBar` 與 `StatusLine` 全部都會重新渲染**。
  - 此外，`PanelBar`（第 137 行）與 `QuickView`（第 285 行）皆訂閱了整個 `useSettings((s) => s.config)`，任何設定或面板狀態儲存都會觸發重繪。
- **修正建議**：
  - 將 `TabStrip`、`PanelBar`、`PathBar`、`StatusLine` 以 `React.memo` 包裝，並只傳入或選取所需的原始型別欄位（如 `tab.path`、`tab.remoteLabel`、`tab.location`、`tab.filter`）。
  - 將 `useSettings((s) => s.config)` 改為細粒度 selector（`s.config?.favorites`、`s.config?.qv_autoplay ?? false`）。
- **處理結果**（Claude，2026-11-07）：現象存在但成本很低，嚴重度應下修為 Low；採納細粒度 selector，不採納 `React.memo` 重構。
  - **查證**：
    - 移動游標時確實會重繪 `Panel`、`TabStrip`、`PanelBar`、`PathBar`、`StatusLine`，但都很輕：`TabStrip` 只有幾個分頁按鈕；`PathBar` 只有一個 input；`StatusLine` 的 O(n) 計算已包在 `useMemo`，依賴不含游標；`PanelBar` 的磁碟清單 `vols` 只在選單打開時才抓取，平常是空陣列，`.filter().sort()` 幾乎沒有成本。
    - 真正重的檔案列表已處理好：`Row`／`Tile` 為 `memo`，移動游標只重繪變動的兩列；`visibleEntries` 以 `entries` 陣列為鍵做 WeakMap 快取，游標移動直接命中。
    - 另一側面板不會重繪：`updateTab` 保留另一側的物件參照，其 selector 拿到同一參照。
  - **已改**（建議第二點）：`PanelBar` 只訂閱 `s.config?.favorites`，`QuickView` 只訂閱 `s.config?.qv_autoplay ?? false`。settings store 以 spread 合併 patch，未變動欄位保留參照，因此每次導覽儲存 `recent_dirs`／面板狀態時，這兩個元件不再因此重繪。
  - **不採納**（建議第一點）：`TabStrip` 自己就訂閱 `s.panels[side]`，游標一動其 props／state 就變，`React.memo` 擋不住；要真正避免得改寫成回傳分頁標籤的 selector 搭配淺比較，改動量不小，省下的只是幾個輕量元件的微秒級重繪。
  - **未驗證**：未以 React Profiler 實測重繪時間；`npx tsc -b` 通過。

### 2.5 `[High]` 每次切換本機目錄都觸發兩次 `settings.save()` 與兩次設定檔寫入 + 主執行緒同步 I/O
- **檔案位置**：
  - `src/store/panels.ts:301-314`
  - `src-tauri/src/commands/config.rs:36-80`
  - `src-tauri/src/commands/smb.rs:87-104`
  - `src-tauri/src/config.rs:194-204`
- **問題說明**：
  1. 在 `src/store/panels.ts` 的 `navigate()`（第 301–314 行）中，切換至本機目錄時先呼叫了一次 `settings.save({ recent_dirs })`，緊接著下一行呼叫 `persistPanels()` 又執行第二次 `settings.save({ left_view, ... })`。這導致每次點擊進入資料夾都會更新兩次 Zustand `config`、發送兩次 `update_config` IPC、寫入兩次 `config.toml`，並廣播兩次 `"config-patched"` 事件。
  2. 後端的 `update_config`（`commands/config.rs:57`）與 6 個 Keychain 密碼讀寫指令（`ssh_*_password`、`smb_*_password`）皆為同步 `fn`，在 **Tauri 主執行緒（UI thread）** 上直接執行檔案寫入與 macOS Keychain / Linux D-Bus Secret Service 呼叫。
  3. `Config::save`（`config.rs:202`）直接對 `config.toml` 呼叫 `std::fs::write` 原地截斷寫入，若寫入途中異常中斷可能留下損毀的設定檔。
- **修正建議**：
  - 在 `panels.ts` 將 `recent_dirs` 合併至 `persistPanels()` 中，每次導覽只呼叫一次 `settings.save()`。
  - 將 `update_config` 與 6 個 `*_password` Tauri 指令改為 `async fn` + `blocking(...)`。
  - 在 `Config::save` 先寫入暫存檔再 `rename` 原子替換，且不在持有 `state.config` Mutex 鎖的狀態下執行磁碟 I/O。
- **處理結果**（Claude，2026-11-07）：三點皆成立，已修；唯「寫檔時不持有鎖」不採納。
  1. **每次導覽存兩次：已修。** `persistPanels(extra)` 新增選填參數，`navigate` 把 `recent_dirs` 併入同一次 `save()`——每次導覽一次 IPC、一次寫檔、一次 `config-patched` 廣播。
  2. **同步指令跑在主執行緒：已修。** Tauri 2 未標 `async` 的指令在主執行緒執行。`update_config` 與 6 個 `ssh_*`／`smb_*_password` 改為 `async fn` + `blocking(...)`；Keychain／Secret Service 跳出解鎖提示時 UI 不再卡住。`update_config` 改以 `app.state::<AppState>()` 在背景執行緒取得狀態。前端呼叫介面不變。
  3. **原地截斷寫入：已修。** 新增 `config::write_replacing`：寫入同資料夾的 `config.toml.tmp`、`sync_all`、再 `rename` 覆蓋；失敗時刪除暫存檔。寫到一半當機或磁碟滿時，舊檔保持完整（原本會留下截斷的檔案，`load` 解析失敗就整份回到預設值）。
  - **不採納**：「不在持有 `state.config` 鎖時寫檔」。`apply_patch` 刻意持有鎖直到寫完並送出事件，確保寫檔與 `config-patched` 的順序一致；若先放鎖再寫，兩個併發 patch 可能讓較舊的內容最後落地、蓋掉較新的。寫檔已移到背景執行緒，持有鎖只會讓其他背景工作等待約數毫秒。
  - **維持原樣**：關窗時（`CloseRequested`）的儲存留在主執行緒——關窗前必須寫完；`fkeys_set` 只在使用者切換設定時呼叫。
  - **測試**：新增 `config_writes_replace_the_file_whole`（覆蓋既有檔案、不留暫存檔；rename 失敗時清除暫存檔並回傳錯誤、原有內容不受影響）。全部 101 個測試通過、`tsc` 通過。未在 app 中實際操作驗證 Keychain 解鎖情境。

### 2.6 `[High]` `AudioPreview.tsx` 在音訊暫停或未播放時仍持續以 60fps 執行 `requestAnimationFrame`
- **檔案位置**：
  - `src/components/AudioPreview.tsx:197-210`
- **問題說明**：
  - 只要在 Quick View 開啟音訊檔案預覽，`useEffect` 就會啟動無限循環的 `requestAnimationFrame(tick)`（每秒 60 次），即使音訊處於暫停或根本尚未播放，仍持續輪詢 `audioRef.current` 並更新 DOM。
- **修正建議**：
  - 改在 `<audio>` 的 `onPlay` 事件啟動 `requestAnimationFrame`，於 `onPause` / `onEnded` 取消 rAF，並在 `onTimeUpdate` / `onSeeked` 更新單次播放頭位置。

### 2.7 `[High]` `PdfPreview.tsx` 快速翻頁時未取消進行中的 `pdfPage.render()` 導致 Canvas 衝突報錯
- **檔案位置**：
  - `src/components/PdfPreview.tsx:47-80`
- **問題說明**：
  - 當使用者快速連按上一頁／下一頁時，前一次 `useEffect` 的 cleanup 只設定了 `gone = true`，並未對 `pdfPage.render(...)` 回傳的 `RenderTask` 呼叫 `.cancel()`。當新頁碼的 `useEffect` 立即在同一個 `canvasRef.current` 上呼叫 `pdfPage.render()` 時，PDF.js 會拋出 `Error: Cannot use the same canvas during multiple render() operations`，並被新 effect 的 `catch` 捕獲而顯示錯誤訊息。
- **修正建議**：
  - 保存 `const renderTask = pdfPage.render(...)`，在 cleanup 中呼叫 `renderTask.cancel()`，並在 `catch` 中忽略 `RenderingCancelledException`。

### 2.8 `[Medium-High]` `App.tsx` 在 `React.StrictMode` 下洩漏 `fs:changed` 事件監聽器
- **檔案位置**：
  - `src/App.tsx:63-75`
- **問題說明**：
  - 第 63–75 行的非同步初始化 IIFE 在 `await useSettings.getState().load()` 與 `await usePanels.getState().init()` 之後才註冊 `unlisten = await listen("fs:changed", ...)`，但缺少 `let gone = false` 保護（對照第 78–86 行的 `initFileDrop` 有加上 `gone` 檢查）。在 `React.StrictMode` 或元件提前卸載時，第一次掛載的 cleanup 執行時 `unlisten` 仍為 `undefined`，導致該監聽器永久洩漏，每次檔案變動都會觸發重複的 `refresh()`。
- **修正建議**：
  - 加入 `let gone = false;` 旗標，若 `listen()` 完成時 `gone` 為 `true` 則立即呼叫回傳的 unlisten 函式。

### 2.9 `[Medium-High]` `panels.ts` 中 `navigate` 與 `refresh` 的非同步競態條件（Stale Directory Listing）
- **檔案位置**：
  - `src/store/panels.ts:276-356`
- **問題說明**：
  - `navigate` 與 `refresh` 僅檢查 `t.id === tab.id`，未追蹤請求世代（request generation ID）。當背景 `fs:changed` 觸發 `refresh` 或慢速 SFTP 目錄正在讀取時，若使用者同時導覽至其他目錄，較早發出的 `listDir` 若較晚回傳，會將舊目錄的檔案列表覆蓋到新目錄上；且 `refresh` 第 350 行會使用舊的 `tab.path` 重新呼叫 `ipc.watchDir(side, tab.path)`。
- **修正建議**：
  - 為每個分頁維護遞增的請求序號（或在 `await ipc.listDir` 後比對當前分頁的 `path` 與 `location` 是否仍一致）。

### 2.10 `[Medium]` 其他前後端效能問題
- **`FindDialog.tsx` 啟動與關閉競態**（`src/components/FindDialog.tsx:66-117`）：`findIdRef.current` 在 `await ipc.startFind(...)` 解析後才賦值。若在 `await` 期間關閉對話框或連按兩次搜尋，無法取消已送出的後端搜尋任務。
- **`FileTable.tsx` 拖曳欄寬重繪所有列**（`src/components/FileTable.tsx:53-69, 131-132, 250-272`）：`ColResize` 在每次 `mousemove` 都呼叫 `setColWidth` 更新 `cols` prop，使所有可見 `<Row>` 的 `React.memo` 失效。可改用 CSS 變數 `--grid-cols` 於拖曳時直接更新 DOM style，放開滑鼠時再寫回 store（如同 `App.tsx:26-44` 的 `--split`）。
- **`MultiRenameDialog.tsx` 預覽列表未虛擬化**（`src/components/MultiRenameDialog.tsx:157-173`）：選取上千個檔案進行批次改名時，未使用 `@tanstack/react-virtual`，每次修改規則都會直接渲染所有 DOM 節點。
- **`QuickView.tsx` 影片截圖多餘的 Base64 編碼**（`src/components/QuickView.tsx:352-363`）：`failVideo` 將 `videoPoster` 回傳的 `ArrayBuffer` 轉為字串再 `btoa` 成 Base64 Data URL，可直接改用 `URL.createObjectURL(new Blob([buf], { type: "image/jpeg" }))`。
- **`repair_video` 的 `stderr` Pipe Buffer 死結風險**（`src-tauri/src/commands/system.rs:163-194`）：同時將 `ffmpeg` 的 `stdout` 與 `stderr` 設為 `Stdio::piped()`，但主迴圈先同步讀完 `stdout` 才由 `wait_with_output()` 讀取 `stderr`。若損毀影片輸出超過 64KB 的 `stderr` 錯誤訊息，OS pipe buffer 滿載會導致 `ffmpeg` 與 Rust 雙向死結。
- **SFTP 內容搜尋緩衝 64MB 且每檔重新配置 1MB Buffer**（`src-tauri/src/commands/find.rs:83-129`）：`scan_reader` 每掃描一個檔案就配置一次 `vec![0u8; 1MB]`，建議將 buffer 放在 `Search` 結構體中重複使用；SFTP 搜尋建議改為串流讀取並檢查 `cancelled()`。
- **磁碟列表 `statfs_timeout` 循序等待、Linux `gsettings` 重複呼叫、多餘的 Tokio Runtime**：
  - `src-tauri/src/fs/local.rs:83-90`：對每個掛載點循序執行 800ms timeout 的 `statfs_timeout`，若有多個無回應的網路磁碟會累加延遲，建議改為平行檢查。
  - `src-tauri/src/platform/linux/mod.rs:453-486`：Linux 每次圖示快取未命中都會啟動 `gsettings` 子行程查詢 `icon-theme`，建議以 `OnceLock` 快取。
  - `src-tauri/src/fs/sftp.rs:20-23`：透過 `OnceLock` 額外建立了一個 multi-thread `tokio::runtime::Runtime`，可直接改用 Tauri 內建的 `tauri::async_runtime::block_on`。

---

## 三、多餘／重複的程式碼（Redundant & Duplicate Code）

### 3.1 前端與後端各自維護一份 PlantUML LRU 快取（且前端快取未含設定值 key）
- **檔案位置**：
  - `src/components/MarkdownPreview.tsx:55-93`（`plantumlCache`）
  - `src-tauri/src/state.rs:71`、`src-tauri/src/commands/plantuml.rs:217-241`（`plantuml_cache`）
- **說明**：前後端各實作了一個容量 64 的 PlantUML SVG LRU 快取。且前端 `plantumlCache` 只以 `code` 為 key，未包含 `plantuml_mode`、`plantuml_server`、`plantuml_jar`、`plantuml_java`；當使用者在偏好設定修改 PlantUML 設定後，前端會直接命中舊快取而不重新請求後端。
- **建議**：移除前端 `MarkdownPreview.tsx` 的 `plantumlCache`，統一由後端 `plantuml_cache` 快取。

### 3.2 `media_server` 為 Linux 專用卻在 macOS 一併編譯與註冊
- **檔案位置**：
  - `src-tauri/src/lib.rs:10, 162`
  - `src-tauri/src/media_server.rs:1-407`
  - `src-tauri/tauri.conf.json:27`
  - `src/components/QuickView.tsx:409-411, 434-439`
- **說明**：`media_server.rs` 是為了處理 Linux WebKitGTK/GStreamer 不對自訂協定發送 `Range` 請求而設的 localhost HTTP server；macOS 固定使用 `stream://`（`isMac` 分支），從不呼叫 `ipc.mediaUrl`。
- **建議**：可對 `media_server` 加上條件編譯，並僅在 `tauri.linux.conf.json` 的 `media-src` 開放 `http://127.0.0.1:*`。

### 3.3 Linux GVfs `smb-share:` 解析與路徑取得重複（且一處漏做百分比解碼）
- **檔案位置**：
  - `src-tauri/src/fs/local.rs:148-153, 257-269`（`gvfs_mounts`、`gvfs_display_name`）
  - `src-tauri/src/platform/linux/smb.rs:12-36`（`gvfs_dir`、`parse_smb_dir`）
- **說明**：兩處重複實作了 `$XDG_RUNTIME_DIR/gvfs` 路徑推導與 `smb-share:server=...,share=...` 字串解析；而且 `linux/smb.rs::parse_smb_dir` 有呼叫 `urlencoding::decode`，`local.rs::gvfs_display_name` 卻漏掉，導致含空白或非 ASCII 字元的 SMB 分享在磁碟選單顯示為 `%20` 編碼字串。
- **建議**：共用 `platform::smb` 中的 `gvfs_dir` 與 `parse_smb_dir`。

### 3.4 `fs/archive_fs.rs` 與 `ops/archive.rs` 重複實作 Zip／Tar 解壓縮迴圈
- **檔案位置**：
  - `src-tauri/src/fs/archive_fs.rs:268-364`（`extract_entries`）
  - `src-tauri/src/ops/archive.rs:366-438, 475-506`（`extract_zip`、`extract_tar`）
- **說明**：兩處各自維護一套 Zip 與 Tar/TarGz 解壓縮與進度回報邏輯，導致 `archive_fs::extract_entries` 漏掉了 Unix 權限還原、symlink 處理與 `create_dir_inside` 安全防護。
- **建議**：將解壓縮邏輯統一收斂至 `ops/archive.rs`，透過參數或閉包支援「完整解壓縮」與「指定項目解壓縮」。

### 3.5 `SettingsDialog.tsx` 中已無作用的 `READY` 集合與 `disabled` 判斷
- **檔案位置**：
  - `src/components/SettingsDialog.tsx:22-23, 74-76`
- **說明**：`SECTIONS` 與 `READY` 皆包含相同的 9 個分頁名稱，`disabled={!READY.has(s)}` 與 `t("status.comingSoon")` 為所有設定頁實作完成前的殘留程式碼。
- **建議**：移除 `READY` 常數與相關判斷。

### 3.6 `QuickView.tsx` 與 `CompareDialog.tsx` 重複的二進位檔偵測邏輯（且 `CompareDialog` 未支援 UTF-16）
- **檔案位置**：
  - `src/components/CompareDialog.tsx:23-25, 146-156`
  - `src/components/QuickView.tsx:475-485`
- **說明**：兩者皆掃描前 8,192 bytes 是否含 `0x00`，但 `QuickView.tsx` 會先檢查 UTF-16 LE/BE BOM（`0xFF 0xFE` / `0xFE 0xFF`）並以對應編碼解碼，`CompareDialog.tsx` 則一律視為二進位檔而無法比對 UTF-16 文字檔。
- **建議**：抽取共用 `decodeTextBuffer(buf: ArrayBuffer): string | null` 至 `src/lib/`。

### 3.7 其他可整併的重複工具函式與內部匯出
- **後端重複邏輯**：
  - 逐位元組比對檔案：`src-tauri/src/ops/sync.rs:91-113`（`files_equal`）與 `src-tauri/src/ops/transfer.rs:212-240`（`verify_copy`）。
  - 衝突時自動產生遞增檔名：`src-tauri/src/ops/queue.rs:272-291`（`keep_both_path`）與 `src-tauri/src/ops/remote.rs:11-29`（`remote_keep_both`）。
  - Linux `file://` URI 百分比編碼：`src-tauri/src/commands/drag.rs:19-30`（`gtk_uri_path`）與 `src-tauri/src/platform/linux/mod.rs:100-107`（`file_uri`）。
  - 本機檔案或資料夾移除片段（`if path.is_dir() && !path.is_symlink() { remove_dir_all } else { remove_file }`）：重複出現於 `ops/delete.rs:21-25`、`ops/remote.rs:99-103`、`ops/remote.rs:237-242`、`ops/transfer.rs:93-102`。
  - macOS `NSString` 建立與視窗預設尺寸常數：`src-tauri/src/dock.rs:27-30` vs `src-tauri/src/icons.rs:46-49`；`src-tauri/src/lib.rs:27-38, 88` vs `src-tauri/src/commands/system.rs:26, 30-31`。
- **前端重複邏輯與多餘 `export`**：
  - 「以其他程式開啟」選單與讀取邏輯重複於 `src/components/ContextMenu.tsx:77-94, 172-185` 與 `src/components/QuickView.tsx:135-196`（`OpenWithButton`，且後者漏了 `Escape` 關閉支援）。
  - `src/components/MultiRenameDialog.tsx:49-55` 手寫 `p.slice(0, p.lastIndexOf("/") + 1)`，可直接改用 `src/lib/path.ts` 的 `parentPath` 與 `joinPath`。
  - `src/styles/tokens.css:97-158` 中 `:root[data-theme="light"]` 與 `@media (prefers-color-scheme: light)` 重複定義了完全相同的 26 個 CSS 變數。
  - 僅在模組內部使用卻標註 `export` 的符號：
    - `src/components/MarkdownPreview.tsx:61`（`renderPlantumlBlocks`）
    - `src/components/icons.tsx:4-5`（`ICON_SIZE`、`ICON_STROKE`）
    - `src/keymap/keymap.ts:8, 66, 95`（`defaultKeymap`、`finderDelta`、`PRESETS`）
    - `src/store/panels.ts:35-37`（`DEFAULT_COLS`、`COL_MIN`、`COL_MAX`）
