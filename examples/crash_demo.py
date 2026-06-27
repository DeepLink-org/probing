#!/usr/bin/env python3
"""单进程 crash demo — ``PROBING=1 python examples/crash_demo.py [--mode exception]``.

Probing 由 site hook 自动加载；本脚本不 import probing。
``record``：后台线程异常，写入 memtable 后正常退出；``exception``：主线程崩溃。
"""

from __future__ import annotations

import argparse
import os
import threading


def _crash(*, record_only: bool) -> None:
    def boom() -> None:
        def inner() -> None:
            raise ValueError("demo crash from crash_demo.py")

        inner()

    if record_only:
        t = threading.Thread(target=boom, name="crash-demo", daemon=True)
        t.start()
        t.join()
    else:
        boom()


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--mode", choices=("record", "exception"), default="record")
    args = p.parse_args()

    os.environ.setdefault("RANK", "0")
    os.environ.setdefault("LOCAL_RANK", "0")

    _crash(record_only=args.mode == "record")


if __name__ == "__main__":
    main()
