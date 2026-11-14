//! 传递给 yt-dlp 的外部工具与插件参数。

use tauri::AppHandle;

use super::paths::{get_ffmpeg_path, get_js_runtime_path, get_plugin_dir};
use super::source::get_js_runtime;

/// 将当前选择的 FFmpeg 目录显式交给 yt-dlp，解决 GUI 进程 PATH 不完整的问题。
pub fn build_ffmpeg_location_args(app: &AppHandle) -> Vec<String> {
    let Ok(ffmpeg_path) = get_ffmpeg_path(app) else {
        return vec![];
    };
    if !ffmpeg_path.exists() {
        return vec![];
    }
    let location = ffmpeg_path
        .parent()
        .unwrap_or(&ffmpeg_path)
        .to_string_lossy()
        .to_string();
    vec!["--ffmpeg-location".to_string(), location]
}

/// 如果插件目录存在，返回 --plugin-dirs 参数
pub fn build_plugin_args(app: &AppHandle) -> Vec<String> {
    if let Ok(plugin_dir) = get_plugin_dir(app) {
        if plugin_dir.exists() {
            return vec![
                "--plugin-dirs".to_string(),
                plugin_dir.to_string_lossy().to_string(),
            ];
        }
    }
    vec![]
}

/// Explicitly select one runtime so yt-dlp's default Deno cannot override it.
pub fn build_js_runtime_args(app: &AppHandle) -> Vec<String> {
    let runtime = get_js_runtime();
    let path = get_js_runtime_path(app).ok();
    runtime_args(runtime, path.as_deref())
}

fn runtime_args(runtime: &str, path: Option<&std::path::Path>) -> Vec<String> {
    let runtime = path
        .map(|path| format!("{}:{}", runtime, path.to_string_lossy()))
        .unwrap_or_else(|| runtime.to_string());
    vec![
        "--no-js-runtimes".to_string(),
        "--js-runtimes".to_string(),
        runtime,
    ]
}

/// 校验交给 yt-dlp 的目标地址必须是带主机名的 http(s) URL。
///
/// 前端已做同样的检查，这里在后端再兜底一次：任何 IPC、深链接或数据库来源的
/// 值都不能以 `-` 开头被 yt-dlp 当成选项，也不能是 `file:` 等其他协议。
pub fn validate_http_url(url: &str) -> Result<(), String> {
    match url::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.has_host() => Ok(()),
        _ => Err("err_invalid_url".to_string()),
    }
}

