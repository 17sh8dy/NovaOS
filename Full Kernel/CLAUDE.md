# Nova Full Kernel: instructions for Claude

You have no memory of earlier sessions. Before doing anything, read, in this order:

1. `docs/START-HERE.md` (current state, build/test commands, rules, traps)
2. `LOCAL-NOTES.md` in this folder (machine-specific: signing machine, USB stick, PC details; git-ignored,
   so it is not in the repo; if it is missing, ask the user)
3. `docs/REAL-HARDWARE.md` (the real-PC plan and the safety net)

Hard rules: Secure Boot stays ON; Nova never writes to a disk or firmware setting; confirm before writing or
formatting any USB drive, pushing, or deploying; push only branch `full-kernel`; this repo is PUBLIC, so never
put addresses, usernames, keys or recovery digits in tracked files.
