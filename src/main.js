import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";

// ---------------------------------------------------------------
// 状態
// ---------------------------------------------------------------
let selectedFile = null; // 絶対パス
let currentBaseName = "transcript"; // エクスポート時のファイル名の基準
let currentSourceName = "transcript";
let segments = [];
let liveSegments = []; // 実行中にリアルタイムで届くセグメント
let busy = false;
let models = [];
let ocrAvailable = false;
let ocrSupportMessage = "画面OCRの環境を確認中…";

const MEDIA_EXTENSIONS = [
  "wav", "mp3", "m4a", "aac", "flac", "ogg", "oga", "aiff", "aif",
  "mp4", "m4v", "mov", "mkv", "mka", "webm", "avi", "wmv", "flv",
  "mpeg", "mpg", "ts", "mts", "m2ts", "3gp",
];
const VIDEO_EXTENSIONS = new Set([
  "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv",
  "mpeg", "mpg", "ts", "mts", "m2ts", "3gp",
]);

// ---------------------------------------------------------------
// DOM
// ---------------------------------------------------------------
const $ = (id) => document.getElementById(id);
const dropzone = $("dropzone");
const dzTitle = $("dz-title");
const modelSelect = $("model-select");
const langSelect = $("lang-select");
const translateCheck = $("translate-check");
const ocrCheck = $("ocr-check");
const ocrInterval = $("ocr-interval");
const ocrNote = $("ocr-note");
const runBtn = $("run-btn");
const cancelBtn = $("cancel-btn");
const modelList = $("model-list");
const progressStrip = $("progress-strip");
const progressLabel = $("progress-label");
const tapeFill = $("tape-fill");
const resultsSection = $("results");
const segmentList = $("segment-list");
const segCount = $("seg-count");
const lamp = $("lamp");
const toast = $("toast");

// ---------------------------------------------------------------
// ユーティリティ
// ---------------------------------------------------------------
let toastTimer = null;
function showToast(message, isError = false) {
  toast.textContent = message;
  toast.classList.toggle("error", isError);
  toast.classList.remove("hidden");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.add("hidden"), 3600);
}

function fileName(path) {
  return path.split(/[\\/]/).pop();
}

