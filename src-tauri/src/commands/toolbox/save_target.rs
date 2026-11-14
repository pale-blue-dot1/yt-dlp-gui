//! 由后端打开的“另存为”对话框与一次性保存目标。
//!
//! webview 不能再直接指定写盘路径：`pick_save_path` 在 Rust 侧打开系统对话框，
//! 并把用户选中的路径登记为一次性保存目标；`tool_save_*` 命令只接受登记过的路径，
//! 用过即作废。这样即便 webview 被注入脚本，也只能写入用户刚在对话框里确认的文件。

use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// 对话框返回、尚未被保存命令使用的路径
#[derive(Default)]
pub struct SaveTargets(Mutex<HashSet<PathBuf>>);

impl SaveTargets {
    fn register(&self, path: PathBuf) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(path);
        Ok(())
    }

    /// 取出并作废一个保存目标；未经对话框登记的路径一律拒绝。
    pub(crate) fn take(&self, path: &str) -> Result<PathBuf, String> {
        let path = PathBuf::from(path);
        let registered = self.0.lock().map_err(|e| e.to_string())?.remove(&path);
        if registered {
            Ok(path)
        } else {
            Err("err_save_path_not_allowed".to_string())
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SaveDialogFilter {
    name: String,
    extensions: Vec<String>,
}

/// 打开系统“另存为”对话框，返回用户选择的路径（取消时为 `None`）。
///
/// `default_path` 只取文件名部分作为建议名，目录完全由用户在对话框中决定。
#[tauri::command]
pub async fn pick_save_path(
    app: AppHandle,
    targets: State<'_, SaveTargets>,
    title: Option<String>,
    default_path: Option<String>,
    filters: Option<Vec<SaveDialogFilter>>,
) -> Result<Option<String>, String> {
    let mut dialog = app.dialog().file();
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    if let Some(title) = title {
        dialog = dialog.set_title(title);
    }
    if let Some(name) = default_path
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .and_then(|name| name.to_str())
    {
        dialog = dialog.set_file_name(name);
    }
    for filter in filters.unwrap_or_default() {
        let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(filter.name, &extensions);
    }

    let (sender, receiver) = tokio::sync::oneshot::channel();
    dialog.save_file(move |path| {
        let _ = sender.send(path);
    });
    let Some(selected) = receiver.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|e| format!("err_save_file:{}", e))?;
    targets.register(path.clone())?;
    Ok(Some(path.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::SaveTargets;

    #[test]
    fn only_registered_paths_can_be_used_and_only_once() {
        let targets = SaveTargets::default();
        let chosen = std::env::temp_dir().join("chosen.srt");
        targets.register(chosen.clone()).unwrap();

        assert!(targets.take("/etc/passwd").is_err());
        assert_eq!(targets.take(&chosen.to_string_lossy()).unwrap(), chosen);
        assert!(targets.take(&chosen.to_string_lossy()).is_err());
        assert!(targets.take("").is_err());
    }
}
