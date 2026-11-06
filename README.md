<p align="center">
  <img src="assets/readme-header.svg" alt="CorpsTex — a native Linux workspace for Codex and OpenCode" width="100%">
</p>

<p align="center">
  <img alt="Linux" src="https://img.shields.io/badge/Linux-desktop-d97757?style=flat-square&logo=linux&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-363632?style=flat-square&logo=tauri&logoColor=efede5">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-native-363632?style=flat-square&logo=rust&logoColor=efede5">
  <img alt="GPL-3.0-only" src="https://img.shields.io/badge/license-GPL--3.0--only-363632?style=flat-square">
</p>

<p align="center">
  <a href="https://github.com/M0KA907/CorpsTex/releases"><strong>Download</strong></a> &nbsp;·&nbsp;
  <a href="#install">Install</a> &nbsp;·&nbsp;
  <a href="#build-from-source">Build</a> &nbsp;·&nbsp;
  <a href="https://github.com/M0KA907/CorpsTex/actions/workflows/release.yml">Build status</a>
</p>

<br>

**CorpsTex** brings the official Codex and OpenCode runtimes into a native Linux desktop app. Projects, conversations and source files share one workspace, with your runtime's own tools, configuration, permissions and history.

| Workspace | Agent work | Make it yours |
| :--- | :--- | :--- |
| Project sidebar and session tabs | Streaming conversations and rich responses | Palettes, accents and typography |
| Monaco editor with manual saves | Centralized, attributed approvals | Glass or solid surfaces |
| File browser and language-server support | Optional execution timeline and diagnostics | Background effects and reduced motion |

CorpsTex uses your **system-installed** runtime. It does not install a replacement agent, transfer context between runtimes, or collect provider API keys.

<br>

## Install

Pick a reviewed version from **[Releases](https://github.com/M0KA907/CorpsTex/releases)**. Each release includes its test results and `SHA256SUMS`.

### Arch Linux

Download `corpstex-VERSION-1-x86_64.pkg.tar.zst`, then install it:

```sh
sudo pacman -U ./corpstex-VERSION-1-x86_64.pkg.tar.zst
```

The package adds CorpsTex to your application menu and declares its GTK3/WebKitGTK 4.1 dependencies.

### AppImage

Download the `.AppImage`, make it executable and launch:

```sh
chmod +x CorpsTex_VERSION_amd64.AppImage
./CorpsTex_VERSION_amd64.AppImage
```

A `.tar.gz` is also available: extract the folder and run `./corpstex` with GTK3/WebKitGTK 4.1 installed. Linux compatibility still depends on the build's glibc requirements; see [Tauri's compatibility guidance](https://v2.tauri.app/distribute/appimage/).

> **Bring your runtime.** Install [Codex](https://github.com/openai/codex) or [OpenCode](https://opencode.ai/docs/) separately, authenticate through that runtime, and make its executable available on `PATH`.

<details>
<summary><strong>Updates, rollback & removal</strong></summary>

Save editor changes, finish or stop active turns, and close CorpsTex before updating.

- **Arch:** download the newer package and install it with `pacman -U`.
- **AppImage:** replace the old AppImage with the newer download.
- **Portable archive:** replace the entire extracted folder.
- **Rollback:** close the app and reinstall a retained older release.
- **Remove an Arch install:** `sudo pacman -R corpstex`.

Codex/OpenCode keep their own histories, credentials and configuration. CorpsTex settings retain the same application identifier. Unsaved editor buffers and transient diagnostics do not survive exit.

When switching from a manual user-local install to the Arch package, move the old `~/.local/bin/corpstex` launcher and `~/.local/share/applications/io.github.m0ka907.corpstex.desktop` entry aside: they otherwise take precedence over the package. Keep them as rollback copies; do not remove runtime data.

There is no automatic in-app updater. Repository changes become a new download when a version is built and released.

</details>

<br>

## Build from source

**Requirements:** Git, Node.js **22.12+**, npm, Rust, and the [Tauri 2 Linux prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
git clone https://github.com/M0KA907/CorpsTex.git
cd CorpsTex
npm ci
npm run desktop:build
```

The optimized executable is `src-tauri/target/release/corpstex`.

<details>
<summary><strong>Package an AppImage or install a source build</strong></summary>

Generate the dependency notices and build the AppImage:

```sh
python3 scripts/release.py notices
npm run tauri -- build --ci -- --locked
```

Output: `src-tauri/target/release/bundle/appimage/`.

For a user-local executable from your source build:

```sh
install -Dm755 src-tauri/target/release/corpstex "$HOME/.local/bin/corpstex"
"$HOME/.local/bin/corpstex"
```

To update that source installation, close CorpsTex, pull your reviewed changes, run `npm ci` and `npm run desktop:build`, then repeat the installation command. Preserve local changes before pulling.

</details>

## Development

Run the native app with live reload:

```sh
npm run desktop:dev
```

<details>
<summary><strong>Checks & release workflow</strong></summary>

```sh
npm run typecheck
npm run build
npm run rust:fmt
npm run rust:check
npm run rust:lint
npm run rust:test
```

Release builds use **Ubuntu 22.04**, **Rust 1.99.1**, **Node.js 22** and the committed dependency lockfiles. GitHub Actions produces an AppImage, an Arch package and a portable archive.

To publish an update:

1. Change the code and run the checks. Update matching versions in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`; run Cargo to refresh `src-tauri/Cargo.lock`.
2. Commit and push the reviewed source, following the owner's publication rules.
3. Push an annotated version tag such as `v0.1.1`, and create your draft release for that tag before the build finishes.
4. **Actions → Build release** attaches downloads and checksums to that draft. Review the install/launch/connect/exit and upgrade/rollback results, then publish it.

**Run workflow** on a branch produces build-only downloads under Actions. It does not publish a release. Failed checks block attachment, and existing release assets are never overwritten. Use a new version for each published update.

Publication, tags and release uploads require owner authorization. Dependency resolution is locked; bit-identical builds are not claimed.

</details>

<br>

---

<p align="center">
  <strong>Linux desktop · Codex + OpenCode · GPL-3.0</strong><br>
  <a href="LICENSE">Application license</a> &nbsp;·&nbsp; <a href="NOTICE">Third-party notices</a>
</p>

Installers and archives retain the application license, dependency notices and provider notices. Corresponding source is the release's matching repository tag. This repository is private; downloads and source require repository access.
