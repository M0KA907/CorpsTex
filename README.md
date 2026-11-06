<h1 align="center">CorpsTex</h1>

<p align="center">
  A native desktop workspace for <strong>Codex</strong> and <strong>OpenCode</strong>.
</p>

<p align="center">
  <img alt="Version 0.1.0" src="https://img.shields.io/badge/version-0.1.0-555555?style=flat-square">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24C8D8?style=flat-square&logo=tauri&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-native-000000?style=flat-square&logo=rust">
  <img alt="GPL-3.0-only" src="https://img.shields.io/badge/license-GPL--3.0--only-2ea44f?style=flat-square">
  <img alt="Release builds" src="https://img.shields.io/badge/install-release%20builds-555555?style=flat-square">
</p>

<p align="center">
  <a href="https://github.com/M0KA907/CorpsTex/releases">Downloads</a> ·
  <a href="https://github.com/M0KA907/CorpsTex/actions/workflows/release.yml">Build status</a> ·
  <a href="#build-from-source">Build from source</a>
</p>

## Download & install

Choose a reviewed version from **[Releases](https://github.com/M0KA907/CorpsTex/releases)**. Builds remain drafts until their downloads have been reviewed.

| Platform | Download | Install |
| :--- | :--- | :--- |
| Windows x64 | `*-setup.exe` | Run the installer; installs for your user. |
| Windows portable | `CorpsTex-VERSION-windows-x86_64.zip` | Extract the entire folder and run `corpstex.exe`. |
| Arch Linux x64 | `corpstex-VERSION-1-x86_64.pkg.tar.zst` | `sudo pacman -U ./corpstex-VERSION-1-x86_64.pkg.tar.zst` |
| Linux AppImage | `*.AppImage` | Make executable, then launch. |
| Linux portable | `CorpsTex-VERSION-linux-x86_64.tar.gz` | Extract and run `./corpstex`. |

Install **Codex and/or OpenCode separately** and authenticate through that runtime. CorpsTex does not bundle runtimes or collect provider API keys. Windows needs WebView2; Linux needs GTK3/WebKitGTK 4.1. Compare downloads with the release's `SHA256SUMS`.

### Updating an installed copy

Save editor changes, finish or stop active turns, then close CorpsTex. Download the newer installer/package and install it over the previous version. For portable builds, replace the entire extracted folder.

Runtime histories, credentials and configuration stay with Codex/OpenCode. Unsaved editor buffers and live diagnostics do not survive exit. To roll back, close the app and reinstall a retained older version.

## Build from source

**Requirements:** Git, Node.js **22.12+**, npm, Rust, and the [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/).

Install the [Codex CLI](https://github.com/openai/codex) and/or [OpenCode](https://opencode.ai/docs/) separately and ensure its executable is on your `PATH`.

```sh
git clone https://github.com/M0KA907/CorpsTex.git
cd CorpsTex
npm ci
npm run desktop:build
```

This produces an optimized release executable without creating an installer:

- **Linux:** `src-tauri/target/release/corpstex`
- **Windows:** `src-tauri\target\release\corpstex.exe`

To build the platform installer with dependency notices:

```sh
python3 scripts/release.py notices
npm run tauri -- build --ci -- --locked
```

On Windows, use `python` if `python3` is unavailable. The release workflow uses Rust **1.99.1**, Node.js **22**, Ubuntu **22.04** and Windows Server **2022**, with locked npm/Cargo dependencies. Builds on newer Linux systems can require a newer glibc; see [Tauri's compatibility guidance](https://v2.tauri.app/distribute/appimage/).

## Install on Linux

For a manual user-local installation from source, run these commands from the cloned repository:

```sh
install -Dm755 src-tauri/target/release/corpstex \
  "$HOME/.local/bin/corpstex"

install -Dm644 src-tauri/icons/icon.png \
  "$HOME/.local/share/icons/hicolor/32x32/apps/io.github.m0ka907.corpstex.png"

mkdir -p "$HOME/.local/share/applications"

cat > "$HOME/.local/share/applications/io.github.m0ka907.corpstex.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=CorpsTex
Comment=Desktop client for Codex and OpenCode
Exec="$HOME/.local/bin/corpstex"
TryExec=$HOME/.local/bin/corpstex
Icon=io.github.m0ka907.corpstex
Terminal=false
Categories=Development;
StartupNotify=true
EOF

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$HOME/.local/share/applications"
fi
```

No `sudo` is required. Launch **CorpsTex** from your application menu or run:

```sh
"$HOME/.local/bin/corpstex"
```

If `~/.local/bin` is on your `PATH`, you can also run:

```sh
corpstex
```

### Update

Close CorpsTex, then run these commands from the repository and repeat the installation commands above:

```sh
git pull --ff-only
npm ci
npm run desktop:build
```

### Uninstall

```sh
rm -f -- \
  "$HOME/.local/bin/corpstex" \
  "$HOME/.local/share/applications/io.github.m0ka907.corpstex.desktop" \
  "$HOME/.local/share/icons/hicolor/32x32/apps/io.github.m0ka907.corpstex.png"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$HOME/.local/share/applications"
fi
```

Application settings and Codex/OpenCode histories are preserved.

## Windows

The NSIS installer installs for the current user and offers Microsoft's WebView2 bootstrapper if needed; that download requires internet access. The portable ZIP requires WebView2 to be installed already. Runtime discovery needs native `codex.exe` or `opencode.exe` executables on `PATH`; `.cmd` and PowerShell wrappers are unsupported.

See each release's notes for exact Windows/Wine test results. Wine testing does not establish native Windows 11 acceptance. Binaries are currently unsigned; macOS is not an initial release target.

## Publishing an update

Code changes become downloads through **Actions → Build release**:

1. Make your changes and run the project checks. Set matching `major.minor.patch` versions in `package.json`, both root version entries in `package-lock.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`.
2. Commit and push the reviewed source to `main`, following the owner's publication rules.
3. Push an annotated version tag, such as `v0.1.1`, and create your draft release for that tag before the builds finish.
4. The workflow builds both platforms and attaches downloads plus `SHA256SUMS` to your draft. Review install/launch/connect/exit and upgrade/rollback results, then publish it.

**Run workflow** on a branch produces build-only downloads under Actions. It does not create a release. Failed checks block attachment; existing assets are never overwritten. Keep a new version for each published update.

Installed copies update when you install the newer download. No automatic in-app updater is configured. Publication, tags and release uploads require owner authorization.

## Development

To run with live reload:

```sh
npm run desktop:dev
```

## License

[GPL-3.0-only](LICENSE) · [Third-party notices](NOTICE)

Installers and archives include the application license, dependency notices and provider notices. Corresponding source is the matching version tag in this repository.
