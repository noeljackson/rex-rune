#!/usr/bin/env python3
"""CLI workspace control on a disposable Rex server and private X11 display.

Requires rex (API 12+), Xvfb, openbox, xdotool, ImageMagick and a built REX_BIN
(default target/debug/rex-linux). Creates a unique directory under ~/.cache with
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
    work = Path(tempfile.mkdtemp(prefix=f"cwt-rex-workspace-{os.getpid()}-", dir=Path.home() / ".cache"))
    (work / "OWNER").write_text(f"workspace-control-acceptance.py PID {os.getpid()}\n")
    print(f"Private workspace-control artifacts: {work}", flush=True)
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

        app, window = launch()
        before = view()
        perform("pane.split.right", error="Workspace control is off")
        assert view()["revision"] == before["revision"]
        print("PASS old saved consent stays view-only; no split created", flush=True)
        stop(app)
        label = "workspace-full-accept"
        config = json.loads(workspace.read_text())
        config["client_label"] = label
        config["hosts"][0]["workspace_control"] = True
        workspace.write_text(json.dumps(config))
        app, window = launch()
        count = len(view()["windows"])
        perform("client.tab.new")
        assert len(view()["windows"]) == count + 1, "creation must be acknowledged before success"
        perform("pane.split", {"direction": "auto"})
        assert len(blocks()) == 2
        ids = blocks()
        perform("pane.resize", {"direction": "left", "amount": 10})
        perform("pane.balance")
        perform("pane.move.left")
        assert blocks() == ids, "moving/resizing must preserve terminal blocks"
        print("PASS acknowledged tab/split creation, Mac auto, resize, balance and move", flush=True)
        for action in ("pane.copy", "pane.paste", "client.settings", "client.host.add"):
            perform(action, error="not available remotely")
        assert blocks() == ids
        print("PASS Workspace consent does not grant clipboard/settings/system actions", flush=True)

        closing = begin("pane.close")
        assert blocks() == ids, "close must wait for confirmation"
        desktop.key("Escape")
        finish(closing, False)
        assert blocks() == ids
        print("PASS local cancellation leaves both terminals running", flush=True)

        closing = begin("pane.close")
        desktop.capture(window, "workspace-close-confirmation")
        # The private 1440x900 fixture's rendered confirmation puts the explicit
        # destructive button here. Return alone intentionally keeps programs running.
        desktop.x("mousemove", "--window", window, 865, 489)
        desktop.x("click", 1)
        finish(closing, True)
        assert len(blocks()) == 1
        print("PASS confirmed close waits for server acknowledgement", flush=True)

        perform("pane.split.right")
        ids = blocks()
        closing = begin("pane.close")
        _, err = finish(closing, False)
        assert "expir" in err.lower(), err
        assert blocks() == ids
        print("PASS expired confirmation cannot close a terminal", flush=True)
        # Let the expiry close its dialog before opening Settings. The CLI timeout
        # can precede the UI's next frame under load.
        time.sleep(.4)
        desktop.key("Escape")
        desktop.key("ctrl+comma")
        perform("client.sidebar.toggle", error="dialog or popup")
        desktop.x("mousemove", "--window", window, 414, 425)
        desktop.x("click", 1)
        time.sleep(.3)
        desktop.capture(window, "workspace-remote-settings")
        print("PASS workspace CLI acceptance", flush=True)
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
