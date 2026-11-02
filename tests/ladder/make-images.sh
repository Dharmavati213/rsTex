#!/usr/bin/env bash
# Generates the images used by rung 12-graphics. They are synthetic on purpose:
# a CMYK JPEG, because a real document in the corpus uses one, and an RGB PNG.
set -eu
cd "$(dirname "$0")"
magick -size 829x379 gradient:'#0060c0'-'#f0d000' -colorspace CMYK -quality 90 -strip ladder-logo.jpg
magick -size 300x400 gradient:'#ffffff'-'#203040' -colorspace sRGB -strip PNG24:ladder-page2.png
