# Your-Commander

開發一個支援 macOS 的檔案管理工具。類似 Windows 的 Total Commander，有左/右兩邊的 Panel，可以同時管理兩個不同的目錄。

UI 設計稿：`File Manager Screens.html`（Nocturne 深色設計系統）。實作計畫與架構說明見 `PLAN.md`。

## 技術棧

- **Tauri 2**：Rust 後端（`src-tauri/`）＋ React 18 + TypeScript 前端（`src/`，Vite）
- 前端狀態：Zustand；虛擬列表：TanStack Virtual；i18n：i18next（en 預設、zh-TW）
- 後端：russh/russh-sftp（SFTP）、zip/tar/flate2/sevenz-rust2（壓縮）、trash、notify、security-framework（Keychain）

## 開發

```bash
npm install          # 首次
npm run tauri dev    # 開發模式（Vite + Tauri）
npm run tauri build  # 打包 .app（ad-hoc 簽章）
cargo test           # 後端單元測試（在 src-tauri/ 下執行）
npx tsc -b           # 前端型別檢查
```

## 基本檔案管理功能

1. Copy
1. Cut/Paste
1. Move
1. Delete
    - 永久刪除
    - 移至垃圾筒

## 壓縮/解壓縮

1. 支援 .zip, 7z, .tar, .gz, tgz 等格式，預設是 .zip（7z 目前僅解壓）
1. 指定目錄或檔案壓縮，如果是指定目錄，壓縮時需排除 .DS_Store 檔案與 ._xxx 的檔案。

## 網路磁碟

1. 支援 SMB (Samba)，可儲存/修改設定，供下次自動連線。連線後，與 macOS Finder 共享（掛載於 /Volumes）。
1. 支援 SSH (SFTP)，可儲存/修改設定，可使用 $HOME/.ssh 設定與公鑰，供下次自動連線。

## UI 顯示

1. 第一列是工具列：重新整理、Brief/Full 檢視切換、左右互換、（比對/批次改名/搜尋/同步：後續階段）、連線
1. 左/右兩邊 Panel，各有分頁列、磁碟選單（Home、常用目錄、已掛載磁碟）、可編輯路徑列（支援萬用字元過濾）、檔案列表（Name/Ext/Size/Date 可排序）
1. F3 Quick View：對側面板切換為預覽（跟隨游標）
1. 底部 F-key 列：F3 預覽、F4 編輯、F5 複製、F6 移動、F7 新資料夾、F8 垃圾筒、⌘K 指令、⌘Q 結束
