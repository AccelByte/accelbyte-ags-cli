# AGS CLI onboarding demo reel.
# Regenerate with ./demos/record.sh onboarding (see CONTRIBUTING.md).

Output demos/onboarding/onboarding.gif

Set Theme "Dracula"
Set FontSize 14
Set Width 1200
Set Height 700
Set TypingSpeed 50ms
Set PlaybackSpeed 1.0

# Point the CLI at the local mock server for this recording only, and isolate
# from the user's real config + OS keychain so login demonstrates a fresh auth.
Env AGS_BASE_URL "http://localhost:8765"
Env AGS_HOME "/tmp/ags-demo-state"
Env AGS_NO_KEYCHAIN "1"

# 1. Sanity check.
Type "ags --version"
Enter
Sleep 1500ms
Ctrl+L

# 2. Drill into IAM — shows the dynamic resource list.
Type "ags iam --help"
Enter
Sleep 3s
Ctrl+L

# 3. Headless login (secret piped via stdin) — auth is part of the onboarding story.
Type "ags auth login --grant client-credentials --client-id demo --client-secret-stdin <<< 'demo-secret'"
Enter
Sleep 3s
Ctrl+L

# 4. Confirm the session.
Type "ags auth status"
Enter
Sleep 3s
Ctrl+L

# 5. Inspect the resolved request.
Type "ags iam roles list --dry-run"
Enter
Sleep 3s
Ctrl+L

# 6. Run for real against the mock server; styled table for humans.
Type "ags iam roles list"
Enter
Sleep 4s
Ctrl+L

# 7. Same data, machine-readable.
Type "ags iam roles list --format json"
Enter
Sleep 4s
