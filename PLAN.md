# Your-Commander 實作計畫

## Context

全新專案(repo 只有 AGENTS.md)。目標:macOS 上類 Total Commander 的雙面板檔案管理工具,功能含基本檔案操作、壓縮/解壓縮(zip/7z/tar/gz/tgz)、SMB(掛載後與 Finder 共享)、SSH/SFTP(使用 ~/.ssh 設定與金鑰)。

**已確認的決策:**
- 技術棧:**Rust + egui (eframe)**
- 散佈:個人使用、不簽章 → 無 Sandbox 限制,可自由存取檔案系統、`~/.ssh`、掛載 SMB
- 7z:第一版**只做解壓**(`sevenz-rust2` 純 Rust),壓縮延後
- 選取模式:**Mac 風格**(Cmd+點擊多選、Shift 範圍選、空白鍵 QuickLook)
- Tabs:第一版不做,但 Panel 狀態物件化以預留擴充
- UI 語言:**英文 + i18n 架構**(字串抽出,預設英文、附繁中翻譯)

**可行性結論:** 無不可行功能。SMB 用系統掛載(`/Volumes`)自然與 Finder 共享;SFTP 僅在 App 內瀏覽(不經 macFUSE/File Provider),符合規格。已知限制:egui 無法做「拖出到 Finder」的 drag source;Mac F5~F8 需 Fn,所有操作需有 Cmd 系等價快捷鍵。

## 技術選型(crates)

| 用途 | Crate |
|---|---|
| GUI | `eframe`/`egui` + `egui_extras`(Table 虛擬化列表) |
| 壓縮 | `zip`、`tar`、`flate2`(gz/tgz)、`sevenz-rust2`(7z 解壓) |
| 垃圾筒 | `trash`(macOS 走 NSFileManager trash) |
| SSH/SFTP | `russh` + `russh-sftp`(純 Rust)、`russh-keys`;`ssh2-config` 解析 ~/.ssh/config |
| Keychain | `security-framework`(存 SMB/SSH 密碼) |
| SMB 掛載 | 呼叫 `/sbin/mount_smbfs`(掛到 /Volumes,Finder 共享);`diskutil unmount` 卸載 |
| 非同步/背景 | `tokio`(russh 需要;檔案操作佇列用 worker thread + channel) |
| 目錄監看 | `notify`(自動刷新面板) |
| 設定持久化 | `serde` + `toml`,存 `~/Library/Application Support/your-commander/` |
| i18n | `rust-i18n`(字串抽離,en 預設 + zh-TW) |
| 開啟檔案 | `open` crate(預設 App 開啟);`qlmanage -p` 做 QuickLook 預覽 |

## 架構

```
src/
├── main.rs            # eframe 進入點
├── app.rs             # App 狀態根:兩個 PanelState、操作佇列、對話框狀態
├── config.rs          # 設定載入/儲存(常用目錄、SMB/SSH 連線設定、視窗狀態)
├── keychain.rs        # Keychain 讀寫密碼
├── i18n/              # 語言檔 (en, zh-TW)
├── fs/
│   ├── provider.rs    # trait FsProvider: list/stat/read/write/mkdir/rename/delete
│   ├── local.rs       # 本機檔案系統(SMB 掛載後也走這裡)
│   ├── sftp.rs        # SFTP 虛擬檔案系統 (russh-sftp)
│   └── smb.rs         # SMB 掛載管理(mount_smbfs / 偵測已掛載 / 卸載)
├── ops/
│   ├── queue.rs       # 背景操作佇列:進度回報 channel、取消、衝突詢問
│   ├── transfer.rs    # copy/move,含跨 provider(本機↔SFTP)串流傳輸
│   ├── delete.rs      # 垃圾筒(trash crate,僅本機)/ 永久刪除
│   └── archive.rs     # 壓縮(zip/tar/gz/tgz,排除 .DS_Store 與 ._*)、解壓(+7z)
└── ui/
    ├── toolbar.rs     # 主工具列:檢視模式切換、SMB/SSH 連線按鈕
    ├── panel.rs       # PanelState + 面板 UI:面板工具列、路徑列、檔案列表
    ├── file_list.rs   # 列表檢視(egui_extras Table,欄位排序)/ 大圖示 grid 檢視
    ├── command_bar.rs # 底部功能鍵列 + 狀態列 + 進度顯示
    └── dialogs/       # SMB 連線、SSH 連線、衝突處理、壓縮選項、刪除確認、常用目錄管理
```