function baseName(path) {
  const name = fileName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

function extension(path) {
  const name = fileName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

function selectedFileIsVideo() {
  return Boolean(selectedFile && VIDEO_EXTENSIONS.has(extension(selectedFile)));
}

function pad(n, len = 2) {
  return String(n).padStart(len, "0");
}

function msToClock(ms, sep) {
  const h = Math.floor(ms / 3600000);
  const m = Math.floor((ms % 3600000) / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  const milli = Math.floor(ms % 1000);
  return `${pad(h)}:${pad(m)}:${pad(s)}${sep}${pad(milli, 3)}`;
}

function msToShort(ms) {
  const h = Math.floor(ms / 3600000);
  const m = Math.floor((ms % 3600000) / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

function formatMB(mb) {
  return mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`;
}

// ---------------------------------------------------------------
// エクスポート形式
// ---------------------------------------------------------------
function toTxt() {
  const hasOcr = segments.some((s) => s.source === "ocr");
  return (
    segments
      .map((s) => {
        if (!hasOcr) return s.text;
        const label = s.source === "ocr" ? "画面" : "音声";
        return `[${msToShort(s.start_ms)}][${label}] ${s.text.replace(/\n/g, " / ")}`;
      })
      .join("\n") + "\n"
  );
}

function toSrt() {
  const speech = segments.filter((s) => s.source !== "ocr");
  return (
    speech
      .map(
        (s, i) =>
          `${i + 1}\n${msToClock(s.start_ms, ",")} --> ${msToClock(s.end_ms, ",")}\n${s.text}`
      )
      .join("\n\n") + "\n"
  );
}

function toVtt() {
  const speech = segments.filter((s) => s.source !== "ocr");
  return (
    "WEBVTT\n\n" +
    speech
      .map(
        (s) =>
          `${msToClock(s.start_ms, ".")} --> ${msToClock(s.end_ms, ".")}\n${s.text}`
      )
      .join("\n\n") +
    "\n"
  );
}

function escapeMarkdown(text) {
  return text.replace(/\\/g, "\\\\").replace(/\|/g, "\\|").replace(/\r?\n/g, "<br>");
}

function toMarkdown() {
  const speechCount = segments.filter((s) => s.source !== "ocr").length;
  const ocrCount = segments.filter((s) => s.source === "ocr").length;
  const sourceName = currentSourceName;
  const generatedAt = new Date().toLocaleString("ja-JP");
  const timeline = segments
    .map((s) => {
      const label = s.source === "ocr" ? "🖼️ 画面" : "🎙️ 音声";
      return `| ${msToShort(s.start_ms)} | ${label} | ${escapeMarkdown(s.text)} |`;
    })
    .join("\n");

  return `# ${currentBaseName}

- 元ファイル: \`${sourceName.replace(/`/g, "'")}\`
- 作成日時: ${generatedAt}
- 音声セグメント: ${speechCount}
- 画面OCRセグメント: ${ocrCount}

## 統合タイムライン

| 時刻 | 種別 | 内容 |
| ---: | :--- | :--- |
${timeline}
`;
}

// ---------------------------------------------------------------
// ファイル選択
// ---------------------------------------------------------------
function setFile(path) {
  if (busy) return;
  selectedFile = path;
  dzTitle.textContent = fileName(path);
  dropzone.classList.add("has-file");
  updateOcrControls(true);
  updateRunButton();
}

function updateOcrControls(autoSelect = false) {
  const isVideo = selectedFileIsVideo();
  const canUse = isVideo && ocrAvailable && !busy;
  if (autoSelect) ocrCheck.checked = canUse;
  ocrCheck.disabled = !canUse;
  ocrInterval.disabled = !canUse || !ocrCheck.checked;

  if (!isVideo && selectedFile) {
    ocrCheck.checked = false;
    ocrNote.textContent = "音声ファイルでは画面OCRを使用しません。";
  } else {
    ocrNote.textContent = ocrSupportMessage;
  }
}

ocrCheck.addEventListener("change", () => updateOcrControls(false));

async function pickFile() {
  if (busy) return;
  const path = await open({
    multiple: false,
    filters: [{ name: "音声・動画", extensions: MEDIA_EXTENSIONS }],
  });
  if (typeof path === "string") setFile(path);
}

dropzone.addEventListener("click", pickFile);
dropzone.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    pickFile();
  }
});

// Vite単体プレビューにはTauri WebView APIがないため、開発時は登録だけスキップする。
try {
  getCurrentWebview().onDragDropEvent((event) => {
    const { type } = event.payload;
    if (type === "over" || type === "enter") {
      dropzone.classList.add("dragover");
    } else if (type === "leave") {
      dropzone.classList.remove("dragover");
    } else if (type === "drop") {
      dropzone.classList.remove("dragover");
      const paths = event.payload.paths || [];
      if (paths.length > 0) setFile(paths[0]);
    }
  });
} catch {
  // Tauri内では必ず登録される。ブラウザでのレイアウト確認時だけここに入る。
}

// ---------------------------------------------------------------
// モデル管理
// ---------------------------------------------------------------
async function refreshModels() {
  models = await invoke("list_models");
  renderModelSelect();
  renderModelList();
  updateRunButton();
}

function renderModelSelect() {
  const current = modelSelect.value;
  modelSelect.innerHTML = "";
  for (const m of models) {
    const opt = document.createElement("option");
    opt.value = m.id;
    opt.textContent = m.downloaded ? m.id : `${m.id}(未ダウンロード)`;
    modelSelect.appendChild(opt);
  }
  const downloaded = models.filter((m) => m.downloaded).map((m) => m.id);
  if (current && models.some((m) => m.id === current)) {
    modelSelect.value = current;
  } else if (downloaded.includes("large-v3-turbo")) {
    modelSelect.value = "large-v3-turbo";
  } else if (downloaded.length > 0) {
    modelSelect.value = downloaded[downloaded.length - 1];
  } else {
    modelSelect.value = "small";
  }
}

