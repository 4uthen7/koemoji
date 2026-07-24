# KoeMoji — 音声・動画のローカル文字起こしアプリ

音声・動画ファイルを **完全ローカル** で文字起こしし、動画内の文字も含む Markdown ノートを作る Tauri 製デスクトップアプリです。音声認識は whisper.cpp（デフォルトで **CUDA GPU アクセラレーション有効**）、画面認識は ffmpeg + Tesseract OCR を使用します。モデルの初回ダウンロード以降は **ネット接続不要・利用料ゼロ** で動作し、メディアデータが外部に送信されることはありません。

## 機能

- 音声・動画ファイルのドラッグ&ドロップ / ファイル選択
- 動画ファイルからの音声トラック自動抽出（Symphonia、未対応コーデックは ffmpeg に自動フォールバック）
- **GPU アクセラレーション**: RTX 3060 等の NVIDIA GPU で高速文字起こし（デフォルトで CUDA 有効）
- **画面OCR（日英）**: ffmpeg で既定5秒ごとにフレームを抽出し、Tesseract (`jpn+eng`) で認識
- **OCR重複除去**: 前フレームと同じ行や微小な認識揺れを除去。画面切り替え時は文脈を保って全文を採用
- **OCR累積テキスト**: 授業スライド等の画面変化を差分追跡し、スライド全文＋追加行を時系列で記録。別ファイルとして書き出し可能
- 音声と画面テキストをタイムスタンプ順に統合表示
- Whisper モデルの管理（アプリ内からダウンロード / 削除）
  - tiny / base / small / medium / large-v3-turbo / large-v3
- 言語指定（自動判定 / 日本語 / 英語 / 中国語 ほか）と英語への翻訳出力
- **日英混在モード**: tiny モデルで10秒ごとに言語を判定し、区間ごとに適切な言語で文字起こし（授業録音・会議など言語が切り替わるファイル向け。デフォルト。初回は判定用 tiny を自動ダウンロード）
- **リアルタイム表示**: 文字起こし中、セグメントが確定するたびに結果が画面に流れる。おかしな出力になっていたら途中でキャンセル可能（途中結果は保持され、コピー・保存もできる）
- 進捗表示・キャンセル
- 結果のコピー、TXT / SRT（音声字幕）/ VTT（音声字幕）/ **Markdown統合ノート** / **OCR累積テキスト** 形式での保存
- 文字起こし履歴の自動保存（アプリ内の「履歴」から再表示・削除可能）

## 対応形式

| 種別 | 形式 |
|---|---|
| 音声 | wav / mp3 / m4a / aac / flac / ogg / aiff など |
| 動画 | mp4 / m4v / mov / mkv / webm / avi / wmv / flv / mpeg / ts / 3gp など |

Symphonia が直接読めない WebM/Opus 等は、ffmpeg がインストールされていれば自動変換して文字起こしします。コンテナ内のコーデックによっては処理できない場合があります。

## 画面OCRの流れ

```text
動画
  ├─ 音声トラック → Whisper (CUDA) → 音声セグメント
  └─ ffmpeg (5秒ごと) → PNG → Tesseract (jpn+eng) → 重複除去 → 画面セグメント
                                    └─ 差分追跡 → OCR累積テキスト
                                             ↓
                         時刻順に統合 → Markdownノート
```

フレーム間隔は画面で3 / 5 / 10 / 15秒から選べます。SRT/VTTには音声だけを出し、Markdownには音声と画面OCRの両方を出します。OCR累積テキストは `.txt` で別途保存できます。

## OCR 累積テキスト

授業のスライドやプレゼン動画のように画面が徐々に切り替わるコンテンツ向けの機能です。文字起こし中に画面OCRをオンにすると、以下のように累積テキストが自動生成されます：

- **スライド切り替え時**（過半数の行が新しい）→ 新しいスライドの全文をタイムスタンプ付きで記録（`← Slide`）
- **差分のみの時**（一部の行だけ追加/変更）→ 追加された行だけを記録（`+`）

文字起こし完了後、結果画面の **「OCR累積テキスト保存」** ボタンから書き出せます。

