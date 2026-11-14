//! 下载结果文件操作命令。
//!
//! 这些命令的路径来自 webview，因此只允许作用于任务记录里已知的文件，并且必须
//! 落在该任务的下载目录内：路径先规范化（解析 `..` 与符号链接）再比对目录前缀，
//! 符号链接本身一律不处理，避免借链接逃逸到下载目录之外。

use crate::db::{tasks, DatabaseState};
use std::path::{Path, PathBuf};
use tauri::State;

/// yt-dlp 写盘过程中的临时后缀（未完成的下载与元数据文件）
const TEMP_SUFFIXES: [&str; 2] = [".part", ".ytdl"];

const ERR_NOT_IN_TASK: &str = "err_file_not_in_task";

/// 规范化任务的下载目录；只接受存在的绝对路径。
fn canonical_download_dir(dir: Option<&str>) -> Result<PathBuf, String> {
    let dir = Path::new(dir.unwrap_or_default());
    if !dir.is_absolute() {
        return Err(ERR_NOT_IN_TASK.to_string());
    }
    dir.canonicalize().map_err(|_| ERR_NOT_IN_TASK.to_string())
}

/// 确认 `path` 是位于 `root`（已规范化）内的普通文件。
///
/// 文件不存在时返回 `Ok(None)`；符号链接、目录以及规范化后跑到 `root` 之外的
/// 路径一律拒绝。
fn regular_file_within(path: &Path, root: &Path) -> Result<Option<PathBuf>, String> {
    if !path.is_absolute() {
        return Err(ERR_NOT_IN_TASK.to_string());
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("err_delete_file:{}", error)),
    };
    if !metadata.file_type().is_file() {
        return Err(ERR_NOT_IN_TASK.to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("err_delete_file:{}", e))?;
    if canonical.starts_with(root) {
        Ok(Some(canonical))
    } else {
        Err(ERR_NOT_IN_TASK.to_string())
    }
}

/// 批量检查文件是否存在；只回答任务记录过的输出文件，其余路径一律视为不存在
#[tauri::command]
pub fn check_files_exist(
    db: State<'_, DatabaseState>,
    paths: Vec<String>,
) -> Result<Vec<bool>, String> {
    let known = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        tasks::known_output_files(&conn).map_err(|e| e.to_string())?
    };
    Ok(paths
        .iter()
        .map(|p| known.contains(p) && Path::new(p).exists())
        .collect())
}

/// 删除某个任务的输出文件；路径必须是任务记录的输出文件且位于其下载目录内
#[tauri::command]
pub fn delete_file(db: State<'_, DatabaseState>, path: String) -> Result<(), String> {
    let scope = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        tasks::find_task_by_output_file(&conn, &path).map_err(|e| e.to_string())?
    }
    .ok_or_else(|| ERR_NOT_IN_TASK.to_string())?;
    let root = canonical_download_dir(scope.download_dir.as_deref())?;
    if let Some(file) = regular_file_within(Path::new(&path), &root)? {
        std::fs::remove_file(file).map_err(|e| format!("err_delete_file:{}", e))?;
    }
    Ok(())
}

/// 判断同一目录下的某个文件名是否属于给定输出文件。
///
/// 候选名去掉可选的临时后缀后，必须正好等于 `output_name`，或等于
/// 「输出主干 + `.fNNN.<ext>`」这类 yt-dlp 分片中间产物命名。
///
/// 关键约束：**绝不能退化成「包含标题」**。用标题做子串匹配会让标题为前缀的
/// 另一个任务被误伤——清理任务 `My Video` 时会删掉另一个任务正在下载的
/// `My Video 2.f137.mp4.part`，直接破坏对方的下载。
fn belongs_to_output(candidate: &str, output_name: &str) -> bool {
    let core = TEMP_SUFFIXES
        .iter()
        .find_map(|suffix| candidate.strip_suffix(suffix))
        .unwrap_or(candidate);
    if core == output_name {
        return true;
    }

    let stem = output_name
        .rsplit_once('.')
        .map_or(output_name, |(stem, _)| stem);
    let Some(rest) = core.strip_prefix(stem) else {
        return false;
    };
    // 主干之后必须紧跟一个点，且分片产物的第一段必须为分片格式（如 .f137.mp4 / .f137）
    rest.starts_with('.') && {
        let mut parts = rest[1..].split('.');
        parts
            .next()
            .is_some_and(|f| f.starts_with('f') && f.len() > 1 && f[1..].chars().all(|c| c.is_ascii_digit()))
    }
}

