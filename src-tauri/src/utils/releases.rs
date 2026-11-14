//! 各平台外部工具的官方发行下载地址与校验信息。

use super::source::get_ytdlp_channel;

/// FFmpeg 固定使用的 eugeneware/ffmpeg-static 发行标签。
///
/// 该仓库不发布校验和，因此不能跟随 latest：固定到具体标签，并在
/// `get_ffmpeg_downloads` 中硬编码每个资产的 SHA-256（取自 GitHub release API
/// 的 asset digest）。升级时需同时更新标签与全部摘要。
pub const FFMPEG_STATIC_TAG: &str = "b6.1.1";

/// 获取工具的最新稳定发行页；请求完成后的重定向 URL 包含实际版本标签。
/// yt-dlp 按当前所选通道返回对应仓库地址（stable/nightly/master）；
/// FFmpeg 返回固定标签的发行页，使更新检查与实际安装的版本一致。
pub fn get_tool_latest_release_url(tool: &str) -> Option<String> {
    match tool {
        "yt-dlp" => Some(get_ytdlp_latest_release_url()),
        "deno" => Some("https://github.com/denoland/deno/releases/latest".to_string()),
        "ffmpeg" => Some(format!(
            "https://github.com/eugeneware/ffmpeg-static/releases/tag/{}",
            FFMPEG_STATIC_TAG
        )),
        _ => None,
    }
}

/// 当前通道下 yt-dlp 最新发行页；重定向 URL 包含实际版本标签。
pub fn get_ytdlp_latest_release_url() -> String {
    format!(
        "https://github.com/{}/releases/latest",
        get_ytdlp_channel().repository()
    )
}

/// 当前平台下 yt-dlp 可执行文件在对应通道中的资产名。
/// 三个通道的构建产物使用相同的资产文件名。
pub fn get_ytdlp_asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else {
        "yt-dlp_linux"
    }
}

/// 指定仓库与发行标签下 yt-dlp 可执行文件及其 `SHA2-256SUMS` 的下载地址。
///
/// 两者取自同一个 release，避免 latest 在两次请求之间切换版本。
/// 三个通道（stable/nightly/master 仓库）都随每个 release 发布 `SHA2-256SUMS`。
pub fn get_ytdlp_release_urls(repository: &str, tag: &str) -> (String, String) {
    let base = format!(
        "https://github.com/{}/releases/download/{}",
        repository, tag
    );
    (
        format!("{}/{}", base, get_ytdlp_asset_name()),
        format!("{}/SHA2-256SUMS", base),
    )
}

/// 当前平台与架构对应的 Deno 发行 zip 资产名
pub fn get_deno_asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "deno-aarch64-pc-windows-msvc.zip"
        } else {
            "deno-x86_64-pc-windows-msvc.zip"
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "deno-aarch64-apple-darwin.zip"
        } else {
            "deno-x86_64-apple-darwin.zip"
        }
    } else if cfg!(target_arch = "aarch64") {
        "deno-aarch64-unknown-linux-gnu.zip"
    } else {
        "deno-x86_64-unknown-linux-gnu.zip"
    }
}

/// 指定发行标签下 Deno zip 及其 `.sha256sum` 文件的下载地址。
/// Deno 为每个资产单独发布 `<asset>.sha256sum`。
pub fn get_deno_release_urls(tag: &str) -> (String, String) {
    let asset_url = format!(
        "https://github.com/denoland/deno/releases/download/{}/{}",
        tag,
        get_deno_asset_name()
    );
    let checksum_url = format!("{}.sha256sum", asset_url);
    (asset_url, checksum_url)
}

/// 当前平台 FFmpeg 与 FFprobe 的固定版本下载地址及期望的 SHA-256。
/// 返回值为 `(名称, 下载地址, SHA-256)`。
pub fn get_ffmpeg_downloads() -> [(&'static str, String, &'static str); 2] {
    let (platform, ffmpeg_sha256, ffprobe_sha256) = if cfg!(target_os = "windows") {
        (
            "win32-x64",
            "04e1307997530f9cf2fe35cba2ca7e8875ca91da02f89d6c7243df819c94ad00",
            "3a7e2dc003dc2cd1472827e4c7c4f056ae1ae0ae7c5bbc580c99b49827351ba4",
        )
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            (
                "darwin-arm64",
                "a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584",
                "bb2db6f5d8cef919da12fbf592119a987202a8c060a886f3cab091f9cab90b64",
            )
        } else {
            (
                "darwin-x64",
                "ebdddc936f61e14049a2d4b549a412b8a40deeff6540e58a9f2a2da9e6b18894",
                "fa3add0ce901f7241abe0dfc0155d958fc834aca3f8ce61f87cc712ae669c1e0",
            )
        }
    } else if cfg!(target_arch = "aarch64") {
        (
            "linux-arm64",
            "6bb182d0d75d23028db82e9e4f723ca69b853d055698486e6984ddb2c06fb8ce",
            "d17ae9b4c297d48e2521ba14e417bb0537c6ff77c584cdbcd6bb0d8d0307a2e8",
        )
    } else if cfg!(target_arch = "arm") {
        (
            "linux-arm",
            "0afba4a11110e6e402053e0fc14c33a7eb207d7a588688ae87dba471a0f06c71",
            "e61ef24870ddc3b80aa8b435512a545bd889e4ed4c2fc7c885f7d3dabad2a53f",
        )
    } else {
        (
            "linux-x64",
            "e7e7fb30477f717e6f55f9180a70386c62677ef8a4d4d1a5d948f4098aa3eb99",
            "4f231a1960d83e403d08f7971e271707bec278a9ae18e21b8b5b03186668450d",
        )
    };

    let url = |name: &str| {
        format!(
            "https://github.com/eugeneware/ffmpeg-static/releases/download/{}/{}-{}",
            FFMPEG_STATIC_TAG, name, platform
        )
    };
    [
        ("ffmpeg", url("ffmpeg"), ffmpeg_sha256),
        ("ffprobe", url("ffprobe"), ffprobe_sha256),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_downloads_are_pinned_and_carry_sha256_digests() {
        for (name, url, sha256) in get_ffmpeg_downloads() {
            assert!(url.contains(&format!(
                "/releases/download/{}/{}-",
                FFMPEG_STATIC_TAG, name
            )));
            assert!(!url.contains("/latest/"));
            assert_eq!(sha256.len(), 64);
            assert!(sha256.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert!(get_tool_latest_release_url("ffmpeg")
            .unwrap()
            .ends_with(&format!("/releases/tag/{}", FFMPEG_STATIC_TAG)));
    }

    #[test]
    fn ytdlp_and_deno_assets_and_checksums_come_from_the_same_release() {
        let (asset, sums) = get_ytdlp_release_urls("yt-dlp/yt-dlp-nightly-builds", "2026.11.01");
        let base = "https://github.com/yt-dlp/yt-dlp-nightly-builds/releases/download/2026.11.01/";
        assert_eq!(asset, format!("{}{}", base, get_ytdlp_asset_name()));
        assert_eq!(sums, format!("{}SHA2-256SUMS", base));

        let (zip, checksum) = get_deno_release_urls("v2.9.8");
        assert!(zip.starts_with("https://github.com/denoland/deno/releases/download/v2.9.8/"));
        assert_eq!(checksum, format!("{}.sha256sum", zip));
    }
}
