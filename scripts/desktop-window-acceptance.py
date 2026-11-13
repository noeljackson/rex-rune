#!/usr/bin/env python3
"""Independent desktop windows on a disposable Rex server and private X11 display.

Requires rex (API 12+), Xvfb, openbox, xdotool, ImageMagick and a built REX_BIN
(default target/debug/rex-linux). REX_ACCEPTANCE_BACKEND=wayland uses private-wayland.py
instead (labwc, Vulkan, wlr-randr, grim, xkbcli, built tools/vptr); optional
REX_ACCEPTANCE_SCALE sets its display scale. Creates a unique directory under ~/.cache with
private HOME/XDG/config/state/socket paths. Ignores inherited Rex targeting and
never uses personal servers, sessions or display input. All processes are owned,
recorded and stopped by PID; logs and captures remain in the printed directory.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get("REX_BIN", str(ROOT / "target/debug/rex-linux"))
REX = shutil.which("rex")


def stop(process):
    if process.poll() is None:
        process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def wait(predicate, message, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.05)
    raise AssertionError(message)


def run():
    assert REX and Path(BINARY).is_file(), "rex and a built REX_BIN are required"
    work = Path(tempfile.mkdtemp(prefix=f"cwt-rex-desktop-{os.getpid()}-", dir=Path.home() / ".cache"))
    (work / "OWNER").write_text(f"desktop-window-acceptance.py PID {os.getpid()}\n")
    print(f"Private desktop-window artifacts: {work}", flush=True)
    for directory in ("home", "config", "state", "runtime"):
        (work / directory).mkdir(mode=0o700)
    endpoint = "unix://" + str(work / "runtime/server.sock")
    env = {key: value for key, value in os.environ.items() if not key.startswith("REX_")}
    env.update(HOME=str(work / "home"), XDG_CONFIG_HOME=str(work / "config"),
               XDG_STATE_HOME=str(work / "state"), XDG_RUNTIME_DIR=str(work / "runtime"),
               REX_SERVER=endpoint, REX_BIN=BINARY, REX_LINUX_NO_UPDATE_CHECK="1",
               REX_CAPTURE_OUT=str(work / "captures"), SHELL="/bin/sh")
    for key in ("ENV", "BASH_ENV", "ZDOTDIR", "DISPLAY", "WAYLAND_DISPLAY"):
        env.pop(key, None)
    # The imported helper owns its Xvfb/openbox and stubs tailnet discovery.
    spec = importlib.util.spec_from_file_location("accept", ROOT / "scripts/rune-acceptance-x11.py")
    accept = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(accept)
    accept.OUT = work / "captures"
    accept.BIN = BINARY
    owned = []
    desktop = None
    server_log = (work / "server.log").open("w")

    def command(*args):
        return [REX, "--autostart=false", "--server", endpoint, *args]

    def cli(*args):
        return subprocess.run(command(*args), env=env, capture_output=True, text=True, timeout=12)

    def api(method, **payload):
        result = cli("api", "call", method, json.dumps(payload))
        assert result.returncode == 0, (method, result.stdout, result.stderr)
        return json.loads(result.stdout)

    label = "workspace-view-accept"

    def perform(action, args=None, error=None):
        result = cli("-C", label, "do", action,
                     *[f"{key}={value if isinstance(value, str) else json.dumps(value)}"
                       for key, value in (args or {}).items()])
        if error:
            assert result.returncode != 0 and error.lower() in result.stderr.lower(), (action, result.stdout, result.stderr)
        else:
            assert result.returncode == 0, (action, result.stdout, result.stderr)
        return result

    def begin(action):
        process = subprocess.Popen(command("-C", label, "do", action), env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        owned.append(process)
        time.sleep(.35)
        assert process.poll() is None, process.communicate(timeout=1)
        return process

    def finish(process, success):
        out, err = process.communicate(timeout=8)
        assert (process.returncode == 0) == success, (out, err)
        return out, err

    try:
        init = work / "init.lua"
        init.write_text("-- private acceptance fixture\n")
        server = subprocess.Popen(command("server", "run", "--listen", endpoint,
                                          "--restore=false", "--config", str(init),
                                          "--state-dir", str(work / "server-state")),
                                  env=env, stdout=server_log, stderr=subprocess.STDOUT)
        owned.append(server)
        wait(lambda: (work / "runtime/server.sock").is_socket(), "private server did not start")
        shell = {"block": {"flavor": "com.superlogical.terminal.shell", "options": {"shell": "/bin/sh"}}}
        sid = api("session.create", label="workspace-accept", initial_windows=[{"window_label": "first", "layout": shell}])["session_id"]
        workspace = work / "workspace.json"
        config = {"client_label": label, "hosts": [{"label": "private", "endpoint": endpoint, "remote_control": True, "shell": "/bin/sh"}],
                  "notifications": {"system_notifications": False}}
        workspace.write_text(json.dumps(config))
        workspace.with_name("workspace.shortcuts.json").write_text(json.dumps([
            {"keys":"ctrl+alt+9", "action":"client.font.increase"}
        ]))
        if os.environ.get("REX_ACCEPTANCE_BACKEND") == "wayland":
            wl_spec = importlib.util.spec_from_file_location("private_wayland", ROOT / "scripts/private-wayland.py")
            wl = importlib.util.module_from_spec(wl_spec)
            wl_spec.loader.exec_module(wl)
            desktop = wl.Desktop(work, env)
        else:
            desktop = accept.Desktop()
        desktop.env = {key: value for key, value in desktop.env.items() if not key.startswith("REX_")}
        launch_env = {key: env[key] for key in (
            "HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_RUNTIME_DIR",
            "REX_SERVER", "REX_LINUX_NO_UPDATE_CHECK", "SHELL",
        )}

        def registered():
            return any(c.get("info", {}).get("label") == label and c.get("connection_state") == "connected"
                       for c in api("client.list")["clients"])

        def launch():
            app, window = desktop.launch(sid, workspace, extra_env=launch_env,
                                         log=(work / f"{label}.log").open("w"), binary=BINARY)
            owned.append(app)
            wait(registered, "client did not register")
            return app, window

        def view():
            return api("session.view", session_id=sid)

        def active():
            value = view()
            return next(w for w in value["windows"] if w["window_id"] == value["active_window_id"])

        def blocks():
            return {b["block_id"] for layer in active()["layers"] for b in layer["blocks"]}

        config["hosts"][0]["workspace_control"] = True
        workspace.write_text(json.dumps(config))
        app, first = launch()
        original = blocks()
        sid2 = api("session.create", label="other-session", initial_windows=[{"window_label":"other", "layout":shell}])["session_id"]
        api("session.new_window", session_id=sid, window_label="second-tab", layout=shell)
        perform("client.window.new")
        windows = desktop.x("search", "--pid", app.pid).splitlines()
        assert len(windows) == 2, windows
        second = next(w for w in windows if w != first)
        second_label = label + "/window-2"
        wait(lambda: any(c.get("info",{}).get("label") == second_label for c in api("client.list")["clients"]), "second client identity missing")
        print("PASS independent native window and discoverable client label", flush=True)
        base_label = label
        label = second_label
        perform("client.tab.goto", {"index":2})
        perform("pane.split.right")
        second_blocks = blocks()
        assert len(second_blocks) == 2
        label = base_label
        # The first view remains on the original tab despite the server active tab changing.
        perform("pane.split.down")
        first_blocks = blocks()
        assert len(first_blocks) == 2 and first_blocks != second_blocks
        assert original.issubset(first_blocks)
        assert len(api("session.view", session_id=sid)["windows"]) == 2
        print("PASS independently targeted tab and pane operations preserve both views", flush=True)
        label = second_label
        perform("session.select", {"session_id":sid2})
        perform("pane.split.right")
        assert len(api("session.view", session_id=sid2)["windows"][0]["layers"][0]["blocks"]) == 2
        desktop.x("windowfocus", second)
        desktop.type("printf 'SECOND_WINDOW_INPUT\\n'")
        desktop.key("Return")
        wait(lambda: any("\nSECOND_WINDOW_INPUT\n" in api("com.superlogical.terminal.format", session_id=sid2, block_id=b["block_id"], args={"format":"text", "trim":True})["content"] for l in api("session.view", session_id=sid2)["windows"][0]["layers"] for b in l["blocks"]), "input did not reach second window")
        desktop.capture(second, "second-window")
        time.sleep(1.7)
        second_block = api("session.view", session_id=sid2)["windows"][0]["focused_block_id"]
        def second_size():
            return api("com.superlogical.terminal.size", session_id=sid2, block_id=second_block, args={})
        old_cell_height = second_size()["cell_height_px"]
        desktop.x("windowfocus", first)
        desktop.key("ctrl+comma")
        label = base_label
        perform("pane.zoom", error="dialog or popup")
        label = second_label
        perform("pane.zoom")
        desktop.key("Escape")
        old_font = json.loads(workspace.read_text())["font_size"]
        desktop.key("ctrl+alt+9")
        wait(lambda: json.loads(workspace.read_text())["font_size"] == old_font + 1, "local font preference was not saved")
        desktop.x("windowfocus", second)
        wait(lambda: second_size()["cell_height_px"] > old_cell_height, "shared font preference did not reach the second view")
        desktop.capture(second, "shared-settings")
        print("PASS dialogs stay window-local; saved font changes propagate to the other view", flush=True)
        time.sleep(1.7)  # Respect the ordinary post-keypress remote-navigation guard.
        label = base_label
        before_detach = api("session.view", session_id=sid)
        existing_blocks = {b["block_id"] for w in before_detach["windows"] for l in w["layers"] for b in l["blocks"]}
        perform("client.tab.move_to_new_window")
        after_detach = api("session.view", session_id=sid)
        assert existing_blocks == {b["block_id"] for w in after_detach["windows"] for l in w["layers"] for b in l["blocks"]}, "detaching created or replaced a terminal"
        assert len(desktop.x("search", "--pid", app.pid).splitlines()) == 3
        label = base_label + "/window-3"
        perform("pane.zoom")
        label = base_label
        perform("pane.split.right")
        label = base_label + "/window-3"
        perform("client.window.close")
        print("PASS tab detaches into its own CLI target without creating replacement programs", flush=True)
        label = base_label
        perform("client.window.close")
        assert app.poll() is None, "original window close quit the process"
        wait(lambda: len(desktop.x("search", "--pid", app.pid).splitlines()) == 1, "original desktop window remained")
        label = second_label
        perform("client.tab.new")
        assert len(api("session.view", session_id=sid2)["windows"]) == 2
        print("PASS original closes after reply; secondary stays usable and programs survive", flush=True)
        perform("client.window.fullscreen.toggle")
        perform("client.window.fullscreen.toggle")
        perform("client.window.close")
        app.wait(timeout=10)
        assert app.returncode == 0
        assert len(api("session.view", session_id=sid)["windows"]) == 2
        assert len(api("session.view", session_id=sid2)["windows"]) == 2
        print("PASS last-window exit leaves both server sessions running", flush=True)
    finally:
        for process in reversed(owned):
            stop(process)
        if desktop:
            desktop.close()
            owned.extend([desktop.wm, desktop.server])
        server_log.close()
        (work / "processes.json").write_text(json.dumps([{"pid": p.pid, "returncode": p.poll()} for p in owned], indent=2))


if __name__ == "__main__":
    run()
