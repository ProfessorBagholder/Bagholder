#!/usr/bin/env python3
"""Runs the Python app in python/ from the repository root: `python3 bagholder.py`.

The server, its supervisor and the in-app update behave exactly as when
python/bagholder.py is run directly.
"""
import os
import runpy
import sys

APP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "python")
sys.path[0] = APP
sys.argv[0] = os.path.join(APP, "bagholder.py")
runpy.run_path(sys.argv[0], run_name="__main__")