/// 清理指定任务产生的残留文件（`.part`、`.ytdl` 与分片中间产物）。
///
/// 只依据调用方给出的**精确输出路径**推导，因此不会再出现「按标题模糊匹配」
/// 带来的跨任务误删。`known_paths` 为空时不做任何事——没有已知输出就没有
/// 可以安全归属到该任务的残留。所有路径都必须位于该任务的下载目录内。
#[tauri::command]
pub fn clean_task_residual_files(
    db: State<'_, DatabaseState>,
    task_id: String,
    known_paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let scope = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        tasks::get_task_file_scope(&conn, &task_id).map_err(|e| e.to_string())?
    }
    .ok_or_else(|| ERR_NOT_IN_TASK.to_string())?;
    let root = canonical_download_dir(scope.download_dir.as_deref())?;
    Ok(clean_residuals_within(&root, &known_paths))
}

fn clean_residuals_within(root: &Path, known_paths: &[String]) -> Vec<String> {
    let mut deleted = Vec::new();
    let mut visited = std::collections::HashSet::new();

    for raw in known_paths {
        let output = Path::new(raw.trim());
        // 先规范化所在目录，再拼回文件名：输出文件可能尚不存在，
        // 但它的目录必须真实存在且位于下载目录内。
        let (Some(parent), Some(output_name)) = (
            output.parent().filter(|_| output.is_absolute()),
            output.file_name().and_then(|name| name.to_str()),
        ) else {
            continue;
        };
        let Ok(dir) = parent.canonicalize() else {
            continue;
        };
        if !dir.starts_with(root) {
            continue;
        }
        let output_path = dir.join(output_name);

        // 输出文件本身及其临时后缀
        remove_if_file(&output_path, &mut visited, &mut deleted);
        for suffix in TEMP_SUFFIXES {
            remove_if_file(
                &dir.join(format!("{output_name}{suffix}")),
                &mut visited,
                &mut deleted,
            );
        }

        // 同目录下的分片中间产物
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if belongs_to_output(&name, output_name) {
                remove_if_file(&entry.path(), &mut visited, &mut deleted);
            }
        }
    }

    deleted
}

/// 删除普通文件并记录（符号链接不处理）；去重避免同一路径被处理两次
fn remove_if_file(
    path: &Path,
    visited: &mut std::collections::HashSet<String>,
    deleted: &mut Vec<String>,
) {
    if !std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
        return;
    }
    let key = path.to_string_lossy().to_string();
    if visited.insert(key.clone()) && std::fs::remove_file(path).is_ok() {
        deleted.push(key);
    }
}

#[cfg(test)]
mod scope_tests {
    use super::{canonical_download_dir, clean_residuals_within, regular_file_within};
    use std::path::{Path, PathBuf};

