# CorpsTex

A local-first desktop client for the official Codex and OpenCode agent runtimes.
M00–M06 and OC01 are verified on Linux. The workspace uses session tabs with
one visible conversation view; split panes are excluded by owner decision. The Tauri 2 + React application connects
project folders to app-bound servers: a Codex App Server or an OpenCode ACP
process, one Rust-owned process per canonical project/runtime, shared across that
project's windows and sessions. Either runtime must already be installed;
CorpsTex does not bundle, install, or replace it. Switching runtime keeps separate
native histories and does not transfer agent context.

Available today: saved native threads, text and image conversations, streaming
Markdown/GFM replies (code, tables, charts, local images), exact-turn steering for
Codex, cancellation, model/effort selection, a searchable runtime/model picker, and
a centralized approval center across projects. OpenCode authentication is
managed entirely through OpenCode (`opencode auth login`); CorpsTex does not
collect provider keys. Settings offers login guidance and an explicit idle-only
connection refresh. The execution
timeline (M05) has transient live events, project/thread/turn filters and native-history
snapshots. Native timeline/OpenCode GUI interaction and Windows remain unverified.
M10 adds Windows NSIS/portable ZIP, Linux AppImage/archive and Arch package
generation. See [Downloads and updates](#downloads-and-updates) for the release
procedure. Build success alone does not establish platform acceptance.

Codex audit/hardening (M09) adds fork/rename/archive/restore/compact/review controls,
reported plan/diff and expandable tool details, same-project subagent history via
All sources, and a read-only MCP status/tool catalog under Settings. The
local versioned capability matrix lists tested behavior and residual limitations.
Unsupported server requests remain visible without fabricated responses. This is
not a full CLI-parity claim; new native
interactions, Windows and inference-dependent checks remain explicitly unverified.

## Local development

Prerequisites: Node.js 22.12 or newer, npm, a stable Rust toolchain with Cargo,
and the [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/).
Linux requires a graphical session, GTK 3, WebKitGTK 4.1, and the documented
build packages. Windows 11 requires the MSVC C++ build tools, a compatible Rust
MSVC toolchain, and WebView2. A separately installed official `codex` executable
must be available on an absolute entry in the desktop process's PATH. Windows
discovery currently requires `codex.exe`; `.cmd`/PowerShell wrappers are not
supported. To use the OpenCode runtime, an official `opencode` executable must
likewise be installed and discoverable; it is optional and Codex-only use needs
no OpenCode installation.

```sh
npm ci
npm run desktop:dev
```

`npm run desktop:dev` starts Vite on `127.0.0.1:1420` and launches the native
window. Close the window and stop the development command when finished.
`npm run dev` runs only the browser frontend; it does not verify native behavior.

Both dependency lockfiles are present; use `npm ci` for repeat frontend installs.
Frontend type-checking, tests, and production build pass on the supplied host.
Rust check, Clippy, formatting, and the Rust test command also pass. The Rust
tests include credential-free transport, concurrency, routing, and cleanup fixtures.
The owner confirmed the M00 shell launch. M03 also has agent-observed native
Linux connection, thread creation, and text conversation smoke evidence. The native release build also
passes and produces `src-tauri/target/release/corpstex` on Linux. Restricted
development environments may require explicit permission for localhost or GUI
access. Windows and detailed native visual/focus checks are recorded separately.

## Checks and builds

```sh
npm run typecheck
npm test
npm run build
npm run rust:fmt
npm run rust:check
npm run rust:lint
npm run rust:test
npm run desktop:build
```

Tests compile the real shell with Vite's SSR build and use Node's built-in test
runner to check its disconnected/empty states and inaccessible messaging.
They do not verify WebView rendering, focus behavior, or a native window.
There is no separate frontend linter configured; TypeScript strict checks and
Rust formatting/Clippy are available. `desktop:build` creates a release binary
without bundling installers. Expected native outputs are
`src-tauri/target/release/corpstex` on Linux and
`src-tauri/target/release/corpstex.exe` on Windows.

Installer builds also need generated dependency notices:

```sh
python3 scripts/release.py notices
npm run tauri -- build --ci -- --locked
```

The Linux build generates an AppImage; Windows generates a per-user NSIS
installer. The Windows installer offers installation of Microsoft's WebView2
bootstrapper if needed; that requires network access. Neither installer installs
Codex/OpenCode. The workflow uses pinned action revisions and Rust 1.99.1,
locked npm/Cargo dependencies, and Ubuntu 22.04 or Windows Server 2022 builders.
Dependency resolution is repeatable; bit-identical binaries are not claimed.
Tauri's [packaging](https://v2.tauri.app/distribute/) and
[Windows installer](https://v2.tauri.app/distribute/windows-installer/) guides
describe the platform requirements.

## Downloads and updates

Open this repository's **Releases** page and choose a reviewed version:

- **Windows x64:** download `*-setup.exe` and run it. Alternatively, extract the
  Windows ZIP and run `corpstex.exe`; WebView2 must already be installed.
- **Arch Linux x64:** download `corpstex-VERSION-1-x86_64.pkg.tar.zst`, then run
  `sudo pacman -U ./corpstex-VERSION-1-x86_64.pkg.tar.zst`. This registers the
  application and desktop launcher with pacman; GTK3/WebKitGTK 4.1 dependencies
  must be available from your configured repositories.
- **Other Linux x64:** make the AppImage executable and launch it, or extract
  the `.tar.gz` and run its `corpstex` binary with GTK3/WebKitGTK 4.1 installed.
  The build's glibc floor still applies; an AppImage is not proof that every
  distribution works. See Tauri's [Linux compatibility guidance](https://v2.tauri.app/distribute/appimage/).

Compare the download's SHA-256 with `SHA256SUMS` attached to that same release.
Unsigned binaries may trigger platform trust prompts; no signing certificate or
automatic updater is configured. This private repository's downloads require
repository access.

To update, save editor changes, finish or stop turns, and close CorpsTex. Run the
newer Windows installer or `pacman -U` package; replace the whole extracted folder
for a portable build. Runtime-owned history, credentials and configuration remain
with Codex/OpenCode. CorpsTex preferences use the unchanged application identifier.
Unsaved editor buffers, live diagnostics and active work do not survive exit.
To roll back, close the app and install a retained older version (Arch may require
an explicit package downgrade). Do not delete runtime configuration/history.

### Producing the next version

Changing repository files does not update an installed copy automatically:

1. Change the code and run the checks. Set the same numeric version in
   `package.json`, both root version entries in `package-lock.json`,
   `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`.
2. Commit/push the reviewed source to main using the owner's publication rules.
3. Create and push an annotated tag matching the version, for example
   `git tag -a v0.1.1 -m 'CorpsTex 0.1.1'` and `git push origin refs/tags/v0.1.1`.
   Create an owner-authored **draft** release for that tag before the build ends.
4. **Actions → Build release** builds both platforms and attaches successful
   downloads/checksums to that draft. Review the artifacts, install/launch/connect/
   exit and upgrade/rollback smoke results, then publish the draft yourself.

The workflow also supports **Run workflow** on a branch for build-only artifacts
under Actions; it does not create a release or publish a draft. Version mismatches,
failed checks, a missing draft or an existing asset prevent attachment. Download
files are immutable per version; fix source and choose a new version instead of
overwriting a published release. Tag and release actions require owner authorization.

Review the window at its default size and minimum size, keyboard-focus the
skip link, and verify all three empty regions and the disabled message field.
For a narrow browser smoke check, use a 390px viewport and check that content
stacks without horizontal overflow. Windows and Linux must be tested separately.
See the release notes for actual platform results, not expected outcomes.

## Project connection smoke check

From a normal terminal, launch `npm run desktop:dev` (or run the release binary).
Select **New conversation**, type a message, then **Send message**. The app uses
the folder it was launched from, your existing Codex login, and Codex defaults.
Open **Project folder** to choose a different absolute directory. Empty input
now gives a visible inline error instead of a silent browser validation stop.

Saved threads for the current folder appear in the sidebar. Select one to resume
it and load native history; **More threads** and **Load older turns** paginate.
**Model** expands selectors populated from Codex's catalog. **Stop turn** requests
an interruption; only a reported terminal status confirms its outcome. Messages
render Markdown/GFM: headings, lists, emphasis, fenced code, blockquotes, task
lists, tables, interactive `chart` blocks, and local images. Raw HTML stays text
and remote images do not load automatically; see
[Rich responses and questions](#rich-responses-and-questions). New empty threads
stay visible even before the runtime materializes them in stored history.

A server crash must show a connection error; **Review reconnect** warns that a
new server cannot automatically resume interrupted work. Closing the last window
shows an exit confirmation with reported turns and pending requests. **Keep
CorpsTex open** cancels exit; **Stop servers and exit** stops/reaps servers first.
Open **Approvals** to review requests across every active project. Select a
request to see its project/thread/turn, typed request ID, command or file details,
and permission context. Responses are validated in Rust against the outstanding
request and its server generation. Closing the panel sends no decision. Lost
connections retain requests visibly and cannot receive replies. Exit confirmation
freezes replies; cancelling exit restores access.

Commands support one-time allow, decline, and cancel when offered by the server;
file approvals and legacy approvals use their respective protocol responses.
Permission grants echo the exact requested access for the current turn. Input
questions support explicit answers. Persistent/session policy amendments are not
exposed. MCP form/URL requests offer decline/cancel only. Unknown requests remain
visible without a fabricated response. Trusted approval controls use a closed
shadow root outside the theme shell and a separate fixed-color stylesheet. Linux
keyboard, layout, and adversarial-theme checks passed. A stalled protocol write
fails the connection after two seconds; a possibly partial decision is never
retried. Lost-connection context identifies the uncertain response outcome.
See the release notes for exact evidence and remaining platform/protocol gaps.

Two UI probes exist and are **manual**; neither is part of `npm test`.

`tests/approvals-webkit.py` uses the existing Linux system PyGObject, WebKitGTK
4.1, and xdotool tools in an X11/XWayland desktop session:

```sh
npm run build
python3 tests/approvals-webkit.py
```

It uses synthetic bridge data, opens its own keyboard-test window, and saves
screenshots to a printed temporary directory. Run GUI probes sequentially. Fixed
web surfaces check 640×480 and 390px layouts inside the normally tiled window;
the probe does not change window-manager rules or positioning. No dependencies
are installed by this probe.

`tests/oc01-ui-check.mjs` is a headless Chromium CDP probe for runtime/model
pickers, the free-model shortlist and filters, keyboard navigation, the Settings
authentication guidance/explicit refresh, Stop attribution, and 390×844/1280×900 layout. Prerequisites:

- a Chromium instance exposing a CDP endpoint (default `http://127.0.0.1:9451`,
  override with `CORPSTEX_CDP`);
- a preview server for the built `dist` output (default
  `http://127.0.0.1:1433/`, override with `CORPSTEX_APP`), for example
  `npm run preview -- --port 1433`;
- `npm run build` first so the preview serves current assets.

```sh
node tests/oc01-ui-check.mjs
```

Both probes use synthetic bridge data. Native Tauri and installed-runtime checks
are recorded separately in the milestone ledger.

Event delivery uses a bounded in-memory transport buffer with explicit gap
reporting. Outstanding requests remain separate from that buffer and receive no
automatic response. Under D11 option A, the timeline adds no local event storage;
older activity comes only from loaded runtime-native conversation snapshots.
The normal Codex home is inherited unchanged. Isolated test homes are used only
for the documented installed-runtime smoke tests in `MILESTONES.md`.

## Structure and boundaries

- `src-tauri/`: installed executable discovery, stdio protocol transport, per-project
  and per-runtime server registries, window-bound event access, approvals, image
  validation, OpenCode ACP, and confirmed application shutdown.
  React invokes a fixed, typed command surface for connection/state/exit plus
  conversation, approval, image, folder-browser, and OpenCode
  operations; it cannot invoke a shell or pass arbitrary process arguments.
- `src/`: semantic, responsive workspace shell, conversation/composer UI, Markdown
  and chart rendering, runtime/model pickers, Settings, and project connection
  status. Model choices and the last folder use device localStorage; conversation
  history is not persisted locally.
- `src/theme/tokens.css`: semantic color, typography, spacing, and surface tokens
  for the restrained Gruvbox Dark-inspired default. Components consume tokens.
  Theme manifests, import/export, and custom CSS belong to M08.
- `tests/`: automated SSR/shell regressions (`shell.test.mjs`, run by `npm test`)
  plus the two manual probes `approvals-webkit.py` and `oc01-ui-check.mjs`.

## License and dependency scope

[GPL-3.0](LICENSE). The license file is preserved verbatim from
`M0KA907/CorpsTex`, blob `f288702d2fa16d3cdf0035b15a9fcbc552cd88e7`.
React/React DOM, Vite, and DefinitelyTyped declarations use MIT licensing;
TypeScript uses Apache-2.0; Tauri uses MIT OR Apache-2.0. These direct dependency
licenses are compatible with GPL-3.0. `react-markdown@10.1.0` and
`remark-gfm@4.0.1` were added after the M00 license audit and are recorded here
now: both are MIT (verified from `node_modules`) and compatible with
`GPL-3.0-only`. npm lockfile license metadata was inspected:
MIT, Apache-2.0, MIT OR Apache-2.0, MPL-2.0, ISC, and BSD-3-Clause. Linux Rust
dependency license metadata was also inspected with platform-filtered Cargo
metadata; the resolved graph uses permissive and MPL-2.0 licenses alongside
CorpsTex's GPL-3.0. This metadata review is not a complete release/license audit.

`python3 scripts/release.py notices` regenerates [NOTICE](NOTICE) from installed
locked npm dependencies and both Linux/Windows Cargo metadata graphs. It includes
upstream license/copyright texts and Monaco's bundled component notices. Exact
upstream copies in `packaging/licenses/` cover crates whose archives omitted their
workspace license files; dependency upgrades without available notices fail.
Installers and portable downloads retain LICENSE, NOTICE and provider notices.
Source for a release is its matching repository tag, including build instructions
and both lockfiles. Preserve access to that source for recipients.

The JavaScript manifests select stable React 19, Vite 8, TypeScript 7, and Tauri 2
release lines. Installed versions: React/React DOM 19.3.1, Vite 8.3.5, TypeScript
7.0.2, Tauri CLI 2.12.4, and React/React DOM declarations 19.3.0. Rust dependencies
resolve to Tauri 2.12.5 and tauri-build 2.7.4; Linux compilation checks pass.

## Rollback

Preserve the working checkout, including local editor/tests/handoff changes.
Review `git status --short` and the exact diff before reverting source. On a clean
up-to-date publishing checkout, a targeted `git revert COMMIT` keeps history;
publication of that revert requires separate authorization and Pushguard.

```sh
git revert <commit>          # in a clean checkout, undo one reviewed commit
```

Working source through `83f475e` is published to private main. Local development
notes/tests may differ from that published tree. After reverting source changes,
rebuild only if you need the generated web
assets or native binary to match (`npm run build` or
`CARGO_NET_OFFLINE=true npm run desktop:build`); documentation-only changes need
no rebuild.

Older `/tmp` snapshots are non-durable. Retain release downloads and their
checksums for binary rollback; never reset the entire dirty checkout.
## Rich responses and questions

Codex replies render Markdown headings, lists, emphasis, code blocks, blockquotes,
links, task lists and tables. User messages remain literal text. Raw HTML is
displayed as text, and remote images do not load automatically. Only HTTP(S)
links and anchors become clickable. Unchanged replies reuse their rendered output
during conversation polling.

For an interactive bar or line chart, ask Codex to return JSON in a fenced block
whose language is `chart`, for example:

````markdown
```chart
{"type":"bar","title":"Completed tasks","data":[{"label":"Monday","value":4},{"label":"Tuesday","value":7}]}
```
````

Charts support hover inspection, a keyboard-accessible point selector, an exact
data table, source inspection and an expanded preview. Values may be positive,
zero or negative. Each chart accepts 1–200 labelled finite values between -1e12
and 1e12; source is limited to 131072 characters. Invalid chart data stays
inspectable as source. During streaming, chart blocks remain code until the
response finishes. Other code languages remain ordinary code blocks. Arbitrary
JavaScript/HTML previews, Mermaid, math typesetting and 3D are not integrated.

Codex questions use labelled answer cards with descriptions, native radio-keyboard
navigation, a custom-answer choice when the request permits one, and multiline
text fields. Nothing is preselected. Complete every question, review the answered
count, then choose **Send answers**. Closing a panel sends no response. Secret
free-text answers use a password field; stopped-server and exit restrictions still
apply. Project/request attribution remains available in Request context.
