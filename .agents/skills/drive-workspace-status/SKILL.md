---
name: drive-workspace-status
description: Drive the workspace-status TUI like a user from tmux: start an isolated run, send keys, read the screen, stop. Use when an agent must operate or inspect `ws` interactively.
---

# Drive workspace-status

Use the controller of the verify skill: [`../verify-workspace-status/scripts/wsctl`](../verify-workspace-status/scripts/wsctl). It runs the `workspace-status` of this checkout in a private tmux run on the demo fixture and prints one JSON object per command. `wsctl --help` lists the commands; `wsctl <command> --help` gives the options.

## Loop

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl   # from the repository root
$WS start                      # build, seed, launch; prints {"ok":true,"run":...}
$WS doctor                     # stop here when a check fails
$WS key j                      # one key
$WS wait --cursor 'Changes'    # the state that key should produce
$WS snap --name after-j        # evidence file + "screen" text
$WS stop                       # always, also after a failure
```

`type <text>` types literal text, `key --chord g p` sends a two-key chord, `wait-ready` waits until the TUI is mounted, `resize 100x40` changes the terminal size, and `run -- --json` reads the headless snapshot of the same fixture.

## Rules

- **One key per call.** `key j j j` is three writes; a chord needs `--chord`.
- **Wait, not sleep.** Follow every key with `wait` on the text it should produce. `wait --cursor` takes plain text on the focus-bar line.
- **Isolated run only.** Drive only a run that `wsctl start` made. Never attach to, signal or type into another `ws`.
- **Never start the binary yourself.** Outside `wsctl`, run `ws` / `workspace-status` only with `--plain` or `--json`: a TTY run waits for keys forever.

Isolation, the proof bar, the report format and the feature recipes are in the verify skill: [../verify-workspace-status/SKILL.md](../verify-workspace-status/SKILL.md).
