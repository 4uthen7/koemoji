# KoeMoji — 音声・動画のローカル文字起こしアプリ

音声・動画ファイルを **完全ローカル** で文字起こしし、動画内の文字も含む Markdown ノートを作る Tauri 製デスクトップアプリです。音声認識は whisper.cpp、画面認識は ffmpeg + Tesseract OCR を使用します。モデルの初回ダウンロード以降は **ネット接続不要・利用料ゼロ** で動作し、メディアデータが外部に送信されることはありません。

**GPU アクセラレーション（CUDA）はアプリ内から後付けで有効化できます。** CPU のみの環境でもそのまま使えます。

## 機能

- 音声・動画ファイルのドラッグ&ドロップ / ファイル選択
- 動画ファイルからの音声トラック自動抽出（Symphonia、未対応コーデックは ffmpeg に自動フォールバック）
- **GPU アクセラレーション**: NVIDIA GPU があればアプリ内のボタン一つで CUDA 対応エンジンをダウンロードし、高速文字起こしが可能
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

## GPU アクセラレーション（オプション・アプリ内設定）

KoeMoji は **デフォルトで CPU 動作** です。NVIDIA GPU（RTX 3060 等）をお持ちの場合、アプリ内からワンクリックで CUDA アクセラレーションを有効化できます。

1. アプリを起動すると自動で GPU を検出します
2. GPU が検出されると「GPU アクセラレーション」パネルに **「GPU を有効化」** ボタンが表示されます
3. クリックすると CUDA 対応 whisper エンジンをダウンロード（モデルと同じ仕組み）
4. ダウンロード完了後、次回の文字起こしから自動で GPU が使われます

**仕組み**: CPU 版のアプリ本体に加えて、CUDA でビルドされた `whisper-cli` をアプリデータフォルダにダウンロードします。GPU が有効な場合、文字起こしはこの CUDA バイナリを経由して実行されます。

## 画面OCRの流れ

```text
動画
  ├─ 音声トラック → Whisper (CPU / GPU) → 音声セグメント
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

## インストール（一般ユーザー）

配布されたインストーラを使う場合、Node.js / Rust / CMake は不要です。画面OCRを使う場合だけ、別途 ffmpeg と Tesseract（日英データ）を導入してください。GPU アクセラレーションはアプリ内からセットアップできるため、CUDA Toolkit の手動インストールは不要です。

### Windows

1. 配布された `.msi` または `.exe` を実行します。
2. 画面OCRを使う場合は PowerShell で次を実行します。

```powershell
winget install --id Gyan.FFmpeg -e
winget install --id UB-Mannheim.TesseractOCR -e
```

Chocolatey を使う場合は、管理者 PowerShell で次を実行します。

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

### macOS

1. 配布された `.dmg` を開き、KoeMoji を `Applications` に移動します。
2. 画面OCRを使う場合は [Homebrew](https://brew.sh/) を導入し、ターミナルで次を実行します。

```bash
brew install ffmpeg tesseract tesseract-lang
which ffmpeg tesseract
tesseract --list-langs
```

`jpn` と `eng` が表示されたら KoeMoji を再起動してください。

### 初回起動と画面OCR

1. アプリ内のモデル管理からモデルをダウンロードします。最初は `small` が扱いやすい選択です。
2. 音声・動画ファイルをドロップするか、ファイル選択で開きます。
3. 動画内の文字も読み取る場合は「画面OCR」をオンにし、抽出間隔を選びます。
4. GPU がある場合は「GPU アクセラレーション」パネルから有効化します。
5. 文字起こしを開始します。

## ソースから起動・ビルド

Node.js 18以上、Rust stable、CMake が必要です。画面OCRを使う場合は ffmpeg と Tesseract（日英データ）も導入します。

### Windows

```powershell
winget install --id OpenJS.NodeJS.LTS -e
winget install --id Rustlang.Rustup -e
winget install --id Kitware.CMake -e
winget install --id Gyan.FFmpeg -e
winget install --id UB-Mannheim.TesseractOCR -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
rustup default stable
```

### macOS

```bash
xcode-select --install
brew install node cmake ffmpeg tesseract tesseract-lang
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup default stable
```

### 開発モード

```bash
npm install
npm run tauri dev
```

### 配布用ビルド

```bash
npm install
npm run tauri build
```

成果物は `src-tauri/target/release/bundle/` 以下に生成されます。

- Windows: `msi/` の `.msi`、`nsis/` の `.exe`
- macOS: `dmg/` の `.dmg`、`macos/` の `.app`

### 配布時の注意

- **GPU アクセラレーションはアプリ内ダウンロード方式** のため、配布バイナリは軽量（CPU 版のみ）です。
- CUDA 対応 whisper バイナリを GitHub Releases にアップロードすることで、ユーザーがアプリ内からダウンロードできるようになります。
- モデルもバイナリに含まれません。
- クロスコンパイルは不可。Windows 版は Windows で、macOS 版は macOS でビルドしてください。

## モデルの選び方

| モデル | サイズ | 目安 |
|---|---|---|
| tiny / base | 78–148 MB | 動作確認用。日本語の精度は低い |
| small | 488 MB | 軽さと精度のバランス。まずはこれ |
| medium | 1.5 GB | 高精度だが CPU ではかなり遅い |
| **large-v3-turbo** | 1.6 GB | **精度と速度の両立でおすすめ。GPU ならこれ** |
| large-v3 | 3.1 GB | 最高精度。CPU では実時間の数倍 |

モデルは Hugging Face の [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) から取得しています。

## アーキテクチャ

```
フロント (Vite + Vanilla JS)
  └─ invoke ─────────────► Rust (Tauri v2)
       transcribe            ├─ audio.rs      symphonia / ffmpeg → 16kHz mono f32
       list_models           ├─ model.rs      HF から ggml モデルを DL
       download_model        ├─ transcribe.rs whisper-rs (CPU) / CUDA whisper-cli
       check_ocr_support     ├─ ocr.rs        ffmpeg + Tesseract、重複除去、差分累積
       get_cumulative_ocr    ├─ gpu.rs        CUDA 検出・whisper-cli DL・GPU 実行
       check_gpu_support     └─ history.rs    履歴の保存/読込
       download_cuda_whisper
  ◄──── emit ──────────────
   transcribe-progress / transcribe-segment / model-download-progress / gpu-download-progress
```

## トラブルシューティング

- **英語部分が「（英語）」とだけ出力される** → 言語を「自動（日英混在向け）」（デフォルト）にしてください
- **`tauri dev` が再起動を繰り返す** → プロジェクトを iCloud 同期対象外の場所（例: `~/dev/`）へ移動
- **`cmake not found`** → CMake をインストールして PATH を通す
- **GPU 有効化ボタンが表示されない** → NVIDIA ドライバと CUDA ランタイムがインストールされているか確認。`nvidia-smi` が通れば検出されます
- **`whisper_full_with_state: failed to encode` で即失敗する** → whisper-rs 0.16.0 の既知のバグです。`Box<dyn FnMut>` での回避コードが入っているので `transcribe.rs` のコールバック箇所を触らないでください
- **whisper-rs のバージョン更新でコンパイルエラー** → 本コードは 0.16 系の API に合わせています
