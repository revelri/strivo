# strivo demo tape — render with `vhs docs/demo/demo.tape`
# https://github.com/charmbracelet/vhs
#
# Output lands at docs/demo/demo.gif and is referenced from README.md.

Output docs/demo/demo.gif

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 14
Set Width 1200
Set Height 720
Set Theme "Tokyo Night"
Set TypingSpeed 35ms
Set Padding 18

# --- title card ---
Type "# strivo — terminal DVR for live streams"
Sleep 600ms
Enter
Sleep 400ms

# --- doctor check ---
Type "strivo doctor"
Sleep 300ms
Enter
Sleep 1.4s

# --- launch the TUI ---
Type "strivo"
Sleep 300ms
Enter
Sleep 2s

# Sidebar nav: down through channels
Down
Sleep 600ms
Down
Sleep 600ms
Down
Sleep 800ms

# Toggle auto-record on the focused channel
Type "a"
Sleep 1.2s

# Open the recording browser
Right
Sleep 800ms
Down Down
Sleep 800ms

# Open the theme picker
Ctrl+t
Sleep 1.5s
Down Down Down
Sleep 800ms
Enter
Sleep 1.2s

# Quit cleanly
Type "q"
Sleep 600ms
