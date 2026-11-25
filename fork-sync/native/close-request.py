"""Prove native xdg_toplevel.close consent using one private Wayland compositor."""

import hashlib
import json
import re
import subprocess
import sys
import time
from pathlib import Path

from native_session import session
from reproduce import OWNED, sway


def windows(node):
    yield node
    for child in node.get("nodes", []) + node.get("floating_nodes", []):
        yield from windows(child)


def window(env, child, title):
    matches = [
        node for node in windows(sway(env, "-t", "get_tree"))
        if node.get("pid") == child.pid and node.get("name") == title
    ]
    assert len(matches) <= 1, (title, matches)
    if not matches:
        return None
    node = matches[0]
    assert node.get("shell") == "xdg_shell", node
    assert node.get("app_id") == "org.rune.CloseRequestProbe", node
    return node["id"]


def wait(child, predicate, event, log):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        if child.poll() is not None:
            break
        time.sleep(.05)
    raise AssertionError(f"{event} absent from compositor or log\n{log.read_text()[-4000:]}")


def command(env, identity, action):
    response = sway(env, f"[con_id={identity}] {action}")
    assert response and all(reply.get("success") for reply in response), response


def send_key(env, identity, injector, key):
    command(env, identity, "focus")
    evdev = {"k": 37, "p": 25, "o": 24}
    injector.stdin.write(f"tap {evdev[key]}\n".encode())
    injector.stdin.flush()


binary = Path(sys.argv[1]).resolve(strict=True)
output = Path(sys.argv[2])
injector_path = Path(sys.argv[3]).resolve(strict=True)
output.mkdir(parents=True, exist_ok=False)
results = []
with session(output) as env, (output / "injector.log").open("wb", buffering=0) as inject_log:
    injector = OWNED.start([str(injector_path)], env=env, stdin=subprocess.PIPE,
                           stdout=inject_log, stderr=subprocess.STDOUT)
    wait(injector, lambda: "layout us" in (output / "injector.log").read_text(), "keymap ready", output / "injector.log")
    env.update(RUNE_CPU_FIRST="only", WAYLAND_DEBUG="client")
    for case, native_closes in (("app", 2), ("programmatic", 1), ("neighbor", 3), ("default", 1)):
        case_env = dict(env, RUNE_CLOSE_REQUEST_CASE=case)
        path = output / f"{case}.log"
        with path.open("wb", buffering=0) as log:
            child = OWNED.start([str(binary)], env=case_env, stdin=subprocess.DEVNULL,
                                stdout=log, stderr=subprocess.STDOUT)
            seen = lambda marker: marker in path.read_text()
            primary = lambda: window(env, child, "Close probe")
            neighbor = lambda: window(env, child, "Close probe neighbor")
            identity = wait(child, primary, "primary window", path)
            wait(child, lambda: seen("primary-ready"), "primary ready", path)
            wait(child, lambda: re.search(r"wl_keyboard#\d+\.keymap\(", path.read_text()), "client keymap", path)
            command(env, identity, "kill")
            if case != "default":
                wait(child, lambda: seen("confirmation-drawn"), "confirmation frame", path)
                assert primary() == identity
                assert path.read_text().splitlines().count("close-request 1") == 1
                assert not seen("primary-closed")
                send_key(env, identity, injector, "k")
                wait(child, lambda: seen("input-after-defer"), "input after defer", path)
                if case == "programmatic":
                    send_key(env, identity, injector, "p")
                else:
                    if case == "neighbor":
                        send_key(env, identity, injector, "o")
                        second = wait(child, neighbor, "neighbor window", path)
                        wait(child, lambda: seen("neighbor-ready"), "neighbor ready", path)
                        assert primary() == identity and second != identity
                    command(env, identity, "kill")
                    if case == "neighbor":
                        wait(child, lambda: seen("primary-closed"), "primary close", path)
                        wait(child, lambda: primary() is None, "primary unmap", path)
                        assert neighbor() == second and not seen("neighbor-closed")
                        command(env, second, "kill")
            assert child.wait(timeout=12) == 0, path.read_text()[-4000:]
        lines = path.read_text().splitlines()
        assert lines.count("primary-closed") == 1
        assert lines.count(f"wayland-close-request: pass {case}") == 1
        events = re.findall(r"xdg_toplevel#(\d+)\.close\(\)", "\n".join(lines))
        received = len(events)
        assert received == native_closes, (case, received, native_closes)
        if case in ("app", "neighbor"):
            assert events[0] == events[1], "both primary closes must target the same toplevel"
        if case == "neighbor":
            assert events[2] != events[0], "neighbor close must target its own toplevel"
        result = {"case": case, "native_close_events": received, "exit": 0}
        results.append(result)
        print(json.dumps(result), flush=True)

(output / "result.json").write_text(json.dumps({
    "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "cases": results,
}, indent=2) + "\n")
