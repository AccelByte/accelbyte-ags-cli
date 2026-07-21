# `--ui auto` choosing the surface by how many required inputs are still missing,
# shown on one command: ags iam roles add-permissions.
# Regenerate with ./demos/record.sh auto-surfaces (see CONTRIBUTING.md).
#
# Same command three times, no --ui flag (auto). What changes is how much is
# passed as flags:
#   1. role-id + --json body  → 0 missing → runs unattended (no prompts)
#   2. --json body only       → 1 missing scalar (role-id) → plain line prompt
#   3. role-id only           → body missing → inline form → JSON editor
# All three keystroke sequences are pty-verified to reach the result. Auth is
# off-camera via demos/auto-surfaces/prelogin, so no login appears here.

Output demos/auto-surfaces/auto-surfaces.gif

Set Theme "Dracula"
Set FontSize 14
Set Width 1200
Set Height 760
Set TypingSpeed 55ms
Set PlaybackSpeed 1.0

Env AGS_BASE_URL "http://localhost:8765"
Env AGS_HOME "/tmp/ags-demo-state"
Env AGS_NO_KEYCHAIN "1"

# ── 1. Auto → unattended: every input supplied, nothing to prompt ──
Type "# 1. Auto picks unattended: role-id + --json body, 0 missing"
Enter
Enter
Sleep 1s
Type `ags iam roles add-permissions --role-id b3f1c2d4-5e6a-4789-9abc-1d2e3f4a5b6c --json '{"permissions":[{"action":2,"resource":"NAMESPACE:demo:USER"}]}'`
Enter
Sleep 5s
Ctrl+L

# ── 2. Auto → plain: body supplied, role-id missing (1 scalar) ──
Type "# 2. Auto picks plain: --json body only, role-id missing → 1 prompt"
Enter
Enter
Sleep 1s
Type `ags iam roles add-permissions --json '{"permissions":[{"action":2,"resource":"NAMESPACE:demo:USER"}]}'`
Enter
Sleep 2s
Type "b3f1c2d4-5e6a-4789-9abc-1d2e3f4a5b6c"
Enter
Sleep 5s
Ctrl+L

# ── 3. Auto → inline: role-id supplied, body missing → form + JSON editor ──
# pty-verified. Tab to the permissions field, Enter opens the structured JSON
# editor. Build the first entry in the STRUCTURED (tree) view — add an entry,
# expand it, edit action + resource — then Ctrl+R to the RAW JSON view and add a
# second entry inline. Ctrl+S saves the editor, Ctrl+S submits → confirm. Shows
# both editor modes working.
Type "# 3. Auto picks inline: role-id only, body missing → form + JSON editor"
Enter
Enter
Sleep 1s
Type "ags iam roles add-permissions --role-id b3f1c2d4-5e6a-4789-9abc-1d2e3f4a5b6c"
Enter
Sleep 3s
# Form opens on role-id (prefilled); Tab to the permissions field.
Tab
Sleep 900ms
# Open the structured JSON editor.
Enter
Sleep 1500ms
# Structured mode: add an array entry (+), move onto it, expand it.
Type "+"
Sleep 800ms
Down
Right
Sleep 1s
# Down to "action", Enter to edit, type the value, Enter to commit.
Down
Sleep 600ms
Enter
Type "2"
Enter
Sleep 800ms
# Down to "resource", edit it the same way.
Down
Sleep 600ms
Enter
Type "NAMESPACE:demo:USER"
Enter
Sleep 1500ms
# Switch to the RAW JSON view and add a second entry, indented to match the
# first. The seed's line 1 is just "[", so Right moves past it; open a new line
# and type the object with the same 2-space / 4-space indentation serde uses.
Ctrl+R
Sleep 1500ms
Right
Enter
Tab
Type "{"
Enter
Sleep 300ms
Tab
Tab
Type `"action": 4,`
Enter
Sleep 300ms
Tab
Tab
Type `"resource": "NAMESPACE:demo:STAT"`
Enter
Sleep 300ms
Tab
Type "},"
Sleep 1500ms
# Save the editor (back to the form).
Ctrl+S
Sleep 1500ms
# Submit the form → Review card → confirm.
Ctrl+S
Sleep 1500ms
Enter
Sleep 4s
# Dismiss the result screen.
Enter
Sleep 1s
