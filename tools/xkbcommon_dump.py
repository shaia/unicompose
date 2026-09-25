#!/usr/bin/env python3
"""Dumps the compose table libxkbcommon builds from a Compose file, for the differential
test in crates/uc-xcompose/tests/differential.rs.

Usage: python3 tools/xkbcommon_dump.py COMPOSE_FILE > dump.txt

Needs only the libxkbcommon shared library, version 1.6 or newer (for the table
iterator); no headers or xkbcli. Each output line is one rule:

    <keysym hex> <keysym hex> ...<TAB><result keysym hex or -><TAB><JSON string or null>
"""

import ctypes
import ctypes.util
import json
import sys

XKB_COMPOSE_FORMAT_TEXT_V1 = 1


def main(path: str) -> None:
    name = ctypes.util.find_library("xkbcommon") or "libxkbcommon.so.0"
    xkb = ctypes.CDLL(name)
    xkb.xkb_context_new.restype = ctypes.c_void_p
    xkb.xkb_compose_table_new_from_buffer.restype = ctypes.c_void_p
    xkb.xkb_compose_table_new_from_buffer.argtypes = [
        ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t, ctypes.c_char_p, ctypes.c_int, ctypes.c_int]
    xkb.xkb_compose_table_iterator_new.restype = ctypes.c_void_p
    xkb.xkb_compose_table_iterator_new.argtypes = [ctypes.c_void_p]
    xkb.xkb_compose_table_iterator_next.restype = ctypes.c_void_p
    xkb.xkb_compose_table_iterator_next.argtypes = [ctypes.c_void_p]
    xkb.xkb_compose_table_entry_sequence.restype = ctypes.POINTER(ctypes.c_uint32)
    xkb.xkb_compose_table_entry_sequence.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t)]
    xkb.xkb_compose_table_entry_keysym.restype = ctypes.c_uint32
    xkb.xkb_compose_table_entry_keysym.argtypes = [ctypes.c_void_p]
    xkb.xkb_compose_table_entry_utf8.restype = ctypes.c_char_p
    xkb.xkb_compose_table_entry_utf8.argtypes = [ctypes.c_void_p]

    with open(path, "rb") as f:
        data = f.read()
    context = xkb.xkb_context_new(0)
    table = xkb.xkb_compose_table_new_from_buffer(
        context, data, len(data), b"C", XKB_COMPOSE_FORMAT_TEXT_V1, 0)
    if not table:
        sys.exit(f"libxkbcommon could not load {path}")
    iterator = xkb.xkb_compose_table_iterator_new(table)
    out = sys.stdout
    while True:
        entry = xkb.xkb_compose_table_iterator_next(iterator)
        if not entry:
            break
        length = ctypes.c_size_t()
        sequence = xkb.xkb_compose_table_entry_sequence(entry, ctypes.byref(length))
        keys = " ".join(f"{sequence[i]:x}" for i in range(length.value))
        keysym = xkb.xkb_compose_table_entry_keysym(entry)
        utf8 = xkb.xkb_compose_table_entry_utf8(entry).decode("utf-8")
        result = f"{keysym:x}" if keysym else "-"
        text = json.dumps(utf8 if utf8 else None, ensure_ascii=True)
        out.write(f"{keys}\t{result}\t{text}\n")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
