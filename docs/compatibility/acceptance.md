# CLI and window parity acceptance

The regular tests use sanitized Mac action metadata, scripted protocol peers and
headless Rune windows. They require no Mac, SSH login or personal Rex server.
Protocol tests bind ephemeral loopback sockets; the workstation must allow those
connections. Run serially because font fixtures use temporary files:

```sh
cargo test --locked --all-features -- --test-threads=1
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked --release --bin rex-linux
```

The live scripts below each create a disposable Rex API 12+ server, unique Unix
socket, HOME/XDG/config/state directories and private display. They disable server
autostart, tailnet discovery and update checks, ignore inherited Rex targeting,
and never send input to the user's display. Owned process IDs and return codes,
logs and captures remain under the printed cache directory. Only their own
processes are stopped and reaped. Read each script's docstring for dependencies.

```sh
export REX_BIN="$PWD/target/release/rex-linux"
python3 scripts/remote-control-acceptance.py
python3 scripts/workspace-control-acceptance.py
python3 scripts/desktop-window-acceptance.py
python3 scripts/floating-pane-acceptance.py
python3 scripts/tab-tearoff-acceptance.py
python3 scripts/settings-parity-acceptance.py
```

| Check | Observable outcomes |
| --- | --- |
| View consent | Off by default, eight-action allowed set, host isolation, typing and popup guards, removed targets, no shell creation. The private SSH transport runs when a local sshd is available; its result is reported separately. |
| Workspace consent | Acknowledged creation/layout changes, old View consent preserved, local close confirmation/cancellation/expiry, clipboard/settings/system refusal. |
| Desktop windows | Independent CLI labels and tabs, real keyboard targeting, window-local dialogs, shared preferences, detach without replacement blocks, original and last-window closure. |
| Tab tear-off | Toolbar/sidebar pointer release and native preview, terminal-only chrome and navigation refusal, all four new split directions, last-tab placeholder, Go keyboard focus, Restore, hidden input/sizing, tab-menu/CLI paths and native lifetime; original block IDs, PID/start time and output survive. |
| Mac Settings and icons | Native search/page history, visual previews, repeated sidebar switching, unchanged preferences and original shell/block/PID/output. Headless tests also inspect every intermediate frame of sidebar/toolbar icon selection. |
| Floating panes | Original block ID/shell PID/start time/output survive moves, resize, hide/restore, reconnect, docking and native-view reattachment; stacking/input, cross-tab refusal, external changes and explicit source-tab destruction. |

Desktop, floating and tear-off checks also run on a private native Wayland compositor at
150% scale. They require labwc, a working Vulkan renderer, wlr-randr, grim,
xkbcli and the virtual input helper. X11 checks require Xvfb, openbox, xdotool and
ImageMagick. The helper uses its own Cargo target:

```sh
cargo build --locked --release --manifest-path tools/vptr/Cargo.toml
REX_ACCEPTANCE_BACKEND=wayland REX_ACCEPTANCE_SCALE=1.5 \
  python3 scripts/desktop-window-acceptance.py
REX_ACCEPTANCE_BACKEND=wayland REX_ACCEPTANCE_SCALE=1.5 \
  python3 scripts/floating-pane-acceptance.py
REX_ACCEPTANCE_BACKEND=wayland REX_ACCEPTANCE_SCALE=1.5 \
  python3 scripts/tab-tearoff-acceptance.py
```

Set `REX_ACCEPTANCE_TABS=sidebar` for the tear-off script to exercise sidebar dragging.
X11 preview capture also requires Pillow and libXfixes.

Protocol/state tests cover revoked or expired grants, changed connection epochs,
disconnects after dispatch without replay, acknowledged replies, negotiated API
revision failures, and minimized stream visibility/retirement. Headless tests
cover focus, dialogs, pinned tabs, drag cancellation, overlapping layers and hidden
pane input/ownership/notifications. Live scripts add real server and input evidence;
they do not establish identical behavior on every compositor or server revision.

The [CLI recipes](cli-recipes.md), [versioned matrix](mac-actions.md), [desktop ownership contract](desktop-windows.md)
and [floating behavior](floating-panes.md) describe supported behavior and explicit
limits. In particular, moving a pane to a new empty tab is unsupported on the
tested server and docking needs an existing tiled anchor. A detached tab, including
the session's last tab, leaves a Go/Restore placeholder in other views. A refused action never creates a sacrificial
program. Clipboard, local-system access and cross-server relaying remain outside
Workspace consent.
