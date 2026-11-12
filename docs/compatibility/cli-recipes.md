# CLI workspace recipes

Use a disposable workspace for creation and close checks. Enable **Workspace**
control only for its saved host, run `rex client list`, and replace `LABEL` below
with the intended desktop client's label. Ordinary server CLI calls have separate
server authorization; this consent controls the client's `do` actions.

```sh
rex -C LABEL do session.select session_id=SESSION window_id=WINDOW
rex -C LABEL do client.tab.goto index=2
rex -C LABEL do client.tab.new
rex -C LABEL do pane.split direction=auto
rex -C LABEL do pane.focus.right
rex -C LABEL do pane.resize direction=right amount=5
rex -C LABEL do pane.move.right
rex -C LABEL do pane.balance
rex -C LABEL do pane.rename
rex -C LABEL do client.tab.rename
rex -C LABEL do pane.close
```

Run form/confirmation actions one at a time. Rename opens the existing local
form; close opens a local confirmation naming the affected programs and defaults
to **Keep running**. Complete it in the desktop client within the original
five-second request budget. Escape cancels; expiry discards the pending approval.
The CLI waits for the acknowledged result. Closing a pane ends that program;
closing a native desktop view preserves it.

All actions after `session.select` act on the targeted client's current host,
session and pane. Unknown, stale, wrong-host and cross-tab targets are refused.
Navigation waits for a 1.5-second pause in typing; an open text field, popup, key
sequence or layout drag also blocks remote requests. Clipboard, terminal input,
Settings and local-system actions remain unavailable remotely.

Directional aliases use the canonical handler. `auto` and Linux `best_fit` both
choose a split side from pane geometry. Resize amounts are percentage points of
the split, not terminal columns. Moves preserve the existing program; moving to
another existing tab uses `pane.move_to_tab` and a local picker, or an explicit
same-session `window_id`. Moving into a new empty tab is unsupported on API 12.

See [desktop window ownership](desktop-windows.md) for independent native targets,
[floating controls](floating-panes.md) for layers inside a tab, and
[acceptance commands](acceptance.md) for disposable server/display fixtures.