出力例：
```text
# OCR Slide Text: lecture.mp4
# Generated: 2026-07-24 15:30:00

---

## [00:05] ← Slide
微積分学 第3回
本日のテーマ：微分の応用

---

## [00:10] +
+ 例題1: f(x) = x² の導関数

---

## [00:15] ← Slide
第2章：積分法
不定積分と定積分
```

## インストール（一般ユーザー）

配布されたインストーラを使う場合、Node.js / Rust / CMake は不要です。画面OCRを使う場合だけ、別途 ffmpeg と Tesseract（日英データ）を導入してください。GPU を使う場合は CUDA ランタイムも必要です。

### Windows

1. 配布された `.msi` または `.exe` を実行します。
2. 画面OCRを使う場合は PowerShell で次を実行します。

```powershell
winget install --id Gyan.FFmpeg -e
winget install --id UB-Mannheim.TesseractOCR -e
```

Chocolatey を使う場合は、管理者 PowerShell で次を実行します。winget と Chocolatey の両方を実行する必要はありません。

```powershell
choco install ffmpeg tesseract -y
```

Tesseract に日本語データがない場合は、管理者 PowerShell で公式の `jpn.traineddata` を追加します。

```powershell
$tesseractDir = Split-Path (Get-Command tesseract).Source
curl.exe -L https://github.com/tesseract-ocr/tessdata_fast/raw/main/jpn.traineddata -o "$tesseractDir\tessdata\jpn.traineddata"
```

導入後は新しい PowerShell で次を確認し、KoeMoji を再起動してください。`jpn` と `eng` が表示されればOCRを利用できます。

```powershell
where.exe ffmpeg
where.exe tesseract
tesseract --list-langs
```

コマンドが見つからない場合は、ffmpeg の `bin` フォルダと `C:\Program Files\Tesseract-OCR` をユーザー環境変数 `Path` に追加してください。

### macOS