function renderModelList() {
  modelList.innerHTML = "";
  for (const m of models) {
    const li = document.createElement("li");
    li.className = "model-row";
    li.dataset.id = m.id;

    const id = document.createElement("span");
    id.className = "m-id";
    id.textContent = m.id;

    const desc = document.createElement("span");
    desc.className = "m-desc";
    desc.textContent = m.description;

    const size = document.createElement("span");
    size.className = "m-size";
    size.textContent = formatMB(m.size_mb);

    li.append(id, desc, size);

    if (m.downloaded) {
      const state = document.createElement("span");
      state.className = "m-state downloaded";
      state.textContent = "✓ 済";
      const del = document.createElement("button");
      del.className = "danger-text";
      del.textContent = "削除";
      del.addEventListener("click", async () => {
        try {
          await invoke("delete_model", { modelId: m.id });
          showToast(`${m.id} を削除しました`);
          await refreshModels();
        } catch (e) {
          showToast(String(e), true);
        }
      });
      li.append(state, del);
    } else {
      const dl = document.createElement("button");
      dl.textContent = "ダウンロード";
      dl.addEventListener("click", () => downloadModel(m.id, li, dl));
      li.append(dl);
    }
    modelList.appendChild(li);
  }
}

async function downloadModel(id, row, btn) {
  btn.disabled = true;
  btn.textContent = "取得中…";
  const bar = document.createElement("div");
  bar.className = "dl-progress";
  const fill = document.createElement("div");
  bar.appendChild(fill);
  row.insertBefore(bar, btn);
  row.dataset.downloading = "1";
  try {
    await invoke("download_model", { modelId: id });
    showToast(`${id} のダウンロードが完了しました`);
  } catch (e) {
    showToast(String(e), true);
  }
  await refreshModels();
}

listen("model-download-progress", (event) => {
  const { id, downloaded, total } = event.payload;
  const row = modelList.querySelector(`.model-row[data-id="${id}"]`);
  if (!row) return;
  const fill = row.querySelector(".dl-progress > div");
  if (fill && total > 0) {
    fill.style.width = `${Math.floor((downloaded / total) * 100)}%`;
  }
});

// ---------------------------------------------------------------
// 文字起こし
// ---------------------------------------------------------------
function updateRunButton() {
  runBtn.disabled = busy || !selectedFile;
}

function setBusy(value) {
  busy = value;
  lamp.classList.toggle("on", value);
  runBtn.classList.toggle("hidden", value);
  cancelBtn.classList.toggle("hidden", !value);
  progressStrip.classList.toggle("hidden", !value);
  modelSelect.disabled = value;
  langSelect.disabled = value;
  translateCheck.disabled = value;
  updateOcrControls(false);
  updateRunButton();
}

runBtn.addEventListener("click", async () => {
  if (!selectedFile || busy) return;

  const modelId = modelSelect.value;
  const model = models.find((m) => m.id === modelId);
  if (!model?.downloaded) {
    showToast(`モデル「${modelId}」が未ダウンロードです。「モデルの管理」からダウンロードしてください。`, true);
    $("model-manager").open = true;
    return;
  }

  setBusy(true);
  currentBaseName = baseName(selectedFile);
  currentSourceName = fileName(selectedFile);
  tapeFill.style.width = "0%";
  progressLabel.textContent = "音声を読み込んでいます…";
  resultsSection.classList.add("hidden");
  segments = [];
  liveSegments = [];
  segmentList.innerHTML = "";

  try {
    segments = await invoke("transcribe", {
      path: selectedFile,
      modelId,
      language: langSelect.value,
      translate: translateCheck.checked,
      ocrEnabled: ocrCheck.checked,
      ocrIntervalSecs: Number(ocrInterval.value),
    });
    renderSegments();
    showToast("文字起こしが完了しました");
    refreshHistory().catch(() => {});
  } catch (e) {
    const msg = String(e);
    // キャンセルはエラーではなく通常の通知として表示する
    showToast(msg, !msg.includes("キャンセル"));
    // 途中までのリアルタイム結果があれば残す(コピー・保存も可能にする)
    if (liveSegments.length > 0) {
      segments = liveSegments;
      renderSegments();
      segCount.textContent = `${segments.length} セグメント(途中まで)`;
    }
  } finally {
    setBusy(false);
  }
});

