"""A disposable labwc/virtual-input backend for acceptance scripts.

Requires labwc, wlr-randr, grim, xkbcli and tools/vptr/target/release/vptr.
The caller supplies a fresh private work/runtime/HOME tree. Never attaches to an
inherited compositor, D-Bus session or input device. Owns every process it stops.
"""
import os
from pathlib import Path
import re
import runpy
import select
import shlex
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
KEYMAP = runpy.run_path(str(ROOT / "scripts/demo-record.py"))


class Desktop:
    def __init__(self, work, base, maximize=False):
        self.maximize = maximize
        self.scale = float(os.environ.get("REX_ACCEPTANCE_SCALE", "1"))
        self.work = work
        self.apps = []
        self.env = dict(base, WLR_BACKENDS="headless", WLR_RENDERER="vulkan", WLR_HEADLESS_OUTPUTS="1")
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DBUS_SESSION_BUS_ADDRESS", "HYPRLAND_INSTANCE_SIGNATURE"):
            self.env.pop(key, None)
        config = work / "labwc"
        config.mkdir()
        rule = '<windowRules><windowRule identifier="dev.shuv.rex-linux"><action name="Maximize"/></windowRule></windowRules>' if maximize else ''
        (config / "rc.xml").write_text('<labwc_config><core><decoration>client</decoration></core><keyboard><keybind key="A-Tab"><action name="NextWindow"/></keybind></keyboard>' + rule + '</labwc_config>')
        marker = work / "wayland-display"
        self.wm = subprocess.Popen(["labwc", "-C", str(config), "-s", "sh -c " + shlex.quote("printf '%s' \"$WAYLAND_DISPLAY\" > " + shlex.quote(str(marker)))], env=self.env, stdout=(work / "labwc.log").open("w"), stderr=subprocess.STDOUT)
        self.server = None
        try:
            deadline = time.monotonic() + 10
            while not marker.exists() or not marker.read_text().strip():
                assert time.monotonic() < deadline and self.wm.poll() is None, "private compositor did not start"
                time.sleep(.05)
            self.env["WAYLAND_DISPLAY"] = marker.read_text().strip()
            subprocess.run(["wlr-randr", "--output", "HEADLESS-1", "--custom-mode", "1920x1200@60", "--scale", os.environ.get("REX_ACCEPTANCE_SCALE", "1")], env=self.env, check=True, timeout=10)
            stub = work / "bin"
            stub.mkdir()
            (stub / "tailscale").write_text("#!/bin/sh\nexit 1\n")
            (stub / "tailscale").chmod(0o755)
            self.env["PATH"] = str(stub) + os.pathsep + self.env["PATH"]
            keymap = work / "us.xkb"
            keymap.write_text(subprocess.check_output(["xkbcli", "compile-keymap", "--layout", "us"], env=self.env, text=True, timeout=10))
            driver = os.environ.get("REX_VPTR", str(ROOT / "tools/vptr/target/release/vptr"))
            self.server = subprocess.Popen([driver, "1920", "1200", str(keymap)], env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, stderr=(work / "vptr.log").open("w"))
            assert select.select([self.server.stdout], [], [], 10)[0], "virtual input did not start"
            assert self.server.stdout.readline().strip() == "ready"
            self.focused = None
            self.trace = None
        except BaseException:
            self.close()
            raise

    def _windows(self):
        trace = self.trace.read_text(errors="replace") if self.trace else ""
        created = re.findall(r"get_toplevel\(new id xdg_toplevel[#@](\d+)\)", trace)
        destroyed = set(re.findall(r"xdg_toplevel[#@](\d+)\.destroy\(", trace))
        return [window for window in created if window not in destroyed]

    def _focused_window(self):
        trace = self.trace.read_text(errors="replace")
        surfaces = dict(re.findall(r"get_xdg_surface\(new id xdg_surface[#@](\d+), wl_surface[#@](\d+)\)", trace))
        tops = {top:surface for surface,top in re.findall(r"xdg_surface[#@](\d+)\.get_toplevel\(new id xdg_toplevel[#@](\d+)\)", trace)}
        entries = re.findall(r"wl_keyboard[#@]\d+\.enter\(\d+, wl_surface[#@](\d+),", trace)
        focused = next((top for top,surface in tops.items() if entries and surfaces.get(surface) == entries[-1]), None)
        return focused

    def launch(self, sid, config, size="1440x900", extra_env=None, log=None, binary=None):
        env = dict(self.env, **(extra_env or {}), WAYLAND_DEBUG="client")
        env.pop("DISPLAY", None)
        self.trace = Path(log.name)
        app = subprocess.Popen([binary, "--session", sid, "--config", str(config), "--window-size", size], env=env, stdout=log, stderr=log)
        self.apps.append(app)
        deadline = time.monotonic() + 12
        while not self._windows():
            assert app.poll() is None and time.monotonic() < deadline, "Wayland app did not open"
            time.sleep(.05)
        self.focused = self._windows()[0]
        time.sleep(1.5)
        return app, self.focused

    def drive(self, *lines):
        self.server.stdin.write("".join(line + "\n" for line in lines))
        self.server.stdin.flush()
        time.sleep(.04)

    def x(self, *args):
        if args[0] == "getwindowgeometry":
            assert self.maximize, "geometry is defined only for this helper's maximized fixture"
            return f"X=0\nY=0\nWIDTH={round(1920/self.scale)}\nHEIGHT={round(1200/self.scale)}"
        if args[0] == "mousemove":
            x,y = map(float,args[-2:])
            self.drive(f"move {round(x*self.scale)} {round(y*self.scale)}")
            return ""
        if args[0] in ("mousedown","mouseup","click"):
            button = {1:272,2:274,3:273}[int(args[1])]
            if args[0] != "mouseup": self.drive(f"down {button}")
            if args[0] != "mousedown": self.drive(f"up {button}")
            return ""
        if args[0] == "search":
            return "\n".join(self._windows())
        if args[0] == "windowfocus":
            target = args[1]
            assert target in self._windows(), "requested private window no longer exists"
            if self._focused_window() == target:
                self.focused = target
                return ""
            # Hold Alt across each cycle: repeated complete Alt-Tab strokes toggle
            # only the last two windows. Confirm real keyboard focus after release.
            for count in range(1, len(self._windows()) + 1):
                self.drive("kdown 56", "mods 8")
                try:
                    for _ in range(count):
                        self.drive("kdown 15", "kup 15")
                finally:
                    self.drive("kup 56", "mods 0")
                deadline = time.monotonic() + .6
                while time.monotonic() < deadline:
                    if self._focused_window() == target:
                        self.focused = target
                        return ""
                    time.sleep(.025)
            raise AssertionError(f"could not focus private Wayland window {target}")
        raise AssertionError(f"Unsupported Wayland acceptance operation: {args}")

    def key(self, *strokes):
        for stroke in strokes:
            mods, name = ([], "+") if stroke == "+" else (stroke.split("+")[:-1], stroke.split("+")[-1])
            name = {"Return":"Enter", "comma":","}.get(name, name)
            mask = sum({"ctrl":4, "shift":1, "alt":8}[m] for m in mods)
            code, shifted = KEYMAP["KEYS"].get(name, KEYMAP["CHARS"].get(name))
            mask |= int(shifted)
            held = [code for bit, code in ((4,29),(1,42),(8,56)) if mask & bit]
            self.server.stdin.write("".join(f"kdown {code}\n" for code in held) + f"mods {mask}\nkdown {code}\n")
            self.server.stdin.flush()
            time.sleep(.025)
            self.server.stdin.write(f"kup {code}\n" + "".join(f"kup {c}\n" for c in held) + "mods 0\n")
            self.server.stdin.flush()
            time.sleep(.04)

    def type(self, text):
        for char in text:
            self.key(char)

    def capture(self, window, name):
        out = self.work / "captures"
        out.mkdir(exist_ok=True)
        subprocess.run(["grim", "-o", "HEADLESS-1", str(out / f"{name}.png")], env=self.env, check=True, timeout=10)

    def close(self):
        for process in [*self.apps, self.server, self.wm]:
            if process is None:
                continue
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
