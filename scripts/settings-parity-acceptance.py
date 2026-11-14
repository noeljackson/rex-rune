#!/usr/bin/env python3
"""Private Mac Settings parity checks on X11 or native Wayland.

Requires a built REX_BIN, rex and the selected backend's dependencies from
private-wayland.py/rune-acceptance-x11.py. Uses a fresh Rex server, Unix socket,
HOME/XDG tree and compositor; never the user's sessions or input devices.
REX_ACCEPTANCE_BACKEND=wayland and REX_ACCEPTANCE_SCALE=1.5 select fractional
Wayland. Logs, captures and reaped PIDs remain in the printed owned cache.
"""
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

ROOT = Path(os.environ.get("REX_PARITY_ROOT", Path(__file__).resolve().parents[1]))
BINARY = os.environ.get("REX_BIN", str(ROOT / "target/debug/rex-linux"))
REX = shutil.which("rex")


def wait(predicate, message, seconds=10):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        if predicate():
            return
        time.sleep(.05)
    raise AssertionError(message)


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run():
    assert REX and Path(BINARY).is_file()
    work = Path(tempfile.mkdtemp(prefix=f"cwt-rex-settings-{os.getpid()}-", dir=Path.home()/".cache"))
    (work/"OWNER").write_text("Private Settings parity acceptance; owns all recorded PIDs.\n")
    print(f"Private Settings artifacts: {work}", flush=True)
    for name in ("home", "config", "state", "runtime"):
        (work/name).mkdir(mode=0o700)
    endpoint = "unix://" + str(work/"runtime/server.sock")
    env = {k:v for k,v in os.environ.items() if not k.startswith("REX_")}
    env.update(HOME=str(work/"home"), XDG_CONFIG_HOME=str(work/"config"),
               XDG_STATE_HOME=str(work/"state"), XDG_RUNTIME_DIR=str(work/"runtime"),
               REX_SERVER=endpoint, REX_LINUX_NO_UPDATE_CHECK="1", SHELL="/bin/sh")
    for key in ("DISPLAY", "WAYLAND_DISPLAY", "ENV", "BASH_ENV", "ZDOTDIR"):
        env.pop(key, None)
    owned = []
    desktop = None

    def cli(*args):
        return subprocess.run([REX,"--autostart=false","--server",endpoint,*args],env=env,capture_output=True,text=True,timeout=12,check=True)

    def api(method, **args):
        return json.loads(cli("api","call",method,json.dumps(args)).stdout)

    def stop(process):
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)

    try:
        init=work/"init.lua"
        init.write_text("-- private Settings fixture\n")
        server=subprocess.Popen([REX,"--autostart=false","--server",endpoint,"server","run","--listen",endpoint,"--restore=false","--config",str(init),"--state-dir",str(work/"server-state")],env=env,stdout=(work/"server.log").open("w"),stderr=subprocess.STDOUT)
        owned.append(server)
        wait(lambda:(work/"runtime/server.sock").is_socket(),"server did not start")
        shell={"block":{"flavor":"com.superlogical.terminal.shell","options":{"shell":"/bin/sh"}}}
        sid=api("session.create",label="settings-reference",initial_windows=[{"window_label":"sample terminal","layout":shell},{"window_label":"reference tab","layout":shell}])["session_id"]
        original=api("session.view",session_id=sid)
        source_index=next((i for i,w in enumerate(original["windows"]) if w["window_id"]==original.get("active_window_id")),0)
        block=original["windows"][source_index]["focused_block_id"]
        original_blocks={b["block_id"] for w in original["windows"] for l in w["layers"] for b in l["blocks"]}
        preferences={"client_label":"settings-accept","hosts":[{"label":"private","endpoint":endpoint,"shell":"/bin/sh"}],"notifications":{"system_notifications":False},"font_size":15,"pane_padding":8,"pane_header":"always","ui_scale":1,"vertical_tabs":True,"sidebar_open":True}
        config=work/"workspace.json"
        config.write_text(json.dumps(preferences))
        if os.environ.get("REX_ACCEPTANCE_BACKEND")=="wayland":
            desktop=load("private_wayland",ROOT/"scripts/private-wayland.py").Desktop(work,env,maximize=True)
        else:
            module=load("private_x11",ROOT/"scripts/rune-acceptance-x11.py")
            module.OUT=work/"captures"
            desktop=module.Desktop()
        desktop.env={k:v for k,v in desktop.env.items() if not k.startswith("REX_")}
        launch_env={k:env[k] for k in ("HOME","XDG_CONFIG_HOME","XDG_STATE_HOME","XDG_RUNTIME_DIR","REX_SERVER","REX_LINUX_NO_UPDATE_CHECK","SHELL")}
        app,window=desktop.launch(sid,config,size=os.environ.get("REX_ACCEPTANCE_WINDOW_SIZE","1440x900"),extra_env=launch_env,log=(work/"app.log").open("w"),binary=BINARY)
        owned.append(app)
        def view():
            return api("session.view",session_id=sid)
        def content():
            return api("com.superlogical.terminal.format",session_id=sid,block_id=block,args={"format":"text","trim":True})["content"]
        def ready(session=sid):
            clients=api("client.list")["clients"]
            cid=next((c["client_id"] for c in clients if c.get("info",{}).get("label")=="settings-accept" and c.get("connection_state")=="connected"),None)
            return cid and any(b.get("owner")==cid for w in api("session.view",session_id=session)["windows"] for l in w["layers"] for b in l["blocks"])
        wait(ready,"client terminal did not attach")
        wait(lambda:"$" in content(),"shell prompt not ready")
        desktop.type("printf 'SETTINGS_PID=%s\\n' \"$$\"")
        desktop.key("Return")
        wait(lambda:re.search(r"SETTINGS_PID=(\d+)",content()),"initial input failed")
        pid=int(re.search(r"SETTINGS_PID=(\d+)",content()).group(1))
        start=Path(f"/proc/{pid}/stat").read_text().split()[21]
        geometry=dict(line.split("=",1) for line in desktop.x("getwindowgeometry","--shell",window).splitlines())
        width,height=int(geometry["WIDTH"]),int(geometry["HEIGHT"])
        dialog_width=min(816,width*.92)
        dialog_height=min(680,height-52-24)
        left,top=(width-dialog_width)/2,(height+52-dialog_height)/2
        def click(x,y):
            desktop.x("mousemove","--window",window,round(x),round(y))
            desktop.x("click",1)
            time.sleep(.3)
        def capture(name):
            desktop.x("mousemove","--window",window,20,60)
            time.sleep(.3)
            desktop.capture(window,name)
        def search(query):
            click(left+70,top+65)
            desktop.key("ctrl+a")
            desktop.type(query)
            time.sleep(.25)
            desktop.key("Return")
            time.sleep(.4)
        desktop.key("ctrl+comma")
        time.sleep(.5)
        capture("settings-interface")
        search("pane padding")
        capture("settings-panes")
        assert json.loads(config.read_text())["pane_padding"] == preferences["pane_padding"]
        # Points are pinned to the padding buttons by the headless geometry check.
        click(left+dialog_width-40,top+293)
        wait(lambda:json.loads(config.read_text())["pane_padding"]==9,"padding control did not apply")
        capture("settings-panes-edited")
        click(left+dialog_width-132,top+293)
        wait(lambda:json.loads(config.read_text())["pane_padding"]==8,"fixture padding was not restored")
        desktop.key("alt+Left")
        time.sleep(.3)
        capture("settings-search-restored")
        search("typography")
        capture("settings-terminal")
        search("tab position")
        capture("settings-tabs")
        search("notification position")
        capture("settings-notifications")
        search("keyboard shortcuts")
        capture("settings-shortcuts")
        desktop.key("alt+Left")
        time.sleep(.3)
        desktop.key("alt+Right")
        time.sleep(.3)
        # Opening and searching Settings must not change real terminal preferences.
        saved=json.loads(config.read_text())
        for key in ("font_size","pane_padding","pane_header","ui_scale","vertical_tabs"):
            assert saved[key]==preferences[key],f"browsing Settings changed {key}"
        assert {b["block_id"] for w in view()["windows"] for l in w["layers"] for b in l["blocks"]}==original_blocks
        assert Path(f"/proc/{pid}/stat").read_text().split()[21]==start
        assert "pane padding" not in content() and "typography" not in content(),"Settings input reached the PTY"
        desktop.key("Escape")
        time.sleep(.3)
        desktop.type("printf 'AFTER_SETTINGS\\n'")
        desktop.key("Return")
        wait(lambda:"AFTER_SETTINGS" in content(),"closing Settings did not restore terminal input")
        capture("pane-controls")
        # Real sidebar switching complements the frame-by-frame tint/geometry regression.
        for i,index in enumerate([1-source_index,source_index,1-source_index,source_index,source_index]):
            desktop.x("mousemove","--window",window,100,210+34*index)
            desktop.x("click",1)
            desktop.capture(window,f"sidebar-switch-{i}-early")
            wait(lambda:view()["active_window_id"]==original["windows"][index]["window_id"],"sidebar click did not select its tab")
            time.sleep(.6)
            desktop.capture(window,f"sidebar-switch-{i}-settled")
        assert {b["block_id"] for w in view()["windows"] for l in w["layers"] for b in l["blocks"]}==original_blocks
        assert Path(f"/proc/{pid}/stat").read_text().split()[21]==start
        print("PASS Settings/search/history/previews preserve preferences, programs and terminal input",flush=True)

        # Session rows in Top tab position have their own terminal glyph. Inserting
        # their matched selection layer used to remount both rows' icon and label.
        # This is a different path from switching windows inside one session above.
        stop(app)
        reference=api("session.create",label="reference-session",initial_windows=[{"window_label":"reference", "layout":shell} for _ in range(2)])["session_id"]
        reference_view=api("session.view",session_id=reference)
        reference_block=next(w["focused_block_id"] for w in reference_view["windows"] if w["window_id"]==reference_view["active_window_id"])
        def block_ids(session):
            return {b["block_id"] for w in api("session.view",session_id=session)["windows"] for l in w["layers"] for b in l["blocks"]}
        reference_blocks=block_ids(reference)
        saved=json.loads(config.read_text())
        saved["vertical_tabs"]=False
        config.write_text(json.dumps(saved))
        app,window=desktop.launch(sid,config,extra_env=launch_env,log=(work/"session-app.log").open("w"),binary=BINARY)
        owned.append(app)
        wait(ready,"reopened client did not attach")
        order=[s["session_id"] for s in api("session.list")["sessions"]]
        def select_session(session, name):
            # Header/row geometry is pinned in the headless session regression.
            desktop.x("mousemove","--window",window,100,184+34*order.index(session))
            desktop.x("click",1)
            # X11's convenience capture settles first; capture directly so these
            # images also show the beginning of the foreground transition.
            def frame(suffix):
                if os.environ.get("REX_ACCEPTANCE_BACKEND")=="wayland":
                    desktop.capture(window,name+suffix)
                else:
                    subprocess.run(["import","-window",str(window),str(work/"captures"/(name+suffix+".png"))],env=desktop.env,check=True,timeout=10)
            frame("-early")
            wait(lambda:json.loads(config.read_text()).get("sessions",{}).get(endpoint)==session,"session row did not select its session")
            time.sleep(.5)
            frame("-settled")
        select_session(reference,"session-reference")
        def reference_content():
            return api("com.superlogical.terminal.format",session_id=reference,block_id=reference_block,args={"format":"text","trim":True})["content"]
        wait(lambda:ready(reference),"reference terminal did not attach")
        wait(lambda:"$" in reference_content(),"reference shell prompt was not ready")
        desktop.type("printf 'REFERENCE_PID=%s\\n' \"$$\"")
        desktop.key("Return")
        wait(lambda:re.search(r"REFERENCE_PID=(\d+)",reference_content()),"reference session input failed")
        reference_pid=int(re.search(r"REFERENCE_PID=(\d+)",reference_content()).group(1))
        reference_start=Path(f"/proc/{reference_pid}/stat").read_text().split()[21]
        for i,session in enumerate([sid,reference,sid,sid,reference,sid]):
            select_session(session,f"session-switch-{i}")
        assert block_ids(sid)==original_blocks and block_ids(reference)==reference_blocks
        assert Path(f"/proc/{pid}/stat").read_text().split()[21]==start
        assert Path(f"/proc/{reference_pid}/stat").read_text().split()[21]==reference_start
        assert "AFTER_SETTINGS" in content() and f"REFERENCE_PID={reference_pid}" in reference_content()
        desktop.type("printf 'AFTER_SESSION_SWITCH\\n'")
        desktop.key("Return")
        wait(lambda:"\nAFTER_SESSION_SWITCH\n" in content(),"returning to the original session lost terminal input")
        print("PASS two-session sidebar switching in Top tab position preserves both original programs",flush=True)
    except BaseException:
        if desktop and "window" in locals():
            try:
                desktop.capture(window,"failure")
            except Exception:
                pass
        raise
    finally:
        for process in reversed(owned):
            stop(process)
        if desktop:
            desktop.close()
            owned.extend([desktop.wm,desktop.server])
        (work/"processes.json").write_text(json.dumps([{"pid":p.pid,"returncode":p.poll()} for p in owned],indent=2)+"\n")


if __name__=="__main__":
    run()
