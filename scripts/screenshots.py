#!/usr/bin/env python3
"""Regenerate site/img/{file,folder,merge}.png from small native macOS windows.

Requires Python 3, Rust, Xcode command-line tools, and Screen Recording permission
for the terminal running this script. Uses stdlib only. Personal MiniDiff settings
and existing windows are untouched; each capture has its own temporary profile.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent


class FrameNotReady(RuntimeError):
    """A visible window did not produce a complete frame within the deadline."""


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def png_info(path):
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise RuntimeError(f"Invalid screenshot: {path}")
    dimensions = struct.unpack(">II", data[16:24])
    # Ignore metadata timestamps when checking whether the rendered frame settled.
    pixels = hashlib.sha256()
    offset = 8
    while offset < len(data):
        length = struct.unpack(">I", data[offset:offset + 4])[0]
        if data[offset + 4:offset + 8] == b"IDAT":
            pixels.update(data[offset + 8:offset + 8 + length])
        offset += length + 12
    return dimensions, pixels.digest()


def capture(binary, helper, args, profile, destination, width, height, log, display):
    # winit stores global positions multiplied by the target display's scale.
    scale = display["scale"]
    x, y = display["x"] + 80, display["y"] + 80
    window = (
        f"(inner_position_pixels:Some((x:{x * scale},y:{(y + 28) * scale})),"
        f"outer_position_pixels:Some((x:{x * scale},y:{y * scale})),fullscreen:false,"
        f"maximized:false,inner_size_points:Some((x:{width}.0,y:{height}.0)))"
    )
    # eframe stores a RON map of strings. JSON is valid RON for this map.
    profile.write_text(json.dumps({
        "window": window,
        "app": "(theme:Dark,auto_update:false,view:(font_size:13.0))",
    }))
    bundle = binary.parent.parent.parent
    # LaunchServices supplies the native app lifecycle and initial drawing events.
    run("open", "-n", "--env", f"MINIDIFF_STORAGE_PATH={profile}",
        "--stdout", str(log), "--stderr", str(log), str(bundle), "--args",
        *(str(ROOT / arg) if arg.startswith("samples/") else arg for arg in args),
        cwd=ROOT)
    pid = None
    deadline = time.monotonic() + 20
    started = time.monotonic()
    previous = None
    settled = 0
    captures = 0
    info = None
    try:
        while time.monotonic() < deadline:
            if pid is None:
                pid = json.loads(run(
                    str(helper), "pid", str(bundle), capture_output=True, text=True
                ).stdout)
            if pid:
                info = json.loads(run(
                    str(helper), "window", str(pid), capture_output=True, text=True
                ).stdout)
            if info and time.monotonic() - started >= 2:
                if info["width"] != width or abs(info["height"] - height - 28) > 4:
                    raise RuntimeError(
                        f"Window is {info['width']}×{info['height']}, not the requested "
                        f"{width}×{height} content size. Make room on your display."
                    )
                if not (display["x"] <= info["x"] and display["y"] <= info["y"]
                        and info["x"] + info["width"] <= display["x"] + display["width"]
                        and info["y"] + info["height"] <= display["y"] + display["height"]):
                    raise RuntimeError(f"Capture window is not entirely on {display['name']}: {info}")
                result = subprocess.run([
                    "/usr/sbin/screencapture", "-x", "-o", "-l", str(info["id"]),
                    str(destination),
                ], capture_output=True, text=True)
                if result.returncode:
                    raise RuntimeError(
                        "Window capture failed. Allow Screen Recording for your terminal "
                        f"in System Settings, then retry.\n{result.stderr}"
                    )
                captures += 1
                colours = int(run(
                    str(helper), "colours", str(destination), capture_output=True, text=True
                ).stdout)
                if colours < 12:
                    previous = None
                    settled = 0
                    time.sleep(0.5)
                    continue
                dimensions, digest = png_info(destination)
                expected = (round(info["width"] * scale), round(info["height"] * scale))
                if dimensions != expected:
                    raise RuntimeError(
                        f"Screenshot density mismatch: got {dimensions}, expected {expected} "
                        f"for {scale:g}× capture on {display['name']}."
                    )
                settled = settled + 1 if digest == previous else 0
                previous = digest
                if settled >= 2:
                    return png_info(destination)[0]
            time.sleep(0.5)
        detail = log.read_text() if log.exists() else "No app log."
        raise FrameNotReady(
            f"Screenshot did not settle: {destination.name} "
            f"(pid={pid}, window={info}, frames={captures}).\n{detail}"
        )
    finally:
        # Terminate only our instance, never an existing user window or merge.
        if pid:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            # Finish disposing of this app before launching the next capture instance.
            for _ in range(20):
                if run(str(helper), "pid", str(bundle), capture_output=True, text=True).stdout.strip() == "null":
                    break
                time.sleep(0.1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--display", choices=("internal", "main"), default="internal",
                        help="Capture display (default: internal, verified at 2× density)")
    parser.add_argument("--binary", type=Path, help="Use an existing binary instead of cargo build")
    parser.add_argument("--width", type=int, default=880, help="App window content width in points (default: 880)")
    parser.add_argument("--height", type=int, default=520, help="App window content height in points (default: 520)")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("Native screenshot capture requires macOS.")
    if args.width < 640 or args.height < 400:
        parser.error("MiniDiff's minimum window size is 640×400 points.")
    for command in ("swiftc", "codesign"):
        if not shutil.which(command):
            parser.error(f"Missing {command}; install the Xcode command-line tools.")
    if args.binary:
        binary = args.binary.resolve()
    else:
        run("cargo", "build", "--locked", cwd=ROOT)
        binary = ROOT / "target/debug/minidiff"
    if not binary.is_file():
        parser.error(f"Binary not found: {binary}")

    with tempfile.TemporaryDirectory(prefix="minidiff-screenshots-") as scratch:
        temp = Path(scratch)
        # A real bundle gives macOS the correct high-resolution window behaviour.
        bundle = temp / "MiniDiff.app/Contents"
        executable = bundle / "MacOS/minidiff"
        executable.parent.mkdir(parents=True)
        shutil.copy2(binary, executable)
        resources = bundle / "Resources"
        resources.mkdir()
        shutil.copy2(ROOT / "assets/MiniDiff.icns", resources / "MiniDiff.icns")
        with (ROOT / "macos/Info.plist").open("rb") as source:
            plist = plistlib.load(source)
        plist["CFBundleIdentifier"] = "ch.signalwerk.minidiff.screenshots"
        plist["CFBundleShortVersionString"] = plist["CFBundleVersion"] = "0.0.0"
        with (bundle / "Info.plist").open("wb") as output:
            plistlib.dump(plist, output)
        run("codesign", "--force", "--sign", "-", str(bundle.parent), capture_output=True)
        tools = ROOT / "target/screenshot-tools"
        tools.mkdir(parents=True, exist_ok=True)
        helper = tools / "window-info"
        source = ROOT / "scripts/screenshot-window.swift"
        if not helper.exists() or helper.stat().st_mtime < source.stat().st_mtime:
            run("swiftc", "-module-cache-path", str(tools / "swift-cache"),
                str(source), "-o", str(helper))
        display = json.loads(run(
            str(helper), "display", args.display, capture_output=True, text=True
        ).stdout)
        if display is None:
            raise RuntimeError(f"The {args.display} display is not available. Open the laptop display and retry.")
        if args.display == "internal" and display["scale"] != 2:
            raise RuntimeError(f"The internal display is not at 2× density: {display}")
        if args.width + 160 > display["width"] or args.height + 188 > display["height"]:
            raise RuntimeError(f"The requested window does not fit on {display['name']} with capture margins.")
        print(f"Using {display['name']} at {display['scale']:g}× density.", flush=True)
        examples = {
            "file": ["--label", "format.ts", "--label", "format.ts",
                     "samples/left/src/util/format.ts", "samples/right/src/util/format.ts"],
            "folder": ["samples/left", "samples/right"],
            "merge": ["--merge", "--label", "Local", "--label", "Base", "--label", "Remote",
                      "samples/merge/local.rs", "samples/merge/remote.rs", "samples/merge/base.rs",
                      str(temp / "result.rs")],
        }
        dimensions = {}
        for name, launch in examples.items():
            for attempt in range(3):
                try:
                    dimensions[name] = capture(
                        executable, helper, launch, temp / f"{name}.ron", temp / f"{name}.png",
                        args.width, args.height, temp / f"{name}.log", display,
                    )
                    break
                except FrameNotReady:
                    if attempt == 2:
                        raise
                    print(f"Retrying {name}: macOS has not supplied a rendered frame.", flush=True)
            print(f"Captured {name}: {dimensions[name][0]}×{dimensions[name][1]} pixels", flush=True)
        # Publish only after all three captures succeeded; keep intrinsic HTML sizes in sync.
        page = ROOT / "site/index.html"
        html = page.read_text()
        for name, (width, height) in dimensions.items():
            html, count = re.subn(
                rf'(<img src="img/{name}\.png"[^>]*\bwidth=")[0-9]+(" height=")[0-9]+(")',
                lambda m: f'{m[1]}{width}{m[2]}{height}{m[3]}', html,
            )
            if count != 1:
                raise RuntimeError(f"Cannot update the HTML dimensions for {name}.png")
        for name in examples:
            shutil.copyfile(temp / f"{name}.png", ROOT / f"site/img/{name}.png")
        page.write_text(html)
        print("Updated site/img/{file,folder,merge}.png and their HTML dimensions.")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError, OSError) as error:
        sys.exit(str(error))
