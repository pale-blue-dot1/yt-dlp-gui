//! 工具箱文本下载与保存。

use crate::commands::support::build_http_client;
use tauri::State;

use super::save_target::SaveTargets;

/// 下载 URL 文本内容并返回（用于前端获取字幕文本做合并处理）
#[tauri::command]
pub async fn tool_download_text(url: String, proxy: Option<String>) -> Result<String, String> {
    let client = build_http_client(proxy.as_deref())?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("err_download_failed:{}", e))?;

    if !response.status().is_success() {
        return Err(format!("err_download_failed:HTTP {}", response.status()));
    }

    response
        .text()
        .await
        .map_err(|e| format!("err_read_text:{}", e))
}

/// 将文本内容保存到用户刚在 `pick_save_path` 对话框中选择的文件
#[tauri::command]
pub async fn tool_save_text_to_file(
    targets: State<'_, SaveTargets>,
    content: String,
    file_path: String,
) -> Result<(), String> {
    let path = targets.take(&file_path)?;

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("err_create_dir:{}", e))?;
    }

    tokio::fs::write(&path, &content)
        .await
        .map_err(|e| format!("err_save_file:{}", e))?;

    Ok(())
}