cancelBtn.addEventListener("click", async () => {
  cancelBtn.disabled = true;
  progressLabel.textContent = "キャンセルしています…";
  try {
    await invoke("cancel_transcribe");
  } finally {
    cancelBtn.disabled = false;
  }
});

const STAGE_LABELS = {
  preparing: "言語判定用モデル(tiny)をダウンロードしています…",
  decoding: "音声をデコードしています…",
  loading_model: "モデルを読み込んでいます…",
  detecting: "言語を判定しています…",
  running: "文字起こしを実行中…",
  extracting_frames: "ffmpegで動画フレームを抽出しています…",
  ocr_running: "画面内の文字を日英OCRしています…",
  merging: "音声と画面テキストをタイムラインに統合しています…",
};
let currentStage = "running";

listen("transcribe-status", (event) => {
  currentStage = event.payload.stage;
  tapeFill.style.width = "0%";
  progressLabel.textContent = STAGE_LABELS[currentStage] ?? currentStage;
});

listen("transcribe-progress", (event) => {
  const p = event.payload;
  tapeFill.style.width = `${p}%`;
  const label = STAGE_LABELS[currentStage] ?? "処理中…";
  progressLabel.textContent = `${label} ${p}%`;
});

// ---------------------------------------------------------------
// 履歴
// ---------------------------------------------------------------
const historyList = $("history-list");
const historyCount = $("history-count");

function formatDate(ms) {
  return new Date(ms).toLocaleString("ja-JP", {
    year: "numeric", month: "2-digit", day: "2-digit",
    hour: "2-digit", minute: "2-digit",
  });
}

async function refreshHistory() {
  const entries = await invoke("list_history");
  historyCount.textContent = entries.length > 0 ? `${entries.length} 件` : "";
  historyList.innerHTML = "";

  if (entries.length === 0) {
    const li = document.createElement("li");
    li.className = "history-empty";
    li.textContent = "まだ履歴はありません。文字起こしを実行すると自動で保存されます。";
    historyList.appendChild(li);
    return;
  }

  for (const h of entries) {
    const li = document.createElement("li");
    li.className = "model-row history-row";

    const name = document.createElement("span");
    name.className = "m-id h-name";
    name.textContent = h.file_name;
    name.title = h.file_name;

    const meta = document.createElement("span");
    meta.className = "m-desc";
    meta.textContent = `${formatDate(h.created_at_ms)} ・ ${h.model_id} ・ ${msToShort(h.duration_ms)}`;

    const openBtn = document.createElement("button");
    openBtn.textContent = "開く";
    openBtn.addEventListener("click", async () => {
      try {
        const entry = await invoke("load_history", { id: h.id });
        segments = entry.segments;
        currentBaseName = baseName(entry.file_name);
        currentSourceName = entry.file_name;
        renderSegments();
        $("save-ocr-text").style.display = "none"; // 履歴から読み込んだ場合はOCR累積テキストは非表示
        resultsSection.scrollIntoView({ behavior: "smooth", block: "start" });
      } catch (e) {
        showToast(String(e), true);
      }
    });

    const delBtn = document.createElement("button");
    delBtn.className = "danger-text";
    delBtn.textContent = "削除";
    delBtn.addEventListener("click", async () => {
      try {
        await invoke("delete_history", { id: h.id });
        showToast("履歴を削除しました");
        await refreshHistory();
      } catch (e) {
        showToast(String(e), true);
      }
    });

    li.append(name, meta, openBtn, delBtn);
    historyList.appendChild(li);
  }
}

