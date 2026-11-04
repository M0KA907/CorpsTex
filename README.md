<h1 align="center">CorpsTex</h1>

<p align="center">
  A native desktop workspace for <strong>Codex</strong> and <strong>OpenCode</strong>.
</p>

<p align="center">
  <img alt="Version 0.1.0" src="https://img.shields.io/badge/version-0.1.0-555555?style=flat-square">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24C8D8?style=flat-square&logo=tauri&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-native-000000?style=flat-square&logo=rust">
  <img alt="GPL-3.0-only" src="https://img.shields.io/badge/license-GPL--3.0--only-2ea44f?style=flat-square">
  <img alt="Source build" src="https://img.shields.io/badge/install-source%20build-555555?style=flat-square">
</p>

## Build from source

**Requirements:** Git, Node.js **22.12+**, npm, Rust, and the [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/).

Install the [Codex CLI](https://github.com/openai/codex) and/or [OpenCode](https://opencode.ai/docs/) separately and ensure its executable is on your `PATH`.

```sh
git clone https://github.com/M0KA907/CorpsTex.git
cd CorpsTex
npm ci
npm run desktop:build
```

This produces an optimized release executable without creating an installer.

## Install on Linux

Run these commands from the cloned repository:

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

The release executable is built at:

```text
src-tauri\target\release\corpstex.exe
```

This build does not create an installer. Windows builds and native behavior remain unverified. macOS is not an initial release target.

## Development

To run with live reload:

```sh
npm run desktop:dev
```

## License

[GPL-3.0-only](LICENSE)
