// Model download with streaming progress and resume support

use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

static CANCEL: AtomicBool = AtomicBool::new(false);

const HF_RESOLVE: &str = "https://huggingface.co";

struct ModelFile {
    name: &'static str,
    repo: &'static str,
    path: &'static str,
    dest: &'static str,
    size_bytes: u64,
}

const DOWNLOADS: &[ModelFile] = &[
    ModelFile {
        name: "TranslateGemma 4B",
        repo: "mradermacher/translategemma-4b-it-GGUF",
        path: "translategemma-4b-it.Q4_K_M.gguf",
        dest: "translategemma-4b-it.Q4_K_M.gguf",
        size_bytes: 2_489_909_760,
    },
    ModelFile {
        name: "Kokoro TTS (fp32)",
        repo: "onnx-community/Kokoro-82M-v1.0-ONNX",
        path: "model.onnx",
        dest: "kokoro/model.onnx",
        size_bytes: 325_532_232,
    },
    ModelFile {
        name: "Kokoro TTS (quantized)",
        repo: "onnx-community/Kokoro-82M-v1.0-ONNX",
        path: "model_quantized.onnx",
        dest: "kokoro/model_quantized.onnx",
        size_bytes: 92_361_116,
    },
    ModelFile {
        name: "Kokoro tokenizer",
        repo: "onnx-community/Kokoro-82M-v1.0-ONNX",
        path: "tokenizer.json",
        dest: "kokoro/tokenizer.json",
        size_bytes: 3_497,
    },
    ModelFile {
        name: "Voice: ef_dora (Spanish female)",
        repo: "onnx-community/Kokoro-82M-v1.0-ONNX",
        path: "voices/ef_dora.bin",
        dest: "kokoro/voices/ef_dora.bin",
        size_bytes: 523_776,
    },
    ModelFile {
        name: "Voice: em_alex (Spanish male)",
        repo: "onnx-community/Kokoro-82M-v1.0-ONNX",
        path: "voices/em_alex.bin",
        dest: "kokoro/voices/em_alex.bin",
        size_bytes: 523_776,
    },
    ModelFile {
        name: "Voice: af_heart (English female)",
        repo: "onnx-community/Kokoro-82M-v1.0-ONNX",
        path: "voices/af_heart.bin",
        dest: "kokoro/voices/af_heart.bin",
        size_bytes: 523_776,
    },
    ModelFile {
        name: "Voice: af_bella (English female)",
        repo: "onnx-community/Kokoro-82M-v1.0-ONNX",
        path: "voices/af_bella.bin",
        dest: "kokoro/voices/af_bella.bin",
        size_bytes: 523_776,
    },
];

#[derive(Serialize, Clone)]
pub struct MissingModel {
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct DownloadProgress {
    pub name: String,
    pub bytes_downloaded: u64,
    pub bytes_total: u64,
    pub overall_bytes_downloaded: u64,
    pub overall_bytes_total: u64,
}

/// Check which models are missing from the data directory.
pub fn check_models(data_dir: &Path) -> Vec<MissingModel> {
    let models_dir = data_dir.join("models");
    DOWNLOADS
        .iter()
        .filter(|m| !models_dir.join(m.dest).exists())
        .map(|m| MissingModel {
            name: m.name.to_string(),
            size_bytes: m.size_bytes,
        })
        .collect()
}

/// Download all missing models with progress events.
pub async fn download_models(
    data_dir: PathBuf,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    CANCEL.store(false, Ordering::SeqCst);

    let models_dir = data_dir.join("models");
    let missing: Vec<&ModelFile> = DOWNLOADS
        .iter()
        .filter(|m| !models_dir.join(m.dest).exists())
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    let overall_total: u64 = missing.iter().map(|m| m.size_bytes).sum();

    // Account for bytes already in .partial files (resume support)
    let mut overall_downloaded: u64 = missing
        .iter()
        .map(|mf| {
            let partial = models_dir.join(format!("{}.partial", mf.dest));
            std::fs::metadata(&partial).map(|meta| meta.len()).unwrap_or(0)
        })
        .sum();

    let client = reqwest::Client::new();

    for model in &missing {
        if CANCEL.load(Ordering::SeqCst) {
            return Err("Download cancelled".into());
        }

        let dest = models_dir.join(model.dest);
        let partial = models_dir.join(format!("{}.partial", model.dest));

        // Ensure parent directory exists
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir failed: {e}"))?;
        }

        let url = format!(
            "{}/{}/resolve/main/{}",
            HF_RESOLVE, model.repo, model.path
        );

        // Resume support: check existing partial download
        let existing_bytes = tokio::fs::metadata(&partial)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let mut request = client.get(&url);
        if existing_bytes > 0 {
            request = request.header("Range", format!("bytes={}-", existing_bytes));
            log::info!(
                "[download] Resuming {} from {} bytes",
                model.name,
                existing_bytes
            );
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed for {}: {e}", model.name))?;

        if !response.status().is_success() && response.status().as_u16() != 206 {
            return Err(format!(
                "HTTP {} for {}",
                response.status(),
                model.name
            ));
        }

        let content_length = response.content_length().unwrap_or(0);
        let total_size = if response.status().as_u16() == 206 {
            existing_bytes + content_length
        } else {
            content_length
        };

        // Open file for append (resume) or create
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(existing_bytes > 0 && response.status().as_u16() == 206)
            .write(true)
            .truncate(existing_bytes == 0 || response.status().as_u16() != 206)
            .open(&partial)
            .await
            .map_err(|e| format!("Failed to open {}: {e}", partial.display()))?;

        let mut stream = response.bytes_stream();
        let mut downloaded = existing_bytes;

        while let Some(chunk) = stream.next().await {
            if CANCEL.load(Ordering::SeqCst) {
                return Err("Download cancelled".into());
            }

            let chunk = chunk.map_err(|e| format!("Download error for {}: {e}", model.name))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Write error: {e}"))?;

            downloaded += chunk.len() as u64;
            overall_downloaded += chunk.len() as u64;

            // Emit progress every ~100KB to avoid flooding
            if downloaded % (100 * 1024) < chunk.len() as u64 || downloaded >= total_size {
                let _ = app_handle.emit(
                    "download-progress",
                    DownloadProgress {
                        name: model.name.to_string(),
                        bytes_downloaded: downloaded,
                        bytes_total: total_size,
                        overall_bytes_downloaded: overall_downloaded,
                        overall_bytes_total: overall_total,
                    },
                );
            }
        }

        file.flush().await.map_err(|e| format!("Flush error: {e}"))?;
        drop(file);

        // Validate downloaded size before accepting
        let actual_size = tokio::fs::metadata(&partial)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        if actual_size != model.size_bytes {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(format!(
                "{}: expected {} bytes, got {} — file removed, retry download",
                model.name, model.size_bytes, actual_size
            ));
        }

        // Rename .partial to final path
        tokio::fs::rename(&partial, &dest)
            .await
            .map_err(|e| format!("Rename failed: {e}"))?;

        log::info!("[download] {} complete ({} bytes)", model.name, downloaded);
    }

    let _ = app_handle.emit("download-complete", ());
    Ok(())
}

/// Cancel an in-progress download.
pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}