**核心抽象:** `FsProvider` trait 統一本機與 SFTP。Panel 持有 `Box<dyn FsProvider>` + 當前路徑 + 選取狀態。SMB 不需要 provider——掛載後是 `/Volumes/xxx` 本機路徑。

**操作佇列:** UI 執行緒只發指令;worker 執行並經 channel 回報進度/衝突;衝突時佇列暫停等 UI 回應(覆蓋/跳過/重新命名/全部套用)。

## UI/UX 設計

- **佈局:** 上:主工具列(大圖示/列表切換、SMB、SSH)。中:左右 Panel,各有面板工具列(Home、常用目錄▾、已連線磁碟▾)、可編輯 breadcrumb 路徑列、檔案列表(欄:名稱/大小/修改日期/種類,點欄頭排序)。下:狀態列(選取數/大小)+ 功能鍵列(F5 Copy、F6 Move、F7 New Folder、F8 Delete)+ 操作進度。
- **作用中面板:** 一側 active(高亮邊框),Tab 切換;複製/移動方向固定 active → 另一側。
- **選取:** Mac 風格(點擊單選、Cmd+點擊、Shift+點擊、Cmd+A);空白鍵 QuickLook。
- **鍵盤:** Enter 開啟/進入目錄、Cmd+↑ 或 Backspace 回上層、直接打字即時過濾、F5~F8 與 Cmd 系等價鍵(Cmd+Shift+C 複製到對側等)、Cmd+Delete 移至垃圾筒、Cmd+Shift+Delete 永久刪除(需確認)。
- **刪除:** 預設移至垃圾筒;永久刪除走明確的確認對話框。SFTP 上只有永久刪除(無垃圾筒),對話框要說明。
- **SMB 對話框:** 伺服器/分享名/帳號/密碼、「記住密碼(Keychain)」;已儲存連線列表可編輯,點擊自動掛載並導向 /Volumes/xxx。
- **SSH 對話框:** 自動列出 ~/.ssh/config 的 Host 供選擇(帶入 HostName/User/Port/IdentityFile),也可手動輸入;金鑰或密碼認證;已儲存連線可編輯。連上後該 Panel 切換為 SFTP provider,麵包屑顯示 `user@host:/path`。
- **壓縮對話框:** 選目標格式(預設 .zip)、檔名、目的面板;目錄壓縮自動排除 `.DS_Store` 與 `._*`。解壓:對壓縮檔按 Enter 或右鍵選單解壓到對側面板。
- **主題:** 跟隨系統深淺色(eframe 支援)。

## 實作里程碑

1. **骨架與本機瀏覽:** eframe app、雙 Panel、本機目錄列表/排序/導覽、列表與大圖示兩種檢視、active panel 與鍵盤導覽、i18n 骨架。
2. **檔案操作:** 操作佇列 + 進度 UI、copy/move/rename/新資料夾、垃圾筒與永久刪除、衝突對話框、`notify` 自動刷新。
3. **壓縮/解壓:** zip/tar/gz/tgz 壓縮(排除規則)、zip/7z/tar/gz/tgz 解壓、壓縮對話框。
4. **SMB:** 連線對話框、Keychain、mount_smbfs 掛載/卸載、已掛載偵測、儲存連線與面板工具列整合。
5. **SFTP:** FsProvider 實作、~/.ssh config/金鑰、跨 provider 傳輸(本機↔SFTP)、儲存連線。
6. **收尾:** 常用目錄管理、QuickLook、快捷鍵總表、設定持久化(視窗大小/各面板最後路徑)。

## 驗證方式

- `cargo build` / `cargo clippy` / `cargo test` 全程保持通過。
- 單元測試:壓縮排除規則(.DS_Store/._*)、路徑導覽邏輯、ssh config 解析、衝突決策邏輯。
- 每個里程碑以 `cargo run` 手動驗證:M2 用大檔案測進度與取消;M4 需實際 SMB 伺服器(使用者環境)驗證掛載後 Finder 可見;M5 用實際 SSH 主機驗證金鑰登入與雙向傳輸。
