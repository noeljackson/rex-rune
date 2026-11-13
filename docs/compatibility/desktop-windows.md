# Desktop window ownership

Each desktop window owns its Workspace, Rex control/terminal connections, navigation,
input, dialogs, search and selection. Connections are deliberately independent: Rex
already routes `client.perform` to a client identity, and each identity has its own
connection epoch. No process-global "last focused window" routes CLI requests.

The first window keeps the configured client label. Additional windows use
`<label>/window-N`, with monotonically increasing process-lifetime IDs. Use `rex
client list` to discover them and `rex -C '<label>/window-2' do ...` to target one.
IDs are not reused while the process lives. Changing the saved base label takes
effect on the next application launch.

Rex session windows are tabs; desktop windows are native surfaces. Selecting a tab
pins that desktop window's view without making other desktop windows follow the
server's active tab. Server layout and focus within each tab remain authoritative.
Only the foreground desktop window claims terminal sizing and applies clipboard
events. A remote-created window requests inactive opening. Wayland compositor policy can still activate a newly mapped surface; the client follows the resulting native focus events.

Preferences are shared across the process. Saves merge each window's changes with
the latest settings, and broadcast the result. Host consent changes invalidate
queued operations in every window before publishing the update. Navigation and
open dialogs remain window-local. Saved sidebar placement and last-visited host
and sessions are defaults for future windows; changing these defaults does not
move existing windows. Notification visibility follows the pinned view. Identical notifications observed by different windows within one second are delivered once; aggregate tab activity visible in the foreground window stays quiet. Program messages are suppressed only when their source pane is visible there, so minimized panes can still notify.

Closing a desktop window disconnects its clients and preserves server sessions and
programs. The last native window closing exits the process. Additional desktop
windows are not restored automatically on restart; server tabs and floating layers
remain server-owned and are restored when attached again.

Toolbar tab tear-off, the tab-menu row and Commands → Move tab to new desktop window
share one captured target. A drag needs 16 pt of travel and a release outside the
strip's 12 pt margin; cancellation or a changed host/session/tab/connection epoch
aborts it. Ownership is published only when the new native view exists and has
attached to that exact server tab. An opening failure or five-second deadline closes
only the unfinished view, leaves the source usable and never replays the request.

The process records a saved endpoint/session/server-tab → native-window owner.
Other views of that tab show **This tab is open in another window**, **Go to window**
and **← Restore to this window**. This includes the session's last tab. Placeholders
forward no terminal input, apply no clipboard writes, claim no sizing and do not
count as visible content for notifications. The server's tabs, block IDs, layouts,
processes and output never change as part of detachment or restoration.

Go selects the owner's exact tab and activates its native window. If it has an open
dialog, Go brings the dialog forward and leaves it intact. On Wayland Rune requests
an XDG token from the focused source's recent real pointer/key press, with its seat
and serial; the compositor decides focus. Tokens are asynchronous, single-use and
expire after two seconds. Closed targets are never recreated.

Restore assigns ownership to the requesting view. A detached destination closes
only while still dedicated to that tab and without an open dialog. Once it has
navigated to another known tab it stays open, even if it later returns. Closing an
owner makes other views usable again; closing a source leaves the owner running.
Known server tab/session removal clears ownership. Disconnected or unattached views
alone are not evidence of removal. Ownership is process-local and is not persisted.
Go and Restore have no default bindings and are refused remotely:
`client.tab.go_to_window`, `client.tab.restore_to_window`. CLI detachment keeps its
existing Workspace consent, connection epoch, deadline and acknowledged completion.
