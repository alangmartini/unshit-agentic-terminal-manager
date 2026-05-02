#!/usr/bin/env python3
"""Capture a Win32 window with PrintWindow without focusing it.

This helper intentionally uses only the Python standard library. It writes a
32-bit BMP because BMP has a small, dependency-free file format; the PowerShell
harness converts it to PNG after a successful capture.
"""

from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import os
import struct
import sys


BI_RGB = 0
DIB_RGB_COLORS = 0
PW_RENDERFULLCONTENT = 0x00000002


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [
        ("biSize", wintypes.DWORD),
        ("biWidth", wintypes.LONG),
        ("biHeight", wintypes.LONG),
        ("biPlanes", wintypes.WORD),
        ("biBitCount", wintypes.WORD),
        ("biCompression", wintypes.DWORD),
        ("biSizeImage", wintypes.DWORD),
        ("biXPelsPerMeter", wintypes.LONG),
        ("biYPelsPerMeter", wintypes.LONG),
        ("biClrUsed", wintypes.DWORD),
        ("biClrImportant", wintypes.DWORD),
    ]


class BITMAPINFO(ctypes.Structure):
    _fields_ = [
        ("bmiHeader", BITMAPINFOHEADER),
        ("bmiColors", wintypes.DWORD * 3),
    ]


user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)

user32.GetWindowDC.argtypes = [wintypes.HWND]
user32.GetWindowDC.restype = wintypes.HDC
user32.ReleaseDC.argtypes = [wintypes.HWND, wintypes.HDC]
user32.ReleaseDC.restype = ctypes.c_int
user32.PrintWindow.argtypes = [wintypes.HWND, wintypes.HDC, wintypes.UINT]
user32.PrintWindow.restype = wintypes.BOOL
user32.IsIconic.argtypes = [wintypes.HWND]
user32.IsIconic.restype = wintypes.BOOL

gdi32.CreateCompatibleDC.argtypes = [wintypes.HDC]
gdi32.CreateCompatibleDC.restype = wintypes.HDC
gdi32.CreateCompatibleBitmap.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
gdi32.CreateCompatibleBitmap.restype = wintypes.HBITMAP
gdi32.SelectObject.argtypes = [wintypes.HDC, wintypes.HGDIOBJ]
gdi32.SelectObject.restype = wintypes.HGDIOBJ
gdi32.DeleteObject.argtypes = [wintypes.HGDIOBJ]
gdi32.DeleteObject.restype = wintypes.BOOL
gdi32.DeleteDC.argtypes = [wintypes.HDC]
gdi32.DeleteDC.restype = wintypes.BOOL
gdi32.GetDIBits.argtypes = [
    wintypes.HDC,
    wintypes.HBITMAP,
    wintypes.UINT,
    wintypes.UINT,
    wintypes.LPVOID,
    ctypes.POINTER(BITMAPINFO),
    wintypes.UINT,
]
gdi32.GetDIBits.restype = ctypes.c_int


def last_error_message(prefix: str) -> str:
    error = ctypes.get_last_error()
    return f"{prefix} failed with Win32 error {error}"


def parse_hwnd(value: str) -> int:
    return int(value, 0)


def bitmap_looks_blank(pixels: bytes, width: int, height: int) -> bool:
    if not pixels:
        return True

    first = pixels[0:4]
    step_x = max(1, width // 96)
    step_y = max(1, height // 64)
    for y in range(0, height, step_y):
        row = y * width * 4
        for x in range(0, width, step_x):
            offset = row + x * 4
            pixel = pixels[offset : offset + 4]
            # BMP pixels are BGRA here.
            delta = (
                abs(pixel[0] - first[0])
                + abs(pixel[1] - first[1])
                + abs(pixel[2] - first[2])
            )
            if delta > 12:
                return False
    return True


def write_bmp(path: str, width: int, height: int, pixels: bytes) -> None:
    header_size = 14 + 40
    image_size = len(pixels)
    file_size = header_size + image_size
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)

    with open(path, "wb") as out:
        out.write(struct.pack("<2sIHHI", b"BM", file_size, 0, 0, header_size))
        out.write(
            struct.pack(
                "<IiiHHIIiiII",
                40,
                width,
                -height,
                1,
                32,
                BI_RGB,
                image_size,
                0,
                0,
                0,
                0,
            )
        )
        out.write(pixels)


def capture_window(hwnd: int, width: int, height: int, flags: int) -> bytes:
    if width <= 0 or height <= 0:
        raise ValueError(f"invalid capture size {width}x{height}")
    if user32.IsIconic(wintypes.HWND(hwnd)):
        raise RuntimeError("window is minimized; live GPU content is usually unavailable")

    window_dc = user32.GetWindowDC(wintypes.HWND(hwnd))
    if not window_dc:
        raise RuntimeError(last_error_message("GetWindowDC"))

    memory_dc = None
    bitmap = None
    old_object = None
    try:
        memory_dc = gdi32.CreateCompatibleDC(window_dc)
        if not memory_dc:
            raise RuntimeError(last_error_message("CreateCompatibleDC"))

        bitmap = gdi32.CreateCompatibleBitmap(window_dc, width, height)
        if not bitmap:
            raise RuntimeError(last_error_message("CreateCompatibleBitmap"))

        old_object = gdi32.SelectObject(memory_dc, bitmap)
        if not old_object:
            raise RuntimeError(last_error_message("SelectObject"))

        if not user32.PrintWindow(wintypes.HWND(hwnd), memory_dc, flags):
            raise RuntimeError(last_error_message("PrintWindow"))

        info = BITMAPINFO()
        info.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
        info.bmiHeader.biWidth = width
        info.bmiHeader.biHeight = -height
        info.bmiHeader.biPlanes = 1
        info.bmiHeader.biBitCount = 32
        info.bmiHeader.biCompression = BI_RGB
        info.bmiHeader.biSizeImage = width * height * 4

        pixel_buffer = ctypes.create_string_buffer(info.bmiHeader.biSizeImage)
        rows = gdi32.GetDIBits(
            memory_dc,
            bitmap,
            0,
            height,
            pixel_buffer,
            ctypes.byref(info),
            DIB_RGB_COLORS,
        )
        if rows != height:
            raise RuntimeError(last_error_message("GetDIBits"))
        return pixel_buffer.raw
    finally:
        if memory_dc and old_object:
            gdi32.SelectObject(memory_dc, old_object)
        if bitmap:
            gdi32.DeleteObject(bitmap)
        if memory_dc:
            gdi32.DeleteDC(memory_dc)
        user32.ReleaseDC(wintypes.HWND(hwnd), window_dc)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--hwnd", required=True, type=parse_hwnd)
    parser.add_argument("--width", required=True, type=int)
    parser.add_argument("--height", required=True, type=int)
    parser.add_argument("--out", required=True)
    parser.add_argument("--flags", type=int, default=PW_RENDERFULLCONTENT)
    parser.add_argument("--allow-blank", action="store_true")
    args = parser.parse_args()

    try:
        pixels = capture_window(args.hwnd, args.width, args.height, args.flags)
        if not args.allow_blank and bitmap_looks_blank(pixels, args.width, args.height):
            raise RuntimeError("PrintWindow returned a blank image")
        write_bmp(args.out, args.width, args.height, pixels)
        return 0
    except Exception as exc:
        print(f"capture_window.py: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
