# competitive-multiplayer workflow across all four input surfaces.
# Regenerate with ./demos/record.sh workflow-surfaces (see CONTRIBUTING.md).
#
# STALE — do not record until re-authored. The workflow gained a native
# upload step (CLI-02), which invalidates this tape three ways: --fleet-image-id
# no longer exists, the gather prompt order below gained Build Path / Build
# Executable / Target Architecture, and the upload step needs AMS routes that
# workflow-surfaces.routes.json does not stub plus a build directory on disk.
# Re-choreographing the keystroke sequences needs a pty verification pass, so
# it is deliberately left for its own change rather than guessed at here.
#
# All four keystroke sequences are pty-verified to reach "completed" (see the
# SKILL.md "Verifying interactive sections" note). Auth happens off-camera:
# demos/workflow-surfaces/prelogin tells record.sh to log in before recording,
# so no login appears here. Sleeps are tuned for human reading — the per-step
# review pauses long enough to register each step.

Output demos/workflow-surfaces/workflow-surfaces.gif

# Larger than the onboarding reel on purpose: the inline surface renders in a
# fixed-height viewport, so on a small terminal it fills the screen and looks
# almost identical to fullscreen. A taller terminal makes the inline viewport
# occupy a clearly smaller fraction. Adjust Width/Height/FontSize if inline and
# fullscreen still look alike.
Set Theme "Dracula"
Set FontSize 14
Set Width 1400
Set Height 1000
Set TypingSpeed 55ms
Set PlaybackSpeed 1.0

Env AGS_BASE_URL "http://localhost:8765"
Env AGS_HOME "/tmp/ags-demo-state"
Env AGS_NO_KEYCHAIN "1"

# ── 1. Automation (no input) — every value as a flag, machine-readable JSON ──
Type "# 1. Automation: all inputs as flags, --format json, no prompts"
Enter
Enter
Sleep 1s
Type "ags workflow run competitive-multiplayer --namespace dev --fleet-image-id b3f1c2d4-5e6a-4789-9abc-1d2e3f4a5b6c --fleet-region us-east-1 --fleet-instance-id 7d9e1f2a-3b4c-4d5e-8f60-9a1b2c3d4e5f --yes --format json"
Enter
Sleep 7s
Ctrl+L

# ── 2. Plain (--ui plain) — line-oriented prompts ──
# pty-verified prompt order: Namespace, Stat Code [mmr], Players Per Team [4],
# Team Count [2], Resource Prefix [ranked], Fleet Region, Fleet Image Id, Fleet
# Instance Id — then the 6 steps run. No briefing, no per-step confirm.
Type "# 2. Plain: one input per line"
Enter
Enter
Sleep 1s
Type "ags workflow run competitive-multiplayer --ui plain"
Enter
Sleep 2s
Type "dev"
Enter
Sleep 900ms
Enter
Sleep 800ms
Enter
Sleep 800ms
Enter
Sleep 800ms
Enter
Sleep 900ms
Type "us-east-1"
Enter
Sleep 900ms
Type "b3f1c2d4-5e6a-4789-9abc-1d2e3f4a5b6c"
Enter
Sleep 900ms
Type "7d9e1f2a-3b4c-4d5e-8f60-9a1b2c3d4e5f"
Enter
Sleep 6s
Ctrl+L

# ── 3. Inline (--ui inline) — in-cursor viewport form ──
# Keymap (pty-verified): Enter begins editing the focused field / commits it;
# Tab = next field; Ctrl+S = submit. fleet-* fields are PLAIN TEXT here (inline
# has no picker). Field order matches §2.
Type "# 3. Inline: briefing, then in-viewport form"
Enter
Enter
Sleep 1s
Type "ags workflow run competitive-multiplayer --ui inline"
Enter
Sleep 3s
# Briefing screen first ([Enter] continue).
Enter
Sleep 2s
# Form: Enter to edit, type, Enter to commit; Tab to the next field.
Enter
Type "dev"
Enter
Sleep 1s
Tab 5
Sleep 1s
Enter
Type "us-east-1"
Enter
Sleep 1s
Tab
Sleep 700ms
Enter
Type "b3f1c2d4-5e6a-4789-9abc-1d2e3f4a5b6c"
Enter
Sleep 1s
Tab
Sleep 700ms
Enter
Type "7d9e1f2a-3b4c-4d5e-8f60-9a1b2c3d4e5f"
Enter
Sleep 1s
Ctrl+S
Sleep 2s
# Per-step review — pause on each of the 6 steps long enough to read it.
Enter
Sleep 2800ms
Enter
Sleep 2800ms
Enter
Sleep 2800ms
Enter
Sleep 2800ms
Enter
Sleep 2800ms
Enter
Sleep 4s
# Dismiss the result screen.
Enter
Sleep 1s
Ctrl+L

# ── 4. Fullscreen (--ui fullscreen) — alt-screen form with live pickers ──
# Same form keymap as §3; the three fleet-* fields are DYNAMIC-ENUM PICKERS:
# Enter opens the picker (options fetched live from the mocked AMS endpoints),
# type to filter, Enter to select. pty-verified end to end.
Type "# 4. Fullscreen: briefing, then alt-screen form with live pickers"
Enter
Enter
Sleep 1s
Type "ags workflow run competitive-multiplayer --ui fullscreen"
Enter
Sleep 3s
# Briefing → continue
Enter
Sleep 2s
# Namespace (scalar): edit / type / commit
Enter
Type "dev"
Enter
Sleep 1s
# Tab past the four defaulted fields to Fleet Region (picker)
Tab 5
Sleep 1s
Enter
Sleep 1500ms
Type "us-east-1"
Sleep 1s
Enter
Sleep 1s
# Fleet Image (picker): filter "shooter" → "shooter-server v1.4.2"
Tab
Sleep 700ms
Enter
Sleep 1500ms
Type "shooter"
Sleep 1s
Enter
Sleep 1s
# Fleet Instance (picker): filter "C5 Large"
Tab
Sleep 700ms
Enter
Sleep 1500ms
Type "C5 Large"
Sleep 1s
Enter
Sleep 1s
Ctrl+S
Sleep 2s
# Per-step review — pause on each of the 6 steps long enough to read it.
Enter
Sleep 2800ms
Enter
Sleep 2800ms
Enter
Sleep 2800ms
Enter
Sleep 2800ms
Enter
Sleep 2800ms
Enter
Sleep 4s
# Dismiss the result screen.
Enter
Sleep 2s
