<p align="center">
  <img src="assets/logo.png" alt="CorpsTex" width="100%">
</p>

<p align="center">
  A Linux desktop interface for <a href="https://github.com/openai/codex">Codex</a> and <a href="https://opencode.ai/">OpenCode</a>.
</p>

<p align="center">
  <a href="https://github.com/M0KA907/CorpsTex/releases">Releases</a> ·
  <a href="https://github.com/M0KA907/CorpsTex/issues">Issues</a> ·
  <a href="LICENSE">License</a>
</p>

### Installation
**[Download CorpsTex from GitHub Releases](https://github.com/M0KA907/CorpsTex/releases/latest)**

**Install with an AI assistant** — Copy this prompt into your coding assistant:

> Install CorpsTex on my Linux x86_64 system using the latest release from https://github.com/M0KA907/CorpsTex/releases. Use the Arch `.pkg.tar.zst` on Arch-based systems or the `.AppImage` otherwise. Verify the download against `SHA256SUMS`, explain what will change, and ask before running commands that require sudo. Do not install Codex or OpenCode for me.

<details>
<summary><strong>Arch Linux</strong></summary>

Download the `.pkg.tar.zst` release file, then run from its download directory:

```bash
sudo pacman -U ./corpstex-*.pkg.tar.zst
```

Launch CorpsTex from your application menu or run `corpstex`.

</details>

<details>
<summary><strong>AppImage</strong></summary>

Download the `.AppImage` release file, then run from its download directory:

```bash
chmod +x CorpsTex_*.AppImage
./CorpsTex_*.AppImage
```

No system-wide installation is required.

</details>

> [!IMPORTANT]
> Install and configure [Codex](https://github.com/openai/codex) or [OpenCode](https://opencode.ai/) separately. CorpsTex uses the runtime already installed on your system.

### Project status

> [!NOTE]
> CorpsTex is in active development. Some runtime capabilities are not yet supported in the desktop interface.

Report bugs and request features through [GitHub Issues](https://github.com/M0KA907/CorpsTex/issues).

---

<p align="center">
  <sub>Linux · Codex / OpenCode · <a href="LICENSE">GPL-3.0</a></sub>
</p>
