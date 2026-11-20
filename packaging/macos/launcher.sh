#!/bin/bash
# Launcher inside "Rat Run.app". Rat Run is a terminal game, so a double-click
# has to open a terminal window and run the real binary in it. Prefers Ghostty
# (the terminal the game is developed against), then iTerm2, then Terminal.
set -u
bin="$(cd "$(dirname "$0")/../Resources" && pwd)/ratrun"

if [ -d "/Applications/Ghostty.app" ] || [ -d "$HOME/Applications/Ghostty.app" ]; then
  exec open -na Ghostty --args -e "$bin"
fi

if [ -d "/Applications/iTerm.app" ]; then
  exec osascript \
    -e 'tell application "iTerm" to activate' \
    -e "tell application \"iTerm\" to create window with default profile command \"$bin\""
fi

exec osascript \
  -e 'tell application "Terminal" to activate' \
  -e "tell application \"Terminal\" to do script \"exec \\\"$bin\\\"\""
