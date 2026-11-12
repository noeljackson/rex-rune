# Floating panes

Floating panes are Rex server layers inside a tab. Native desktop windows are
separate views of those tabs. Moving, docking, minimizing and restoring a layer
preserves its block IDs and running programs. Only **New floating pane** starts a
new terminal, inheriting the source shell's working directory when available.

Use Commands or the pane menu to float a tiled pane. Floating headers always show,
even in Compact density or with Pane headers Hidden. Drag the header to move the
layer, drag the bottom-right handle to resize, and use the header's Dock or Minimize
button. Minimized layers have restore capsules at the bottom of the tab. Clicking
a visible floating pane raises its layer and focuses that pane.

Dragging previews an outline and submits one bounds change on release. A changed
host, tab, connection epoch, native focus, layer, source block or external bounds
cancels the gesture. No failed input or layout mutation is replayed. Terminal
geometry never animates or becomes client-owned.

The four Mac action names are available, along with Linux bounds, movement,
resizing, stacking and explicit restore actions. They have no default shortcuts;
bind them in `shortcuts.json` and inspect them in Settings → Shortcuts. CLI calls require the saved
host's **Workspace** consent and target a particular desktop client label:

```sh
rex client list
rex -C LABEL do pane.float
rex -C LABEL do pane.floating.move direction=right amount=5
rex -C LABEL do pane.floating.resize direction=down amount=5
rex -C LABEL do pane.floating.bounds 'bounds=[0.18,0.15,0.62,0.60]'
rex -C LABEL do pane.floating.raise
rex -C LABEL do pane.floating.lower
rex -C LABEL do pane.minimize_floating
rex -C LABEL do pane.restore_floating
rex -C LABEL do pane.minimize_all_floating
rex -C LABEL do pane.new_floating
```

`pane.float` toggles float/dock. `pane.minimize_floating` hides the focused float,
or restores the most recent minimized float when a tiled pane has focus.
`pane.minimize_all_floating` hides the visible floats, or restores all when none
are visible. `pane.restore_floating` optionally accepts `layer_id`, restricted to
the targeted client's current tab. Restore also raises and focuses the layer.
Movement and resize amounts are positive percentage points (default 5); bounds
are finite normalized `[x,y,width,height]` within the tab, with positive size.

API 12 cannot dock into an empty tiled layer. The client therefore refuses to float
the last tiled pane, preserving a docking anchor. If another client removes that
anchor, docking is refused; the existing pane can still move to another existing
tab. No temporary shell or replacement program is created. Minimize-all also
requires visible tiled content so the tab retains a visible pane. These are
explicit compatibility limits, not full Mac behavior claims.

Minimized panes keep running. They receive no ordinary keyboard input, desktop clipboard writes or terminal
sizing claims from this view. Their program messages can still notify, subject to
notification and mute preferences. Their bounds and minimized state remain on
the server across control reconnects, closing a desktop view and reattachment.
Closing a tab or pane remains an explicit destructive action with the usual local
confirmation.

`scripts/floating-pane-acceptance.py` uses a private server and private X11 or
Wayland display. It checks the original shell PID, process start time, block ID and
output through movement, resize, minimize/restore, reconnect, docking and native
view reattachment, plus overlap hit testing and keyboard targeting.
