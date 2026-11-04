#!/usr/bin/env python3
"""Send a tiny Kitty PNG to the terminal for the GNOME rendering test."""

import sys

PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg=="
sys.stdout.write("\x1b_Ga=T,f=100,i=3;" + PNG + "\x1b\\")
sys.stdout.flush()
sys.stdin.read()