    /// 每个测试一个独立目录：<tmp>/ydl-files-<name>-<pid>/{downloads,outside}
    fn sandbox(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("ydl-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let downloads = base.join("downloads");
        let outside = base.join("outside");
        std::fs::create_dir_all(downloads.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        (base, downloads, outside)
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"x").unwrap();
    }

    fn text(path: PathBuf) -> String {
        path.to_string_lossy().to_string()
    }

    #[test]
    fn only_regular_files_inside_the_download_dir_are_accepted() {
        let (base, downloads, outside) = sandbox("within");
        let root = canonical_download_dir(downloads.to_str()).unwrap();
        touch(&downloads.join("sub").join("video.mp4"));
        touch(&outside.join("secret.txt"));

        let inside = regular_file_within(&downloads.join("sub").join("video.mp4"), &root);
        assert!(inside.unwrap().is_some());
        let missing = regular_file_within(&downloads.join("missing.mp4"), &root);
        assert_eq!(missing.unwrap(), None);
        assert!(regular_file_within(&outside.join("secret.txt"), &root).is_err());
        let escaped = downloads.join("..").join("outside").join("secret.txt");
        assert!(regular_file_within(&escaped, &root).is_err());
        assert!(regular_file_within(&downloads.join("sub"), &root).is_err());
        assert!(regular_file_within(Path::new("relative.mp4"), &root).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn download_dir_must_be_an_existing_absolute_directory() {
        assert!(canonical_download_dir(None).is_err());
        assert!(canonical_download_dir(Some("")).is_err());
        assert!(canonical_download_dir(Some("downloads")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_that_escape_the_download_dir_are_rejected() {
        let (base, downloads, outside) = sandbox("symlink");
        let root = canonical_download_dir(downloads.to_str()).unwrap();
        touch(&outside.join("secret.txt"));
        std::os::unix::fs::symlink(outside.join("secret.txt"), downloads.join("link.mp4")).unwrap();
        std::os::unix::fs::symlink(&outside, downloads.join("linked-dir")).unwrap();

        let through_dir = downloads.join("linked-dir").join("secret.txt");
        assert!(regular_file_within(&downloads.join("link.mp4"), &root).is_err());
        assert!(regular_file_within(&through_dir, &root).is_err());
        let removed = clean_residuals_within(
            &root,
            &[text(downloads.join("link.mp4")), text(through_dir)],
        );
        assert!(removed.is_empty());
        assert!(outside.join("secret.txt").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn residual_cleanup_stays_inside_the_download_dir() {
        let (base, downloads, outside) = sandbox("residuals");
        let root = canonical_download_dir(downloads.to_str()).unwrap();
        for name in ["video.mp4.part", "video.f137.mp4", "other.mp4"] {
            touch(&downloads.join(name));
        }
        for name in ["video.mp4.part", "video.f137.mp4"] {
            touch(&outside.join(name));
        }

        let removed = clean_residuals_within(
            &root,
            &[
                text(downloads.join("video.mp4")),
                text(outside.join("video.mp4")),
                text(downloads.join("..").join("outside").join("video.mp4")),
                "video.mp4".to_string(),
            ],
        );

        assert_eq!(removed.len(), 2);
        assert!(!downloads.join("video.mp4.part").exists());
        assert!(!downloads.join("video.f137.mp4").exists());
        assert!(downloads.join("other.mp4").exists());
        assert!(outside.join("video.mp4.part").exists());
        assert!(outside.join("video.f137.mp4").exists());
        let _ = std::fs::remove_dir_all(base);
    }
}

#[cfg(test)]
mod tests {
    use super::belongs_to_output;

    #[test]
    fn matches_the_output_file_and_its_temp_suffixes() {
        assert!(belongs_to_output("video.mp4", "video.mp4"));
        assert!(belongs_to_output("video.mp4.part", "video.mp4"));
        assert!(belongs_to_output("video.mp4.ytdl", "video.mp4"));
    }

    #[test]
    fn matches_yt_dlp_fragment_intermediates() {
        assert!(belongs_to_output("video.f137.mp4", "video.mp4"));
        assert!(belongs_to_output("video.f137.mp4.part", "video.mp4"));
        assert!(belongs_to_output("video.f140.m4a.part", "video.mp4"));
        // 无扩展名的输出模板同样适用
        assert!(belongs_to_output("video.f137", "video"));
    }

    /// 回归测试：标题为前缀的其它任务不得被误伤
    #[test]
    fn does_not_match_a_longer_name_sharing_the_same_prefix() {
        assert!(!belongs_to_output("My Video 2.f137.mp4.part", "My Video.mp4"));
        assert!(!belongs_to_output("My Video 2.mp4.part", "My Video.mp4"));
        assert!(!belongs_to_output("video-extra.mp4.part", "video.mp4"));
        // 主干之后不是合法后缀
        assert!(!belongs_to_output("video.mkv", "video.mp4"));
        assert!(!belongs_to_output("video.", "video.mp4"));
        assert!(!belongs_to_output("videof137.mp4", "video.mp4"));
    }
}
