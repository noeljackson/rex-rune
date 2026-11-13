#!/usr/bin/env python3
"""Floating layers on a disposable Rex server and private display.

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
    work = Path(tempfile.mkdtemp(prefix=f"cwt-rex-floating-{os.getpid()}-", dir=Path.home() / ".cache"))
    (work / "OWNER").write_text(f"floating-pane-acceptance.py PID {os.getpid()}\n")
    print(f"Private floating-pane artifacts: {work}", flush=True)
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

    label = "floating-accept"

    def perform(action, args=None, error=None):
        result = cli("-C", label, "do", action,
                     *[f"{key}={value if isinstance(value, str) else json.dumps(value)}"
                       for key, value in (args or {}).items()])
        if error:
            assert result.returncode != 0 and error.lower() in result.stderr.lower(), (action, result.stdout, result.stderr)
        else:
            assert result.returncode == 0, (action, result.stdout, result.stderr)
        return result

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
        sid = api("session.create", label="floating-accept", initial_windows=[{"window_label": "first", "layout": shell}])["session_id"]
        workspace = work / "workspace.json"
        config = {"client_label": label, "hosts": [{"label": "private", "endpoint": endpoint, "remote_control": True, "shell": "/bin/sh"}],
                  "notifications": {"system_notifications": False}}
        workspace.write_text(json.dumps(config))
        workspace.with_name("workspace.shortcuts.json").write_text(json.dumps([
            {"keys":"ctrl+alt+1", "action":"pane.float"},
            {"keys":"ctrl+alt+2", "action":"pane.minimize_floating"},
            {"keys":"ctrl+alt+3", "action":"pane.restore_floating"},
            {"keys":"ctrl+alt+4", "action":"pane.floating.move", "args":{"direction":"right","amount":3}},
            {"keys":"ctrl+alt+5", "action":"pane.floating.resize", "args":{"direction":"down","amount":3}}
        ]))
        if os.environ.get("REX_ACCEPTANCE_BACKEND") == "wayland":
            wl_spec = importlib.util.spec_from_file_location("private_wayland", ROOT / "scripts/private-wayland.py")
            wl = importlib.util.module_from_spec(wl_spec)
            wl_spec.loader.exec_module(wl)
            desktop = wl.Desktop(work, env, maximize=True)
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
        config.update(pane_padding=8, pane_density="comfortable", pane_header="auto")
        workspace.write_text(json.dumps(config))
        app, window = launch()
        wid = view()["windows"][0]["window_id"]

        def current():
            return next(w for w in view()["windows"] if w["window_id"] == wid)

        def layer_of(block):
            return next(l for l in current()["layers"] if any(b["block_id"] == block for b in l["blocks"]))

        def content(block):
            return api("com.superlogical.terminal.format", session_id=sid, block_id=block,
                       args={"format":"text", "trim":True})["content"]

        def focus():
            return current()["focused_block_id"]

        perform("pane.float", error="Keep a tiled pane")
        assert len(blocks()) == 1 and all(l["kind"] == "tiled" for l in current()["layers"])
        perform("pane.split.right")
        perform("pane.split.down")
        original_ids = blocks()
        assert len(original_ids) == 3
        original = focus()
        desktop.type('printf \'FLOAT_PID=%s\\n\' "$$"')
        desktop.key("Return")
        import re
        wait(lambda: re.search(r"FLOAT_PID=(\d+)", content(original)), "fixture PID was not printed")
        pid = int(re.search(r"FLOAT_PID=(\d+)", content(original)).group(1))
        start = Path(f"/proc/{pid}/stat").read_text().split()[21]

        def preserved():
            assert original in blocks()
            assert Path(f"/proc/{pid}/stat").read_text().split()[21] == start, "original shell was replaced"
            assert f"FLOAT_PID={pid}" in content(original)

        time.sleep(1.7)
        perform("pane.float")
        original_layer = layer_of(original)["layer_id"]
        assert layer_of(original)["kind"] == "floating"
        assert blocks() == original_ids
        perform("pane.floating.bounds", {"bounds":[0.18,0.15,0.62,0.60]})
        perform("pane.floating.move", {"direction":"right", "amount":2})
        perform("pane.floating.resize", {"direction":"down", "amount":2})
        preserved()
        print("PASS float/move/resize preserve original block, shell PID and output; sole tiled pane guarded", flush=True)

        geometry = dict(line.split("=",1) for line in desktop.x("getwindowgeometry", "--shell", window).splitlines())
        width,height = int(geometry["WIDTH"]),int(geometry["HEIGHT"])
        iw,ih = width-20,height-52-10

        def card(bounds):
            x,y,w,h = bounds
            left,top,right,bottom = x>1e-6,y>1e-6,x+w<1-1e-6,y+h<1-1e-6
            return [10+x*iw+4*left, 52+y*ih+4*top, w*iw-4*(left+right), h*ih-4*(top+bottom)]

        def move(x,y):
            desktop.x("mousemove", "--window", window, round(x), round(y))
            time.sleep(.06)

        def click_at(x,y):
            move(x,y)
            desktop.x("click",1)
            time.sleep(.15)

        before = layer_of(original)["bounds"]
        x,y,w,h = card(before)
        move(x+20,y+12)
        desktop.x("mousedown",1)
        move(x+50,y+32)
        time.sleep(.3)
        assert layer_of(original)["bounds"] == before, "drag must preview until release"
        perform("pane.floating.move", {"direction":"left"}, error="layout drag")
        desktop.x("mouseup",1)
        wait(lambda: layer_of(original)["bounds"] != before, "header drag did not move the layer")
        after = layer_of(original)["bounds"]
        time.sleep(.4)  # Wait for the native frame before aiming at its newly moved corner.
        desktop.capture(window,"floating-header-moved")
        assert abs(after[0]-before[0]-30/iw)<.003 and abs(after[1]-before[1]-20/ih)<.003, (before,after)
        x,y,w,h = card(after)
        move(x+w-5,y+h-5)
        desktop.x("mousedown",1)
        move(x+w+35,y+h+20)
        desktop.x("mouseup",1)
        wait(lambda: layer_of(original)["bounds"][2] > after[2]+.01, "corner drag did not resize the layer")
        preserved()
        desktop.capture(window,"floating-moved-resized")
        print("PASS real header/corner drags, no early geometry mutation and remote drag guard", flush=True)

        desktop.key("ctrl+alt+4")
        desktop.key("ctrl+alt+5")
        time.sleep(1.7)
        preserved()
        desktop.key("ctrl+alt+2")
        wait(lambda: layer_of(original).get("state") == "minimized", "keyboard minimize failed")
        assert focus() != original
        preserved()
        desktop.capture(window,"floating-minimized")
        bootstrap_count = (work / "server.log").read_text().count("netapi_method=session.bootstrap ")
        desktop.key("ctrl+shift+p")
        desktop.type("Reconnect host")
        desktop.key("Return")
        wait(lambda: (work / "server.log").read_text().count("netapi_method=session.bootstrap ") > bootstrap_count and registered(), "control reconnect did not complete")
        time.sleep(1.7)
        assert layer_of(original).get("state") == "minimized"
        preserved()
        click_at(40,height-25)
        wait(lambda: layer_of(original).get("state") != "minimized" and focus() == original, "capsule did not restore original layer")
        preserved()
        print("PASS keyboard minimize, control reconnect and pointer restore retain original program", flush=True)

        perform("pane.new_floating")
        extra = focus()
        assert extra not in original_ids and len(blocks()) == 4
        extra_layer = layer_of(extra)["layer_id"]
        c_bounds = layer_of(original)["bounds"]
        perform("pane.floating.bounds", {"bounds":[c_bounds[0]+.02,c_bounds[1]+.02,.50,.50]})
        perform("pane.floating.lower")
        x,y,w,h=card(c_bounds)
        click_at(x+w/2,y+h/2)
        wait(lambda: focus() == original, "topmost original pane did not receive focus")
        desktop.type("printf 'TOP_FLOAT_INPUT\\n'")
        desktop.key("Return")
        wait(lambda: "\nTOP_FLOAT_INPUT\n" in content(original), "keyboard input missed the topmost pane")
        assert "TOP_FLOAT_INPUT" not in content(extra)
        time.sleep(1.7)
        perform("pane.minimize_all_floating")
        assert all(l.get("state") == "minimized" for l in current()["layers"] if l["kind"] == "floating")
        preserved()
        perform("pane.minimize_all_floating")
        assert all(l.get("state") != "minimized" for l in current()["layers"] if l["kind"] == "floating")
        preserved()
        desktop.capture(window,"floating-stacking")
        print("PASS explicit new floating pane, overlap hit testing, keyboard target and minimize/restore all", flush=True)

        api("session.focus_block",session_id=sid,block_id=original)
        perform("pane.minimize_floating")
        perform("client.window.close")
        app.wait(timeout=10)
        assert app.returncode == 0
        preserved()
        label = "floating-reattached"
        saved = json.loads(workspace.read_text())
        saved["client_label"] = label
        workspace.write_text(json.dumps(saved))
        app,window = launch()
        assert layer_of(original).get("state") == "minimized"
        perform("pane.restore_floating", {"layer_id":original_layer})
        perform("pane.float")
        assert layer_of(original)["kind"] == "tiled"
        preserved()
        api("session.resize_layer",session_id=sid,layer_id=extra_layer,bounds=[.05,.05,.30,.30])
        api("session.focus_block",session_id=sid,block_id=extra)
        perform("pane.floating.move", {"direction":"right","amount":2})
        assert abs(layer_of(extra)["bounds"][0]-.07)<.001, layer_of(extra)
        other = api("session.new_window",session_id=sid,window_label="other",layout=shell,focus=False)
        other_window = other["window_id"]
        other_block = next(b["block_id"] for w in view()["windows"] if w["window_id"] == other_window for l in w["layers"] for b in l["blocks"])
        foreign = api("session.float_block",session_id=sid,block_id=other_block,bounds=[.1,.1,.6,.6])["layer_id"]
        perform("pane.restore_floating", {"layer_id":foreign}, error="No matching floating layer")
        perform("client.tab.goto", {"index":2})
        perform("pane.float", error="no tiled docking anchor")
        perform("pane.minimize_all_floating", error="Keep a visible tiled pane")
        perform("client.tab.goto", {"index":1})
        preserved()
        desktop.capture(window,"floating-docked")
        assert sum(len(l["blocks"]) for w in view()["windows"] for l in w["layers"]) == 5, "a move or refusal created a replacement program"
        print("PASS native view close/reattach, docking, external layer updates and cross-tab/empty-anchor refusals", flush=True)
        before_bounds = layer_of(extra)["bounds"]
        def terminal_size():
            size = api("com.superlogical.terminal.size",session_id=sid,block_id=original,args={})
            return size["columns"],size["rows"]
        old_size = terminal_size()
        if os.environ.get("REX_ACCEPTANCE_BACKEND") == "wayland":
            subprocess.run(["wlr-randr","--output","HEADLESS-1","--custom-mode","1680x1050@60"],env=desktop.env,check=True,timeout=10)
        else:
            desktop.x("windowsize",window,1200,760)
        wait(lambda: terminal_size() != old_size,"native view resize did not reach the server terminal")
        assert layer_of(extra)["bounds"] == before_bounds, "native resizing changed normalized floating bounds"
        preserved()
        desktop.capture(window,"floating-native-resized")
        print("PASS native view resizing preserves normalized floating bounds and the original process",flush=True)
        api("session.focus_block",session_id=sid,block_id=extra)
        perform("pane.minimize_floating")
        assert layer_of(extra).get("state") == "minimized"
        # An external client's explicit source-tab close destroys every layer in it.
        # Local destructive confirmation/cancellation is checked by workspace acceptance.
        api("session.close_window",session_id=sid,window_id=wid)
        wait(lambda: all(w["window_id"] != wid for w in view()["windows"]),"closed source tab remained on the server")
        wait(lambda: not Path(f"/proc/{pid}").exists(),"source-tab close left its shell running")
        assert {b["block_id"] for w in view()["windows"] for l in w["layers"] for b in l["blocks"]} == {other_block}
        perform("pane.restore_floating", {"layer_id":extra_layer}, error="No matching floating layer")
        perform("client.window.close")
        app.wait(timeout=10)
        assert app.returncode == 0
        print("PASS explicit source-tab closure destroys its tiled/minimized programs and preserves the other tab",flush=True)
        print("PASS floating pane acceptance", flush=True)
    except BaseException:
        if desktop and "window" in locals():
            try:
                desktop.capture(window,"failure")
            except (OSError, subprocess.SubprocessError):
                pass
        raise
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
