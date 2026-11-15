# Fork notes

This is a fork of [imsyy/yt-dlp-gui](https://github.com/imsyy/yt-dlp-gui) with security hardening on top of upstream: argument-injection defenses for yt-dlp, no shell permissions in the webview, scoped file commands, a strict CSP, checksum-verified tool downloads, an authenticated browser-extension bridge and confirmation before parsing deep links.

## Rules

- Keep the code identical to upstream wherever possible, so upstream merges stay easy.
- Fork-only build settings live in `src-tauri/tauri.fork.conf.json`, which is passed to the build with `--config`. Upstream never touches that file.
- Never push to or open pull requests on upstream from this fork's workflow. Fixes meant for upstream are prepared separately (see "Sending a fix upstream").

## What the override changes

- `plugins.updater.endpoints` points at this fork's releases. As upstream configures it, the app checks upstream's releases and trusts upstream's updater key, so accepting its update prompt would replace a fork build with an upstream release. The fork has no releases, so the startup check fails silently and offers nothing.
- `bundle.createUpdaterArtifacts` is off, because a local build has no updater signing key and would fail while signing update files.

## Deliberately unchanged

- App identifier `com.imsyy.yt-dlp-gui`, deep-link scheme `ytdlp-gui://` and browser bridge port 17654.
- Changing the scheme or port means editing Rust and browser-extension code that upstream also edits, which would cause merge conflicts. Changing the identifier would move the app data folder, so settings and history would not carry over.
- Consequence: a fork build replaces an upstream install. Never install an upstream release over it.

## One-time setup

Install the pnpm version the project pins (`packageManager` in `package.json`), globally with npm:

```bash
npm install -g pnpm@11.12.0
```

- The build runs `pnpm build` itself, so a plain `pnpm` command must be on the PATH; `corepack pnpm` alone is not enough.
- Use the pinned version, not the latest. pnpm 12 refuses to switch to 11.12.0 (it flags that release as broken for the standalone `@pnpm/exe` build) and runs itself instead, which can rewrite `pnpm-lock.yaml`. The npm package of 11.12.0 works fine.
- npm may warn that pnpm's install scripts were skipped; pnpm runs without them.
- When upstream moves the pin, install the new version the same way.

## Building on Windows

1. Install dependencies: `pnpm install --frozen-lockfile`
2. Build the installers: `pnpm tauri build --config src-tauri/tauri.fork.conf.json`
   - The first build downloads the WiX and NSIS bundler tools, and a release build takes about 15 minutes.
   - Installers land in `src-tauri/target/release/bundle/msi/` and `src-tauri/target/release/bundle/nsis/`.
3. Run either installer. They are not code-signed, so Windows SmartScreen warns; choose "More info", then "Run anyway".
4. Browser extension:
   - In the app, open the Browser Extension page and click "Open extension folder".
   - In `chrome://extensions` (or `edge://extensions`), click the reload button on YDL GUI Helper. For a first install, turn on Developer mode and use "Load unpacked" on that folder.
   - Copy the pairing token from the Browser Extension page and paste it into the extension popup's "Pairing token" field. The app ignores extension requests without it.
5. Optional safety net: turn off "Check for updates on startup" in Settings.

## Syncing from upstream

1. `git fetch upstream` (remote `upstream` is https://github.com/imsyy/yt-dlp-gui).
2. `git checkout -b sync/upstream-<date> master`, then `git merge upstream/master`.
3. Resolve conflicts. Where upstream fixes one of the same security issues its own way, prefer upstream's version when it covers the same thing; that shrinks the fork over time.
4. Run the checks below.
5. Push the branch and open a pull request into this fork's `master`. In a fork, `gh pr create` defaults to the parent repository, so always pass `--repo pale-blue-dot1/yt-dlp-gui --base master`.

## Checks

- In `src-tauri/`: `cargo check`, `cargo clippy --all-targets`, `cargo test`
- At the repository root: `pnpm typecheck`, `pnpm lint`, `pnpm test`

## Sending a fix upstream

Branch from `upstream/master`, not from this fork's `master`, so the change carries none of the fork-only files.

## Later: fork releases with in-app updates

Not set up. It would need:

- an updater signing key of your own (`pnpm tauri signer generate`), with its public key in the override's `plugins.updater.pubkey`;
- `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets on the fork;
- `--config src-tauri/tauri.fork.conf.json` added to the build args in `.github/workflows/release.yml`, and `createUpdaterArtifacts` turned back on;
- a version scheme that stays ahead of installed builds, since the updater only installs higher versions and the fork shares upstream's version numbers.

Windows code signing is not configured in `release.yml` at all.
