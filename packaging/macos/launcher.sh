#!/bin/bash
# Launcher inside "Rat Run.app". Rat Run is a terminal game, so a double-click
# has to open a terminal window and run the real binary in it. Prefers Ghostty
# (the terminal the game is developed against), then iTerm2, then Terminal.
set -u
bin="$(cd "$(dirname "$0")/../Resources" && pwd)/ratrun"

if [ -d "/Applications/Ghostty.app" ] || [ -d "$HOME/Applications/Ghostty.app" ]; then
  exec open -na Ghostty --args -e "$bin"
fi

# The bundle is "Rat Run.app", so the binary's path always holds a space. It is
# handed to AppleScript as an argument and shell-quoted there (`quoted form
# of`), never pasted into the script text: iTerm2 splits an unquoted command on
# spaces, and a path with a quote in it would otherwise break the script.
if [ -d "/Applications/iTerm.app" ] || [ -d "$HOME/Applications/iTerm.app" ]; then
  exec osascript \
    -e 'on run argv' \
    -e 'set cmd to quoted form of (item 1 of argv)' \
    -e 'tell application "iTerm"' \
    -e 'activate' \
    -e 'create window with default profile command cmd' \
    -e 'end tell' \
    -e 'end run' \
    "$bin"
fi

exec osascript \
  -e 'on run argv' \
  -e 'set cmd to "exec " & quoted form of (item 1 of argv)' \
  -e 'tell application "Terminal"' \
  -e 'activate' \
  -e 'do script cmd' \
  -e 'end tell' \
  -e 'end run' \
  "$bin"