/// 在参数末尾追加 `--` 和目标 URL。
///
/// yt-dlp 的用法是 `yt-dlp [OPTIONS] [--] URL [URL...]`，`--` 之后的参数一律按
/// URL 处理，因此它必须是参数列表的最后一段，调用方之后不能再追加选项。
pub fn append_url_args<I, S>(args: &mut Vec<String>, urls: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let urls = urls
        .into_iter()
        .map(|url| {
            let url = url.as_ref();
            validate_http_url(url).map(|_| url.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    args.push("--".to_string());
    args.extend(urls);
    Ok(())
}

/// 用户自定义参数中禁止出现的长选项：它们会让 yt-dlp 执行外部程序、加载任意
/// 代码或配置，或改写应用自己传入的工具路径（见仓库根目录 yt-dlp.md）。
/// 自定义参数本身是正常功能，但这些选项会把一次 webview 注入直接变成本机代码执行。
const DENIED_CUSTOM_LONG_OPTIONS: &[&str] = &[
    // 直接执行命令
    "--exec",
    "--exec-before-download",
    "--netrc-cmd",
    // 外部下载器：可指定任意可执行文件路径，aria2c 等的参数还能挂载完成回调
    "--downloader",
    "--external-downloader",
    "--downloader-args",
    "--external-downloader-args",
    // 加载 Python 插件 / 后处理器（含内置的 Exec 后处理器）
    "--plugin-dirs",
    "--use-postprocessor",
    // 配置文件与别名可以间接引入上面任何一个选项
    "--config-locations",
    "--config-location",
    "--alias",
    // 指定要启动的 JS 运行时 / FFmpeg 可执行文件
    "--js-runtimes",
    "--ffmpeg-location",
    // 自更新可指向任意仓库且不做校验
    "--update",
    "--update-to",
];

/// 本身合法、但恰好是上面某个被禁选项前缀的完整长选项（yt-dlp 允许缩写长选项）。
const ALLOWED_PREFIX_OPTIONS: &[&str] = &["--netrc"];

/// 禁止的短选项（`-U` 即 `--update`）。短选项可以合写成 `-xU`，因此逐字符检查。
const DENIED_CUSTOM_SHORT_OPTIONS: &[char] = &['U'];

/// 检查用户自定义的 yt-dlp 参数，拒绝可执行外部命令的选项。
///
/// yt-dlp 基于 optparse，长选项可写作 `--opt=value`，也可缩写为任意无歧义前缀
/// （`--exe` 即 `--exec`），因此按“是否为被禁选项的前缀”判断。参数值若恰好形如
/// 被禁选项也会被拒绝，宁可误拒也不放过。
pub fn validate_custom_args(args: &[String]) -> Result<(), String> {
    for arg in args {
        let denied = if arg == "--" {
            // `--` 会让应用随后追加的选项全部被当成 URL
            true
        } else if let Some(long) = arg.strip_prefix("--") {
            let name = format!("--{}", long.split('=').next().unwrap_or_default()).to_lowercase();
            !ALLOWED_PREFIX_OPTIONS.contains(&name.as_str())
                && DENIED_CUSTOM_LONG_OPTIONS
                    .iter()
                    .any(|denied| denied.starts_with(&name))
        } else if let Some(short) = arg.strip_prefix('-') {
            short
                .chars()
                .any(|c| DENIED_CUSTOM_SHORT_OPTIONS.contains(&c))
        } else {
            false
        };
        if denied {
            return Err(format!("err_custom_arg_not_allowed:{}", arg));
        }
    }
    Ok(())
}

/// 拆分并校验用户自定义的 yt-dlp 参数
pub fn parse_custom_ytdlp_args(input: &str) -> Result<Vec<String>, String> {
    let args = split_command_line_args(input)?;
    validate_custom_args(&args)?;
    Ok(args)
}

/// 解析用户自定义命令行参数字符串为独立的参数列表（支持单双引号与转义）
pub fn split_command_line_args(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut started = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double_quote => {
                started = true;
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                started = true;
                in_double_quote = !in_double_quote;
            }
            '\\' if in_double_quote => {
                if let Some(&next) = chars.peek() {
                    if next == '"' || next == '\\' {
                        current.push(chars.next().unwrap());
                        continue;
                    }
                }
                current.push('\\');
            }
            c if c.is_whitespace() && !in_single_quote && !in_double_quote => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                started = true;
                current.push(c);
            }
        }
    }
    if in_single_quote || in_double_quote {
        return Err("Unclosed quote in custom yt-dlp arguments".to_string());
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_node_exclusively_and_preserves_windows_paths() {
        assert_eq!(
            runtime_args(
                "node",
                Some(std::path::Path::new(r"C:\Program Files\nodejs\node.exe"))
            ),
            [
                "--no-js-runtimes",
                "--js-runtimes",
                r"node:C:\Program Files\nodejs\node.exe"
            ]
        );
        assert_eq!(
            runtime_args("bun", None),
            ["--no-js-runtimes", "--js-runtimes", "bun"]
        );
    }

    #[test]
    fn test_split_command_line_args() {
        let input = r#"--sleep-requests 2 --sleep-interval 10 --user-agent "Mozilla/5.0 (Windows NT 10.0)""#;
        let args = split_command_line_args(input).unwrap();
        assert_eq!(
            args,
            vec![
                "--sleep-requests",
                "2",
                "--sleep-interval",
                "10",
                "--user-agent",
                "Mozilla/5.0 (Windows NT 10.0)"
            ]
        );
    }

    #[test]
    fn preserves_empty_values_and_windows_paths() {
        assert_eq!(
            split_command_line_args(
                r#"--proxy "" -P "temp:C:\My Downloads" --replace-in-metadata title 'ad' ''"#
            )
            .unwrap(),
            [
                "--proxy",
                "",
                "-P",
                r"temp:C:\My Downloads",
                "--replace-in-metadata",
                "title",
                "ad",
                ""
            ]
        );
    }

    #[test]
    fn accepts_only_http_urls_with_a_host() {
        assert!(validate_http_url("https://www.youtube.com/watch?v=abc").is_ok());
        assert!(validate_http_url("http://example.com/video").is_ok());
        assert!(validate_http_url("HTTPS://EXAMPLE.COM/").is_ok());
        for rejected in [
            "",
            "--exec=calc",
            "-o/tmp/x",
            "file:///etc/passwd",
            "ytsearch:cats",
            "javascript:alert(1)",
            "ftp://example.com/file",
            "https://",
            "example.com/video",
        ] {
            assert!(validate_http_url(rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn rejects_exec_type_custom_options_in_every_spelling() {
        for input in [
            "--exec calc",
            "--exec=calc",
            "--exe calc",
            "--EXEC calc",
            "--exec-before-download calc",
            "--netrc-cmd 'echo x'",
            "--netrc-c=echo",
            "--downloader /tmp/evil",
            "--external-downloader aria2c",
            "--downloader-args 'aria2c:--on-download-complete=calc'",
            "--plugin-dirs /tmp/evil",
            "--use-postprocessor Exec:exec_cmd=calc",
            "--config-locations /tmp/evil.conf",
            "--config-location /tmp/evil.conf",
            "--alias get-it,-X '--exec {0}'",
            "--js-runtimes node:/tmp/evil",
            "--ffmpeg-location /tmp/evil",
            "--update-to evil/yt-dlp",
            "-U",
            "-xU",
            "--sleep-requests 1 --exec calc",
            "--sleep-requests 1 -- --print x",
        ] {
            let error = parse_custom_ytdlp_args(input).unwrap_err();
            assert!(error.starts_with("err_custom_arg_not_allowed:"), "{input}");
        }
    }

    #[test]
    fn appends_urls_after_an_end_of_options_separator() {
        let mut args = vec!["-J".to_string()];
        append_url_args(
            &mut args,
            ["https://example.com/a", "https://example.com/b"],
        )
        .unwrap();
        assert_eq!(
            args,
            ["-J", "--", "https://example.com/a", "https://example.com/b"]
        );
    }

    #[test]
    fn appends_nothing_when_any_url_is_invalid() {
        let mut args = vec!["-J".to_string()];
        assert!(append_url_args(&mut args, ["https://example.com/a", "--exec=calc"]).is_err());
        assert_eq!(args, ["-J"]);
    }

    #[test]
    fn keeps_ordinary_custom_options() {
        for input in [
            "--sleep-requests 2 --sleep-interval 10",
            "--no-exec",
            "--netrc",
            "-n --netrc-location /home/me/.netrc",
            "--user-agent \"Mozilla/5.0\"",
            "-x --audio-format mp3",
            "-P \"temp:C:\\My Downloads\"",
            "--extractor-args youtube:player_client=web",
            "--embed-subs",
        ] {
            assert!(parse_custom_ytdlp_args(input).is_ok(), "{input}");
        }
    }

    #[test]
    fn rejects_unclosed_quotes_instead_of_silently_changing_arguments() {
        assert!(split_command_line_args("--proxy \"unfinished").is_err());
        assert!(split_command_line_args("--user-agent 'unfinished").is_err());
    }
}
