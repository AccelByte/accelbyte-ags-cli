# `social stat-definitions create` across all four input surfaces.
# Regenerate with ./demos/record.sh command-surfaces (see CONTRIBUTING.md).
#
# The three interactive keystroke sequences are pty-verified to reach the
# created result (see the SKILL.md "Verifying interactive sections" note). Auth
# happens off-camera: demos/command-surfaces/prelogin tells record.sh to log in
# before recording, so no login appears here. Sleeps are tuned for human reading.

Output demos/command-surfaces/command-surfaces.gif

# A single command's output is short, so a smaller viewport than the
# workflow-surfaces reel keeps dead space down while inline (a fixed-height
# scrollback viewport) still reads as smaller than the fullscreen alt-screen.
Set Theme "Dracula"
Set FontSize 14
Set Width 1200
Set Height 760
Set TypingSpeed 55ms
Set PlaybackSpeed 1.0

Env AGS_BASE_URL "http://localhost:8765"
Env AGS_HOME "/tmp/ags-demo-state"
Env AGS_NO_KEYCHAIN "1"

# ── 1. Automation (no input) — body as --json, machine-readable JSON ──
Type "# 1. Automation: --json body, --format json, no prompts"
Enter
Enter
Sleep 1s
Type `ags social stat-definitions create --namespace dev --json '{"defaultValue":1,"name":"Player Level","setBy":"SERVER","statCode":"player-level"}' --yes --format json`
Enter
Sleep 6s
Ctrl+L

# ── 2. Plain (--ui plain) — line-oriented prompts ──
# pty-verified prompt order: Namespace, Default Value, Name, Set By, Stat Code.
# No confirm prompt — create is not a confirm-gated operation.
Type "# 2. Plain: one input per line"
Enter
Enter
Sleep 1s
Type "ags social stat-definitions create --ui plain"
Enter
Sleep 2s
Type "dev"
Enter
Sleep 900ms
Type "1"
Enter
Sleep 900ms
Type "Player Level"
Enter
Sleep 900ms
Type "SERVER"
Enter
Sleep 900ms
Type "player-level"
Enter
Sleep 5s
Ctrl+L

# ── 3. Inline (--ui inline) — in-cursor viewport form ──
# Keymap (pty-verified): Enter edits / commits the focused field; Tab = next
# field; Space cycles an enum; Ctrl+S submits → a Review card ([Enter] confirm).
Type "# 3. Inline: in-viewport form"
Enter
Enter
Sleep 1s
Type "ags social stat-definitions create --ui inline"
Enter
Sleep 3s
# namespace: edit / type / commit
Enter
Type "dev"
Enter
Sleep 1s
# default-value
Tab
Sleep 700ms
Enter
Type "1"
Enter
Sleep 1s
# name
Tab
Sleep 700ms
Enter
Type "Player Level"
Enter
Sleep 1s
# set-by (enum): Space cycles CLIENT → SERVER
Tab
Sleep 700ms
Space
Sleep 400ms
Space
Sleep 1s
# stat-code
Tab
Sleep 700ms
Enter
Type "player-level"
Enter
Sleep 1s
# Submit → Review card → confirm
Ctrl+S
Sleep 2s
Enter
Sleep 3s
# Dismiss the result screen.
Enter
Sleep 1s
Ctrl+L

# ── 4. Fullscreen (--ui fullscreen) — alt-screen form ──
# Same keymap as §3. The set-by enum cycles with Space — single commands have no
# dynamic-enum picker (that is a workflow-only feature).
Type "# 4. Fullscreen: alt-screen form"
Enter
Enter
Sleep 1s
Type "ags social stat-definitions create --ui fullscreen"
Enter
Sleep 3s
# namespace
Enter
Type "dev"
Enter
Sleep 1s
# default-value
Tab
Sleep 700ms
Enter
Type "1"
Enter
Sleep 1s
# name
Tab
Sleep 700ms
Enter
Type "Player Level"
Enter
Sleep 1s
# set-by (enum): Space cycles CLIENT → SERVER
Tab
Sleep 700ms
Space
Sleep 400ms
Space
Sleep 1s
# stat-code
Tab
Sleep 700ms
Enter
Type "player-level"
Enter
Sleep 1s
# Submit → Review → confirm
Ctrl+S
Sleep 2s
Enter
Sleep 3s
# Dismiss the result screen.
Enter
Sleep 2s