1. 配布された `.dmg` を開き、KoeMoji を `Applications` に移動します。
2. 画面OCRを使う場合は [Homebrew](https://brew.sh/) を導入し、ターミナルで次を実行します。

```bash
brew install ffmpeg tesseract tesseract-lang
which ffmpeg tesseract
tesseract --list-langs
```

`jpn` と `eng` が表示されたら KoeMoji を再起動してください。署名されていないアプリがブロックされた場合は、Finder でアプリを Control + クリックして「開く」を選びます。

### 初回起動と画面OCR

1. アプリ内のモデル管理からモデルをダウンロードします。最初は `small` が扱いやすい選択です。
2. 音声・動画ファイルをドロップするか、ファイル選択で開きます。
3. 動画内の文字も読み取る場合は「画面OCR」をオンにし、抽出間隔（3 / 5 / 10 / 15秒）を選びます。
4. 文字起こしを開始します。OCRが選べない場合は、上記の確認コマンドを実行してからアプリを再起動してください。
5. 音声と画面文字をまとめて保存する場合は Markdown を選びます。SRT / VTT には音声字幕だけが入ります。スライドの文字だけを抜き出したい場合は「OCR累積テキスト保存」を選びます。

初回のモデルダウンロードにはネット接続が必要です。以後の文字起こしとOCRはローカルで動作します。OCRを使わない場合、Tesseractは不要です。ffmpegがなくても多くの音声形式は処理できますが、未対応コーデックの変換と画面OCRには必要です。

## ソースから起動・ビルド

Node.js 18以上、Rust stable、CMake、各OSのビルドツールが必要です。画面OCRを使う場合は、一般ユーザー向け手順と同じく ffmpeg と Tesseract（日英データ）も導入します。**GPU（CUDA）ビルドには CUDA Toolkit 12.x が追加で必要です。**

### Windows の依存関係（CUDA ビルド）

PowerShell で次を実行します。CUDA Toolkit は別途 [NVIDIA のサイト](https://developer.nvidia.com/cuda-downloads) からインストールしてください。

```powershell
winget install --id OpenJS.NodeJS.LTS -e
winget install --id Rustlang.Rustup -e
winget install --id Kitware.CMake -e
winget install --id Gyan.FFmpeg -e
winget install --id UB-Mannheim.TesseractOCR -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
rustup default stable
```

Chocolatey を使う場合は、管理者 PowerShell で次を実行できます。

```powershell
choco install nodejs-lts rustup.install cmake ffmpeg tesseract visualstudio2022buildtools visualstudio2022-workload-vctools -y
rustup default stable
```

CUDA Toolkit インストール後、`nvcc --version` が通ることを確認してください。

**CPU ビルドに切り替える場合** は `src-tauri/Cargo.toml` の whisper-rs 行を `features = []` に変更します。macOS では `features = ["metal"]` にしてください。

```toml
# CPU のみ
whisper-rs = { version = "0.16", features = [] }

# macOS Metal
whisper-rs = { version = "0.16", features = ["metal"] }
```

Windows 10 / 11 の WebView2 Runtime は通常導入済みです。見つからない場合は [Microsoft WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) から Evergreen Runtime を導入してください。インストール後はターミナルを開き直します。

### macOS の依存関係

ターミナルで Command Line Tools と Homebrew のパッケージを導入し、Rustは公式インストーラで追加します。

```bash
xcode-select --install
brew install node cmake ffmpeg tesseract tesseract-lang
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup default stable
```

macOS ではビルド前に `src-tauri/Cargo.toml` の whisper-rs features を `["metal"]` に変更してください。

### 開発モードで起動

PowerShell またはターミナルで、展開したソースのルートへ移動して実行します。

```bash
npm install
npm run tauri dev
```

初回は whisper.cpp のネイティブビルドが走るため数分かかります。

### 配布用ビルド

ビルド対象のOS上で次を実行します。Windows版はWindows、macOS版はmacOSでビルドしてください。

```bash
npm install
npm run tauri build
```

成果物は `src-tauri/target/release/bundle/` 以下に生成されます。

- Windows: `msi/` の `.msi`、`nsis/` の `.exe`
- macOS: `dmg/` の `.dmg`、`macos/` の `.app`
- Linux: `.deb` / `.rpm` / `.AppImage`

Linuxでビルドする場合は `cmake`、`build-essential`、ffmpeg、Tesseractに加えて [Tauri prerequisites](https://tauri.app/start/prerequisites/) の `libwebkit2gtk-4.1-dev` などが必要です。

### 配布時の注意

- **履歴の保存先**: アプリデータフォルダの `history/` に JSON で保存されます（macOS: `~/Library/Application Support/com.koemoji.app/history`）。
- **モデルはバイナリに含まれません。** ユーザーが初回起動時にアプリ内からダウンロードする方式なので、インストーラ自体は軽量です。モデルは各 OS のアプリデータフォルダ（Windows: `%APPDATA%\com.koemoji.app\models`）に保存されます。
- **CUDA ビルドの配布**: 実行環境にも CUDA ランタイム（`cudart` 等）が必要です。配布する場合は CUDA Toolkit の再配布可能パッケージを同梱するか、CPU 版を別途ビルドしてください。
- **コード署名なしの場合**、Windows では SmartScreen、macOS では Gatekeeper の警告が出ます。広く配布するなら署名（Windows: コード署名証明書、macOS: Apple Developer Program + notarization）を検討してください。
- クロスコンパイルは基本不可です。Windows 版は Windows で、macOS 版は macOS でビルドしてください。GitHub Actions で 3 OS 分を自動ビルドするのが定番です（`tauri-apps/tauri-action` が便利）。

## GPU アクセラレーション

KoeMoji は **デフォルトで CUDA（NVIDIA GPU）が有効** です。RTX 3060 等の NVIDIA GPU で高速に文字起こしできます。

| 設定 | Cargo.toml | ビルド環境 | 実行環境 |
|---|---|---|---|
| **CUDA（デフォルト）** | `features = ["cuda"]` | CUDA Toolkit 12.x | CUDA ランタイム |
| CPU のみ | `features = []` | 追加不要 | 追加不要 |
| macOS Metal | `features = ["metal"]` | Xcode CLT | macOS のみ |
| Vulkan | `features = ["vulkan"]` | Vulkan SDK | Vulkan ランタイム |

モデルを `large-v3-turbo` にすれば、RTX 3060 で実時間の数分の一で文字起こしが完了します。

## モデルの選び方

| モデル | サイズ | 目安 |
|---|---|---|
| tiny / base | 78–148 MB | 動作確認用。日本語の精度は低い |
| small | 488 MB | 軽さと精度のバランス。まずはこれ |
| medium | 1.5 GB | 高精度だが CPU ではかなり遅い |
| **large-v3-turbo** | 1.6 GB | **精度と速度の両立でおすすめ。GPU ならこれ** |
| large-v3 | 3.1 GB | 最高精度。CPU では実時間の数倍かかることも |

モデルは Hugging Face の [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) から取得しています。

## アーキテクチャ

```
フロント (Vite + Vanilla JS)
  └─ invoke ─────────────► Rust (Tauri v2)
       transcribe            ├─ audio.rs      symphonia / ffmpeg → 16kHz mono f32
       list_models           ├─ model.rs      HF から ggml モデルを DL（進捗イベント付き）
       download_model        ├─ transcribe.rs whisper-rs (CUDA) で実行、進捗/キャンセル対応
       check_ocr_support     ├─ ocr.rs        ffmpeg + Tesseract、重複除去、差分累積
       get_cumulative_ocr    └─ history.rs    履歴の保存/読込
  ◄──── emit ──────────────
   transcribe-progress / transcribe-segment / transcribe-status / model-download-progress
```

- タイムスタンプは whisper.cpp から centi 秒で返るため、Rust 側でミリ秒に変換しています。
- キャンセルは `AtomicBool` + whisper.cpp の abort コールバックで実装しています。
- OCR累積テキストは `AppState` に保持され、`get_cumulative_ocr_text` コマンドでフロントから取得します。
- 長時間ファイルはメモリに全展開されます（16kHz f32 で 1 時間 ≈ 230 MB）。数時間超のファイルを扱う場合は分割を検討してください。

## トラブルシューティング

- **英語部分が「（英語）」とだけ出力される** → Whisper の言語自動判定はファイル全体で1回のため、冒頭が日本語だと英語パートで注釈ハルシネーションが起きます。言語を「自動（日英混在向け）」（デフォルト）にするか、英語主体のファイルなら「英語」を明示指定してください。なお混在モードの言語切り替えの粒度は10秒単位です。1文の途中で言語が変わるような高速な切り替えは Whisper の仕組み上、完全には追従できません
- **`tauri dev` が再起動を繰り返す** → `src-tauri/.taurignore` でウォッチ除外を設定済みです。それでも続く場合、プロジェクトが iCloud 同期フォルダ（デスクトップ / 書類）にあると同期のたびに再起動されるため、同期対象外の場所（例: `~/dev/`）へ移動してください
- **`cmake not found`** → CMake をインストールして PATH を通す
- **Linux で WebKit エラー** → Tauri prerequisites のパッケージを確認
- **ビルドは通るが文字起こしが遅い** → モデルを `large-v3-turbo` に変更、または GPU が正しく認識されているか `nvidia-smi` で確認
- **CUDA ビルドで `nvcc not found`** → CUDA Toolkit をインストールして PATH を通す
- **whisper-rs のバージョン更新でコンパイルエラー** → whisper-rs はメジャーごとに API が変わります。本コードは 0.16 系の API（`full_n_segments() -> i32`、`get_segment(i)`、`set_abort_callback_safe` 等）に合わせています
- **`whisper_full_with_state: failed to encode` で即失敗する** → whisper-rs 0.16.0 の `set_abort_callback_safe` には型不一致のバグがあり、素のクロージャを渡すとコールバックが不定値を返して即中断されます。本コードでは `Box<dyn FnMut() -> bool>` に包んで渡すことで回避しています（`transcribe.rs` のコメント参照）。この箇所を素のクロージャに書き換えないでください
