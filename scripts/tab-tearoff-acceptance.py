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
            move(width / 2, (height + 52) / 2 + (61 if restore else 13))
            desktop.x("click", 1)
            time.sleep(0.2)

        desktop.type("printf 'TEAR_PID=%s\\n' \"$$\"")
        desktop.key("Return")
        wait(
            lambda: re.search(r"TEAR_PID=(\d+)", content()),
            "fixture shell PID was not printed",
        )
        pid = int(re.search(r"TEAR_PID=(\d+)", content()).group(1))
        start = Path(f"/proc/{pid}/stat").read_text().split()[21]
        expected_blocks = {block}
        expected_tabs = {tab}

        def preserved():
            assert blocks() == expected_blocks, (
                "detach/restore created or replaced a program"
            )
            assert Path(f"/proc/{pid}/stat").read_text().split()[21] == start
            assert f"TEAR_PID={pid}" in content()
            assert {w["window_id"] for w in view()["windows"]} == expected_tabs

        # A move within the tab strip is not a tear-off.
        move(270, 26)
        desktop.x("mousedown", 1)
        move(290, 26)
        desktop.x("mouseup", 1)
        time.sleep(0.2)
        assert len(windows()) == 1
        move(270, 26)
        desktop.x("mousedown", 1)
        move(300, 160)
        desktop.x("mouseup", 1)
        wait(
            lambda: len(windows()) == 2,
            "dragging a tab out did not open a native window",
        )
        second = next(w for w in windows() if w != first)
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
        move(270, 26)
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
        # Restoration preserves an owner that has navigated to unrelated work.
        label = base_label + "/window-4"
        perform("client.tab.new")
        expected_blocks = blocks()
        expected_tabs = {w["window_id"] for w in view()["windows"]}
        assert len(expected_blocks) == 2 and block in expected_blocks
        fourth = next(w for w in windows() if w != first)
        focus(first)
        placeholder_click()
        wait(
            lambda: focused() == fourth,
            "Go after navigation did not activate the owner",
        )
        desktop.type("printf 'GO_AFTER_NAVIGATION\\n'")
        desktop.key("Return")
        wait(
            lambda: "\nGO_AFTER_NAVIGATION\n" in content(),
            "Go did not select the original tab after destination navigation",
        )
        preserved()
        focus(first)
        placeholder_click(restore=True)
        time.sleep(0.5)
        assert len(windows()) == 2, (
            "Restore closed a destination showing unrelated work"
        )
        preserved()
        print(
            "PASS Restore preserves a destination that has navigated to another tab",
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
