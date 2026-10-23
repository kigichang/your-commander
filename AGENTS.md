# Your-Commander

開發一個支援 MAC OS 的檔案管理工具。類似 Windows 的 Total Commander 有左/右兩邊的 Panel，可以同時管理兩個不同的目錄。

## 基本檔案管理功能


1. Copy
1. Cut/Paste
1. Move
1. Delete
    - 永久刪除
    - 移至垃圾筒

## 壓縮/解壓縮

1. 支援 .zip, 7z, .tar, .gz, tgz 等格式，預設是 .zip
1. 指定目錄或檔案壓縮，如果是指定目錄，壓縮時需排除 .DS_Store 檔案與 ._xxx 的檔案。

## 網路磁碟

1. 支援 SMB (Samba)，可儲存/修改設定，供下次自動連線。連線後，與 MacOS Finder 共享。
1. 支搜 SSH，可儲存/修改設定，可使用 $HOME/.ssh 設定與公鑰，，供下次自動連線。

## UI 顯示

1. 第一列是工具列，有大圖示顯示，列表顯示，SMB 連線，SSH 連線
1. 左/右兩邊 Panel，Panel 工具列顯示：Home 目錄，自定義常用的目錄，已連線網路磁碟。
