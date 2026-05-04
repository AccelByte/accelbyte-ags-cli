# AGS CLI onboarding demo reel.
# Regenerate with ./demo/record.sh (see CONTRIBUTING.md).

Output demo/reel.gif

Set Theme "Dracula"
Set FontSize 14
Set Width 1200
Set Height 700
Set TypingSpeed 50ms
Set PlaybackSpeed 1.0

# Point the CLI at the local mock server for this recording only.
# Set off-camera so the reel stays focused on ags commands.
Env AGS_BASE_URL "http://localhost:8765"

# Isolate the recording from the user's real config and OS keychain so
# every run starts unauthenticated and the login step actually demonstrates
# a fresh authentication. record.sh is responsible for cleaning AGS_HOME.
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

# 3. Headless login (secret piped via stdin).
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

# 7. Same data, machine-readable — pipe into jq or anything else.
Type "ags iam roles list --format json"
Enter
Sleep 4s
