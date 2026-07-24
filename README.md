# KoeMoji — 音声・動画のローカル文字起こしアプリ

音声・動画ファイルを完全ローカルで文字起こしして、動画内の文字も含むノートにする Tauri 製デスクトップアプリです。whisper.cpp で音声認識、ffmpeg + Tesseract で画面OCR。モデルの初回DL以降はネット不要・無料。GPU 使いたい人はアプリ内から後付けで入れられます。

## 機能

- 音声・動画ファイルのドラッグ&ドロップ / ファイル選択
- 動画から音声トラックを自動抽出（Symphonia、無理なら ffmpeg にフォールバック）
- **GPU アクセラレーション**: NVIDIA GPU 持ってる人はアプリ内のボタンで CUDA エンジンをDLすれば使える。なくても CPU で動く
- **画面OCR（日英）**: ffmpeg でフレーム抽出 → Tesseract (`jpn+eng`) で文字起こし
- **OCR重複除去**: 同じ画面の文字は弾く。スライド切り替わったら全文取り直す
- **OCR累積テキスト**: 授業スライドみたいに画面が変わっていくやつ用。差分を追いかけてタイムスタンプ付きでテキスト化
- 音声と画面テキストをタイムラインに統合
- Whisper モデルの管理（アプリ内からDL/削除）
  - tiny / base / small / medium / large-v3-turbo / large-v3
- 言語指定（自動 / 日英混在 / 日本語 / 英語 / 中国語 / 韓国語 / スペイン語 / フランス語 / ドイツ語）
- **日英混在モード**: 10秒ごとに言語判定して区間別に文字起こし。授業とか会議とか
- **リアルタイム表示**: セグメント確定するたびに画面に出る。変だったら途中キャンセル可
- 結果のコピー / TXT保存 / SRT保存 / VTT保存 / Markdownノート保存 / OCR累積テキスト保存
- 文字起こし履歴の自動保存（アプリ内から再表示・削除できる）

## 対応形式

| 種別 | 形式 |
|---|---|
| 音声 | wav / mp3 / m4a / aac / flac / ogg / aiff など |
| 動画 | mp4 / m4v / mov / mkv / webm / avi / wmv / flv / mpeg / ts / 3gp など |

WebM/Opus とか Symphonia が読めないやつは ffmpeg があれば自動変換します。

## 画面OCRの流れ

```text
動画
  ├─ 音声トラック → Whisper (CPU or GPU) → 音声セグメント
  └─ ffmpeg (5秒ごと) → PNG → Tesseract (jpn+eng) → 重複除去 → 画面セグメント
                                    └─ 差分追跡 → OCR累積テキスト
                                             ↓
                         時刻順に統合 → Markdownノート
```

フレーム間隔は3 / 5 / 10 / 15秒から選べる。SRT/VTTは音声だけ、Markdownは音声+画面、OCR累積テキストは.txtで別途保存。

## GPU アクセラレーション

デフォルトはCPU。NVIDIA GPU（RTX 3060とか）があればアプリ起動時に自動で検出して「有効にする？」って聞いてくる。OKすると CUDA 対応 whisper エンジンをDLして、次からGPUで動く。モデルのDLと同じノリ。

CPUしかない環境でもそのまま動く。GPUの有無でバイナリ分けなくていい。

内部的には CUDA ビルドした whisper-cli をアプリデータフォルダに落として使ってる。中身気になる人は `gpu.rs` 見て。

## OCR 累積テキスト

スライドが徐々に切り替わる授業動画とか用。

- スライドがガラッと変わった → 全文を `← Slide` で記録
- 一部だけ追加された → 追加行だけ `+` で記録

文字起こし終わったら結果画面の「OCR累積テキスト保存」で.txtに書き出せる。

## インストール（一般ユーザー）

配布されてる `.msi` か `.exe` 入れるだけ。Node.js / Rust / CMake 不要。画面OCRしたい人は ffmpeg と Tesseract（日英データ）を別途入れて。GPUはアプリ内で完結。

### Windows

