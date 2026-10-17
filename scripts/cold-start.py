#!/usr/bin/env python3
"""Cold start to first paint under Hyprland.

Launches the binary and waits for Hyprland's `openwindow` event for the
`gutenberg` class. A Wayland window maps when its first frame is committed,
so the event marks first paint. Prints each run in ms, then the median.

    scripts/cold-start.py target/release/gutenberg [runs]
"""
import os
import socket
import statistics
import subprocess
import sys
import time


def socket2():
    runtime = os.environ["XDG_RUNTIME_DIR"]
    sig = os.environ["HYPRLAND_INSTANCE_SIGNATURE"]
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(f"{runtime}/hypr/{sig}/.socket2.sock")
    return s


def once(binary):
    events = socket2()
    start = time.perf_counter()
    proc = subprocess.Popen([binary], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    buf = b""
    try:
        events.settimeout(5)
        while True:
            buf += events.recv(4096)
            *lines, buf = buf.split(b"\n")
            for line in lines:
                if line.startswith(b"openwindow>>") and b",gutenberg," in line:
                    return (time.perf_counter() - start) * 1000
    finally:
        proc.terminate()
        proc.wait()
        events.close()
        time.sleep(0.3)


def main():
    binary = sys.argv[1]
    runs = int(sys.argv[2]) if len(sys.argv) > 2 else 10
    times = [once(binary) for _ in range(runs)]
    for t in times:
        print(f"{t:.1f} ms")
    print(f"median {statistics.median(times):.1f} ms")


if __name__ == "__main__":
    main()