// ---------------------------------------------------------------
// 結果表示・エクスポート
// ---------------------------------------------------------------
function makeSegmentLi(s) {
  const li = document.createElement("li");
  li.className = "segment";
  const time = document.createElement("span");
  time.className = "seg-time";
  time.textContent = `${msToShort(s.start_ms)} → ${msToShort(s.end_ms)}`;
  const source = document.createElement("span");
  const isOcr = s.source === "ocr";
  source.className = `seg-source ${isOcr ? "ocr" : "speech"}`;
  source.textContent = isOcr ? "画面" : "音声";
  const text = document.createElement("span");
  text.className = "seg-text";
  text.textContent = s.text;
  li.append(time, source, text);
  return li;
}

function renderSegments() {
  segmentList.innerHTML = "";
  for (const s of segments) {
    segmentList.appendChild(makeSegmentLi(s));
  }
  segCount.textContent = `${segments.length} セグメント`;
  resultsSection.classList.remove("hidden");
  const hasOcr = segments.some(s => s.source === "ocr");
  $("save-ocr-text").style.display = hasOcr ? "" : "none";
}

// 実行中: セグメントが確定するたびにリアルタイムで追記する
listen("transcribe-segment", (event) => {
  if (!busy) return;
  const s = event.payload;
  liveSegments.push(s);
  const nearBottom =
    segmentList.scrollHeight - segmentList.scrollTop - segmentList.clientHeight < 60;
  segmentList.appendChild(makeSegmentLi(s));
  segCount.textContent = `${liveSegments.length} セグメント(処理中)`;
  // 下端付近を見ているときだけ自動スクロールで追従する
  if (nearBottom) segmentList.scrollTop = segmentList.scrollHeight;
});

$("copy-btn").addEventListener("click", async () => {
  if (segments.length === 0) return;
  const text = toTxt();
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // WKWebView(macOS)などで Clipboard API が使えない場合のフォールバック
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
  showToast("テキストをコピーしました");
});

async function saveAs(ext, content) {
  if (segments.length === 0) return;
  const path = await save({
    defaultPath: `${currentBaseName}.${ext}`,
    filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
  });
  if (!path) return;
  try {
    await invoke("save_text_file", { path, content });
    showToast(`${fileName(path)} に保存しました`);
  } catch (e) {
    showToast(String(e), true);
  }
}

$("save-txt").addEventListener("click", () => saveAs("txt", toTxt()));
$("save-srt").addEventListener("click", () => saveAs("srt", toSrt()));
$("save-vtt").addEventListener("click", () => saveAs("vtt", toVtt()));
$("save-md").addEventListener("click", () => saveAs("md", toMarkdown()));

// OCR 累積テキストの保存
$("save-ocr-text").addEventListener("click", async () => {
  try {
    const result = await invoke("get_cumulative_ocr_text");
    if (!result.available) {
      showToast("OCR累積テキストはありません（OCRが未実行、または動画以外のファイルです）", true);
      return;
    }
    const path = await save({
      defaultPath: `${currentBaseName}_ocr_slides.txt`,
      filters: [{ name: "TXT", extensions: ["txt"] }],
    });
    if (!path) return;
    await invoke("save_text_file", { path, content: result.text });
    showToast(`${fileName(path)} に保存しました`);
  } catch (e) {
    showToast(String(e), true);
  }
});

// ---------------------------------------------------------------
// 初期化
// ---------------------------------------------------------------
refreshModels().catch((e) => showToast(String(e), true));
refreshHistory().catch((e) => showToast(String(e), true));
invoke("check_ocr_support")
  .then((support) => {
    ocrAvailable = support.available;
    ocrSupportMessage = support.message;
    updateOcrControls(true);
  })
  .catch((e) => {
    ocrAvailable = false;
    ocrSupportMessage = `画面OCRの確認に失敗しました: ${e}`;
    updateOcrControls(false);
  });