```
winget install --id Gyan.FFmpeg -e
winget install --id UB-Mannheim.TesseractOCR -e
```

Chocolatey:

```
choco install ffmpeg tesseract -y
```

日本語データ:

```powershell
$tesseractDir = Split-Path (Get-Command tesseract).Source
curl.exe -L https://github.com/tesseract-ocr/tessdata_fast/raw/main/jpn.traineddata -o "$tesseractDir\tessdata\jpn.traineddata"
```

`jpn` と `eng` が出ればOK。

```powershell
where.exe ffmpeg
where.exe tesseract
tesseract --list-langs
```

### macOS

```bash
brew install ffmpeg tesseract tesseract-lang
```

### 初回起動

1. モデル管理からモデルDL（small か large-v3-turbo が無難）
2. ファイルをドロップ
3. 動画なら画面OCRオン（任意）
4. GPUあるなら有効化（任意）
5. 文字起こし開始

## ソースからビルド

Node.js 18+、Rust stable、CMake。Windows は Visual Studio Build Tools（C++ワークロード）も。

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
rustup default stable
```

### 起動

```bash
npm install
npm run tauri dev
```

### 配布用

```bash
npm run tauri build
```

成果物: `src-tauri/target/release/bundle/`

- Windows: `.msi`、`.exe`
- macOS: `.dmg`、`.app`

### 配布時の注意

- GPU エンジンはアプリ内DL方式。配布バイナリはCPU版のみで軽い。CUDA 対応 whisper-cli は GitHub Releases に `cuda-runtime` タグで置いておくとアプリが拾う
- モデルもバイナリに入ってない。初回にDL
- コード署名なしだと SmartScreen / Gatekeeper が出る
- クロスコンパイル不可。Windows 版は Windows で、Mac 版は Mac でビルド
- 履歴: `%APPDATA%\com.koemoji.app\history`（Win）`~/Library/Application Support/com.koemoji.app/history`（Mac）
- モデル保存先: `%APPDATA%\com.koemoji.app\models`

## モデルの選び方

| モデル | サイズ | 目安 |
|---|---|---|
| tiny / base | 78–148 MB | 確認用。日本語精度は低い |
| small | 488 MB | バランス型。まずはこれ |
| medium | 1.5 GB | 高精度、CPUだと遅い |
| **large-v3-turbo** | 1.6 GB | 精度と速度のいいとこ取り。GPUあるならこれ |
| large-v3 | 3.1 GB | 最高精度。CPUだと実時間の数倍 |

モデルは [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) から。

## アーキテクチャ

```
フロント (Vite + Vanilla JS)
  invoke → Rust (Tauri v2)
    transcribe          audio.rs      symphonia / ffmpeg → 16kHz mono f32
    list_models         model.rs      HF から ggml モデルDL
    download_model      transcribe.rs whisper-rs (CPU) / CUDA whisper-cli
    check_ocr_support   ocr.rs        ffmpeg + Tesseract、重複除去、差分累積
    check_gpu_support   gpu.rs        GPU検出、CUDAエンジンDL、GPU実行
    download_cuda       history.rs    履歴保存/読込
  emit ←
    transcribe-progress / transcribe-segment / model-download-progress / gpu-download-progress
```

## トラブルシューティング

- **英語が「（英語）」になる** → 言語を「日英混在」にして
- **tauri dev が再起動ループ** → iCloud 同期フォルダ避けて `~/dev/` とかに置く
- **cmake not found** → CMake 入れて PATH 通す
- **GPU 有効化ボタン出ない** → `nvidia-smi` が通るか確認。CUDA Toolkit が入ってないと検出されない
- **GPU DL に失敗する** → GitHub Releases の `cuda-runtime` タグに `whisper-cli-cuda.exe` が置いてあるか確認
- **whisper_full_with_state: failed to encode** → whisper-rs 0.16 のバグ。`transcribe.rs` の abort callback を触らないで
- **whisper-rs バージョンアップでコンパイルエラー** → 0.16 系のAPIに合わせてる

---

@4uthent / tkmt_wonderkid
