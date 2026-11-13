#!/usr/bin/env python3
"""Native tab tear-off, Go/Restore and lifetime checks on a disposable Rex server and private X11 display.

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
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

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
        time.sleep(0.05)
    raise AssertionError(message)


def run():
    assert REX and Path(BINARY).is_file(), "rex and a built REX_BIN are required"
    work = Path(
        tempfile.mkdtemp(
            prefix=f"cwt-rex-tearoff-{os.getpid()}-", dir=Path.home() / ".cache"
        )
    )
    (work / "OWNER").write_text(f"tab-tearoff-acceptance.py PID {os.getpid()}\n")
    print(f"Private tab tear-off artifacts: {work}", flush=True)
    for directory in ("home", "config", "state", "runtime"):
        (work / directory).mkdir(mode=0o700)
    endpoint = "unix://" + str(work / "runtime/server.sock")
    env = {
        key: value for key, value in os.environ.items() if not key.startswith("REX_")
    }
    env.update(
        HOME=str(work / "home"),
        XDG_CONFIG_HOME=str(work / "config"),
        XDG_STATE_HOME=str(work / "state"),
        XDG_RUNTIME_DIR=str(work / "runtime"),
        REX_SERVER=endpoint,
        REX_BIN=BINARY,
        REX_LINUX_NO_UPDATE_CHECK="1",
        REX_CAPTURE_OUT=str(work / "captures"),
        SHELL="/bin/sh",
    )
    for key in ("ENV", "BASH_ENV", "ZDOTDIR", "DISPLAY", "WAYLAND_DISPLAY"):
        env.pop(key, None)
    # The imported helper owns its Xvfb/openbox and stubs tailnet discovery.
    spec = importlib.util.spec_from_file_location(
        "accept", ROOT / "scripts/rune-acceptance-x11.py"
    )
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
        return subprocess.run(
            command(*args),
            env=env,
            capture_output=True,
            text=True,
            timeout=12,
            check=False,
        )

    def api(method, **payload):
        result = cli("api", "call", method, json.dumps(payload))
        assert result.returncode == 0, (method, result.stdout, result.stderr)
        return json.loads(result.stdout)

    base_label = "tearoff-accept"
    label = base_label

    def perform(action, args=None, error=None):
        result = cli(
            "-C",
            label,
            "do",
            action,
            *[
                f"{key}={value if isinstance(value, str) else json.dumps(value)}"
                for key, value in (args or {}).items()
            ],
        )
        if error:
            assert result.returncode != 0 and error.lower() in result.stderr.lower(), (
                action,
                result.stdout,
                result.stderr,
            )
        else:
            assert result.returncode == 0, (action, result.stdout, result.stderr)
        return result

    try:
        import re

        init = work / "init.lua"
        init.write_text("-- private tab tear-off fixture\n")
        server = subprocess.Popen(
            command(
                "server",
                "run",
                "--listen",
                endpoint,
                "--restore=false",
                "--config",
                str(init),
                "--state-dir",
                str(work / "server-state"),
            ),
            env=env,
            stdout=server_log,
            stderr=subprocess.STDOUT,
        )
        owned.append(server)
        wait(
            lambda: (work / "runtime/server.sock").is_socket(),
            "private server did not start",
        )
        shell = {
            "block": {
                "flavor": "com.superlogical.terminal.shell",
                "options": {"shell": "/bin/sh"},
            }
        }
        sid = api(
            "session.create",
            label="tearoff",
            initial_windows=[{"window_label": "source", "layout": shell}],
        )["session_id"]
        workspace = work / "workspace.json"
        workspace.write_text(
            json.dumps(
                {
                    "client_label": base_label,
                    "hosts": [
                        {
                            "label": "private",
                            "endpoint": endpoint,
                            "remote_control": True,
                            "workspace_control": True,
                            "shell": "/bin/sh",
                        }
                    ],
                    "notifications": {"system_notifications": False},
                }
            )
        )
        workspace.with_name("workspace.shortcuts.json").write_text(
            json.dumps(
                [
                    {
                        "keys": f"ctrl+alt+{index}",
                        "action": "pane.split",
                        "args": {"direction": direction},
                    }
                    for index, direction in enumerate(
                        ["left", "right", "up", "down"], 1
                    )
                ]
            )
        )
        if os.environ.get("REX_ACCEPTANCE_TABS") == "sidebar":
            config = json.loads(workspace.read_text())
            config.update(sidebar_open=True, vertical_tabs=True, sidebar_width=236)
            workspace.write_text(json.dumps(config))
        if os.environ.get("REX_ACCEPTANCE_BACKEND") == "wayland":
            spec = importlib.util.spec_from_file_location(
                "private_wayland", ROOT / "scripts/private-wayland.py"
            )
            wl = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(wl)
            desktop = wl.Desktop(work, env, maximize=True)
        else:
            desktop = accept.Desktop()
        desktop.env = {
            key: value
            for key, value in desktop.env.items()
            if not key.startswith("REX_")
        }
        launch_env = {
            key: env[key]
            for key in [
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR",
                "REX_SERVER",
                "REX_LINUX_NO_UPDATE_CHECK",
                "SHELL",
            ]
        }
        app, first = desktop.launch(
            sid,
            workspace,
            extra_env=launch_env,
            log=(work / "app.log").open("w"),
            binary=BINARY,
        )
        owned.append(app)

        def windows():
            try:
                return desktop.x("search", "--pid", app.pid).splitlines()
            except subprocess.CalledProcessError:
                return []

        def clients():
            return api("client.list")["clients"]

        def client_id(name):
            return next(
                c["client_id"]
                for c in clients()
                if c.get("info", {}).get("label") == name
                and c.get("connection_state") == "connected"
            )

        wait(
            lambda: any(
                c.get("info", {}).get("label") == label
                and c.get("connection_state") == "connected"
                for c in clients()
            ),
            "source client did not register",
        )

        def view():
            return api("session.view", session_id=sid)

        original_view = view()
        tab = original_view["windows"][0]["window_id"]
        block = original_view["windows"][0]["focused_block_id"]

        def blocks():
            return {
                b["block_id"]
                for w in view()["windows"]
                for l in w["layers"]
                for b in l["blocks"]
            }

        def content():
            return api(
                "com.superlogical.terminal.format",
                session_id=sid,
                block_id=block,
                args={"format": "text", "trim": True},
            )["content"]

        def owner():
            return next(
                b["owner"]
                for w in view()["windows"]
                for l in w["layers"]
                for b in l["blocks"]
                if b["block_id"] == block
            )

        def focus(window):
            if os.environ.get("REX_ACCEPTANCE_BACKEND") != "wayland":
                desktop.x("windowraise", window)
            desktop.x("windowfocus", window)
            time.sleep(0.2)

        def focused():
            if os.environ.get("REX_ACCEPTANCE_BACKEND") != "wayland":
                return desktop.x("getwindowfocus")
            trace = desktop.trace.read_text(errors="replace")
            surfaces = dict(
                re.findall(
                    r"get_xdg_surface\(new id xdg_surface[#@](\d+), wl_surface[#@](\d+)\)",
                    trace,
                )
            )
            tops = {
                top: surface
                for surface, top in re.findall(
                    r"xdg_surface[#@](\d+)\.get_toplevel\(new id xdg_toplevel[#@](\d+)\)",
                    trace,
                )
            }
            entries = re.findall(
                r"wl_keyboard[#@]\d+\.enter\(\d+, wl_surface[#@](\d+),", trace
            )
            return next(
                (
                    top
                    for top, surface in tops.items()
                    if entries and surfaces.get(surface) == entries[-1]
                ),
                None,
            )

        def move(x, y):
            desktop.x("mousemove", "--window", first, round(x), round(y))
            time.sleep(0.1)

        def placeholder_click(restore=False):
            geometry = dict(
                line.split("=", 1)
                for line in desktop.x(
                    "getwindowgeometry", "--shell", first
                ).splitlines()
            )
            width, height = int(geometry["WIDTH"]), int(geometry["HEIGHT"])
            left = 236 if os.environ.get("REX_ACCEPTANCE_TABS") == "sidebar" else 0
            move((width + left) / 2, (height + 52) / 2 + (61 if restore else 13))
            desktop.x("click", 1)
            time.sleep(0.2)

        # Control registration can precede the terminal stream/ownership claim.
        # Wait before sending the first keystroke so cold startup cannot drop a
        # prefix of the fixture command (failed input is never replayed).
        focus(first)
        wait(lambda: owner() == client_id(label), "source did not own its terminal")
        wait(lambda: "$" in content(), "fixture shell prompt was not ready")
        desktop.type("printf 'TEAR_PID=%s\\n' \"$$\"")
        desktop.key("Return")
        wait(
            lambda: re.search(r"TEAR_PID=(\d+)", content()),
            "fixture shell PID was not printed",
        )
        pid = int(re.search(r"TEAR_PID=(\d+)", content()).group(1))
        shell_start = Path(f"/proc/{pid}/stat").read_text().split()[21]
        expected_blocks = {block}
        expected_tabs = {tab}

        def preserved():
            assert blocks() == expected_blocks, (
                "detach/restore created or replaced a program"
            )
            assert Path(f"/proc/{pid}/stat").read_text().split()[21] == shell_start
            assert f"TEAR_PID={pid}" in content()
            assert {w["window_id"] for w in view()["windows"]} == expected_tabs

        def capture_cursor(name):
            (work / "captures").mkdir(exist_ok=True)
            if os.environ.get("REX_ACCEPTANCE_BACKEND") == "wayland":
                subprocess.run(
                    [
                        "grim",
                        "-c",
                        "-o",
                        "HEADLESS-1",
                        str(work / "captures" / f"{name}.png"),
                    ],
                    env=desktop.env,
                    check=True,
                    timeout=10,
                )
                return
            import ctypes as C

            class Image(C.Structure):
                _fields_ = [
                    ("x", C.c_short),
                    ("y", C.c_short),
                    ("width", C.c_ushort),
                    ("height", C.c_ushort),
                    ("xhot", C.c_ushort),
                    ("yhot", C.c_ushort),
                    ("serial", C.c_ulong),
                    ("pixels", C.POINTER(C.c_ulong)),
                ]

            xlib = C.CDLL("libX11.so.6")
            fixes = C.CDLL("libXfixes.so.3")
            xlib.XOpenDisplay.argtypes = [C.c_char_p]
            xlib.XOpenDisplay.restype = C.c_void_p
            fixes.XFixesGetCursorImage.argtypes = [C.c_void_p]
            fixes.XFixesGetCursorImage.restype = C.POINTER(Image)
            xlib.XFree.argtypes = [C.c_void_p]
            xlib.XCloseDisplay.argtypes = [C.c_void_p]
            display = xlib.XOpenDisplay(desktop.env["DISPLAY"].encode())
            assert display
            pointer = fixes.XFixesGetCursorImage(display)
            try:
                im = pointer.contents
                if name == "held-drag-preview":
                    assert im.width >= 200 and im.height >= 180, "native drag preview missing"
                else:
                    assert im.width < 100 and im.height < 100, "drag preview cursor was not cleared"
                (work / f"{name}-cursor.json").write_text(
                    json.dumps(
                        {
                            "width": im.width,
                            "height": im.height,
                            "xhot": im.xhot,
                            "yhot": im.yhot,
                        }
                    )
                )
                from PIL import Image as Pixels

                rgba = bytes(
                    v
                    for i in range(im.width * im.height)
                    for v in (
                        (im.pixels[i] >> 16) & 255,
                        (im.pixels[i] >> 8) & 255,
                        im.pixels[i] & 255,
                        (im.pixels[i] >> 24) & 255,
                    )
                )
                Pixels.frombytes("RGBA", (im.width, im.height), rgba).save(
                    work / "captures" / f"{name}.png"
                )
            finally:
                xlib.XFree(pointer)
                xlib.XCloseDisplay(display)

        sidebar = os.environ.get("REX_ACCEPTANCE_TABS") == "sidebar"
        start = (100, 210) if sidebar else (270, 26)
        finish = (400, 220) if sidebar else (300, 160)
        if os.environ.get("REX_ACCEPTANCE_BACKEND") != "wayland":
            # Beyond the native source, near the private display's bottom-right.
            # The expanded view must still put its pane controls on screen.
            finish = (1500, 980)
        # A moved drag released inside the source creates no view.
        move(*start)
        desktop.x("mousedown", 1)
        move(start[0] + 20, start[1])
        desktop.x("mouseup", 1)
        time.sleep(0.2)
        assert len(windows()) == 1
        # Escape cancels a moved drag even when the pointer returns to its row.
        move(*start)
        desktop.x("mousedown", 1)
        move(*finish)
        desktop.key("Escape")
        move(*start)
        desktop.x("mouseup", 1)
        time.sleep(0.2)
        assert len(windows()) == 1, "Escape did not cancel the drag"
        capture_cursor("cancelled-drag-cursor")
        preserved()
        move(*start)
        desktop.x("mousedown", 1)
        move(*finish)
        time.sleep(0.6)
        assert len(windows()) == 1, "terminal view opened before release"
        preserved()
        capture_cursor("held-drag-preview")
        desktop.x("mouseup", 1)
        wait(
            lambda: len(windows()) == 2,
            "dragging a tab out did not open a native window",
        )
        second = next(w for w in windows() if w != first)
        capture_cursor("released-drag-cursor")
        second_label = base_label + "/window-2"
        wait(
            lambda: any(
                c.get("info", {}).get("label") == second_label
                and c.get("connection_state") == "connected"
                for c in clients()
            ),
            "detached client did not attach",
        )
        time.sleep(0.6)
        preserved()
        focus(second)
        wait(
            lambda: owner() == client_id(second_label), "destination did not own sizing"
        )
        focus(first)
        time.sleep(0.3)
        assert owner() == client_id(second_label), (
            "source placeholder took size ownership"
        )
        desktop.type("HIDDEN_SOURCE_INPUT")
        time.sleep(0.2)
        assert "HIDDEN_SOURCE_INPUT" not in content(), (
            "source placeholder forwarded input"
        )
        time.sleep(1.7)
        refused = cli("-C", base_label, "do", "pane.split.right")
        assert refused.returncode != 0, "hidden source accepted a pane operation"
        preserved()
        desktop.capture(first, "source-placeholder")
        if os.environ.get("REX_ACCEPTANCE_BACKEND") != "wayland":
            desktop.x("windowminimize", second)
            time.sleep(0.2)
        placeholder_click()
        wait(
            lambda: focused() == second, "Go to window did not activate the destination"
        )
        desktop.type("printf 'GO_TO_WINDOW_INPUT\\n'")
        desktop.key("Return")
        wait(
            lambda: "\nGO_TO_WINDOW_INPUT\n" in content(),
            "Go input did not reach the original program",
        )
        preserved()
        desktop.capture(second, "detached-go-window")
        print(
            "PASS real tab tear-off, last-tab placeholder, hidden source input/sizing and native Go activation",
            flush=True,
        )
        label = second_label
        for action, args in [
            ("client.tab.new", None),
            ("client.tab.next", None),
            ("client.sidebar.toggle", None),
            ("session.select", {"session_id": sid}),
        ]:
            perform(action, args, error="own tab")
        preserved()

        def positions(node, rect=(0.0, 0.0, 1.0, 1.0)):
            if node.get("block_id"):
                return {node["block_id"]: rect}
            split = node.get("split")
            assert split, node
            x, y, w, h = rect
            r = split["ratio"]
            if split["direction"] == "horizontal":
                a = (x, y, w * r, h)
                b = (x + w * r, y, w * (1 - r), h)
            else:
                a = (x, y, w, h * r)
                b = (x, y + h * r, w, h * (1 - r))
            return positions(split["before"], a) | positions(split["after"], b)

        def click_header_split(direction):
            geometry = dict(
                line.split("=", 1)
                for line in desktop.x(
                    "getwindowgeometry", "--shell", second
                ).splitlines()
            )
            width = int(geometry["WIDTH"])
            desktop.x(
                "mousemove",
                "--window",
                second,
                width - 140 + ["left", "right", "up", "down"].index(direction) * 24,
                18,
            )
            desktop.x("click", 1)

        for index, direction in enumerate(["left", "right", "up", "down"], 1):
            api("session.focus_block", session_id=sid, block_id=block)
            time.sleep(0.3)
            before = blocks()
            if index == 1:
                click_header_split(direction)
            else:
                desktop.key(f"ctrl+alt+{index}")
            wait(
                lambda: len(blocks()) == len(before) + 1,
                f"split {direction} did not create one pane",
            )
            new = (blocks() - before).pop()
            tiled = next(
                l
                for w in view()["windows"]
                for l in w["layers"]
                if l["kind"] == "tiled"
            )
            xy = positions(tiled["layout"])
            x, y, w, h = xy[block]
            nx, ny, nw, nh = xy[new]
            assert {
                "left": nx + nw <= x + 1e-6,
                "right": nx >= x + w - 1e-6,
                "up": ny + nh <= y + 1e-6,
                "down": ny >= y + h - 1e-6,
            }[direction], (direction, xy)
            expected_blocks = blocks()
            preserved()
        api("session.focus_block", session_id=sid, block_id=block)
        time.sleep(0.3)
        desktop.capture(second, "isolated-terminal-four-splits")
        print(
            "PASS terminal-only navigation guards and new left/right/up/down panes preserve the original program",
            flush=True,
        )
        focus(first)
        placeholder_click(restore=True)
        wait(
            lambda: len(windows()) == 1,
            "Restore did not close the dedicated detached view",
        )
        wait(
            lambda: owner() == client_id(base_label),
            "Restore did not reclaim the original terminal",
        )
        preserved()
        desktop.capture(first, "restored-source")
        desktop.type("printf 'RESTORED_INPUT\\n'")
        desktop.key("Return")
        wait(
            lambda: "\nRESTORED_INPUT\n" in content(),
            "restored input did not reach the original program",
        )
        print(
            "PASS Restore returns the original tab/block/PID/output without a replacement shell",
            flush=True,
        )
        # The tab menu follows the same path; disabled left/right rows are skipped.
        move(*start)
        desktop.x("click", 3)
        desktop.key("Down")
        desktop.key("Down")
        desktop.key("Return")
        wait(lambda: len(windows()) == 2, "tab menu did not detach")
        label = base_label + "/window-3"
        wait(
            lambda: any(
                c.get("info", {}).get("label") == label
                and c.get("connection_state") == "connected"
                for c in clients()
            ),
            "menu destination did not attach",
        )
        time.sleep(1.7)
        perform("client.window.close")
        wait(
            lambda: len(windows()) == 1,
            "owner close did not close only that native view",
        )
        focus(first)
        wait(
            lambda: owner() == client_id(base_label),
            "owner close did not restore the source view",
        )
        preserved()
        print(
            "PASS tab-menu detachment and owner closure restore the source without ending programs",
            flush=True,
        )
        label = base_label
        time.sleep(1.7)
        perform("client.tab.move_to_new_window")
        assert len(windows()) == 2
        label = base_label + "/window-4"
        perform("client.tab.new", error="own tab")
        perform("client.tab.goto", {"index": 1}, error="own tab")
        preserved()
        print(
            "PASS detached terminal refuses tab creation/navigation without changing its server tab",
            flush=True,
        )
        label = base_label
        time.sleep(1.7)
        perform("client.window.close")
        wait(lambda: len(windows()) == 1, "source close affected the owner window")
        assert app.poll() is None
        label = base_label + "/window-4"
        preserved()
        perform("pane.zoom")
        perform("client.window.close")
        app.wait(timeout=10)
        assert app.returncode == 0
        preserved()
        print(
            "PASS acknowledged CLI detach, source-window closure and last-window exit preserve programs",
            flush=True,
        )
        print("PASS tab tear-off acceptance", flush=True)
    except BaseException:
        if desktop and "first" in locals() and app.poll() is None:
            try:
                desktop.capture(first, "failure")
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
        (work / "processes.json").write_text(
            json.dumps(
                [{"pid": p.pid, "returncode": p.poll()} for p in owned], indent=2
            )
        )


if __name__ == "__main__":
    run()
