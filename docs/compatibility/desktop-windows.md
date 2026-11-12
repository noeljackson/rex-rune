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
move existing windows. Notification visibility follows the pinned view. Identical notifications observed by different windows within one second are delivered once; a tab visible in the foreground window stays quiet.

Closing a desktop window disconnects its clients and preserves server sessions and
programs. The last native window closing exits the process. Additional desktop
windows are not restored automatically on restart; server tabs and floating layers
remain server-owned and are restored when attached again.
