# Multi-surface journeys

A journey drives one user path across several surfaces in one run and checks the state at each hand-off: which pane has the focus bar, what the right pane shows, and the breadcrumb (`workspace › <checkout>`; the focused right pane is in brackets, `workspace › [merger]`). Run the journeys last in a sweep ([README.md](README.md)): they pass only when the surfaces they cross pass on their own.

## MJ1 Tree → diff → graph (`Tab`) → commit files → back

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
$WS doctor
trap '$WS stop' EXIT
$WS wait-ready
$WS snap --name mj1-01-start --raw | grep -E '▌ +· session\.ts'

# Tree -> diff: select a dirty file, then focus its diff and come back.
$WS key j j j
$WS wait --cursor 'auth.ts'
$WS wait '\+const REFRESH_MS = 2 \* 60 \* 1000;'
$WS key Enter
$WS wait --cursor 'UNSTAGED'
$WS snap --name mj1-02-diff-focus --raw | grep -F 'app/src/auth.ts'
$WS key Escape
$WS wait --cursor 'auth.ts'

# Tree -> graph: jump to merger, Tab into its graph, walk to a commit.
$WS key /
$WS type merger
$WS key Enter
$WS wait --cursor 'v merger'
$WS wait 'merge billing into main'
$WS key Tab
$WS wait --cursor 'working tree clean'
$WS wait 'workspace › \[merger\]'
$WS key j
$WS wait --cursor 'stash@{0}'
$WS key j
$WS wait --cursor 'Start reconciliation job'
$WS snap --name mj1-03-graph --raw | grep -E '▌ @-/ +Start reconciliation job'

# Graph -> commit files.
$WS key Enter
$WS wait 'reconcile\.ts'
$WS wait '1 file in 49b9717'
$WS snap --name mj1-04-files --raw | grep -E '^ graph _+ files _+'

# Back: Esc clears the armed search, Esc returns to the graph, Esc to the tree.
$WS key Escape
$WS wait 'search cleared'
$WS key Escape
$WS wait 'Esc +↑ graph'
$WS key Escape
$WS wait '^ tree _+ graph _+'
$WS wait --cursor 'v merger'
$WS snap --name mj1-05-tree --raw | grep -E '^workspace › merger'

# Tab round trip from the tree.
$WS key Tab
$WS wait 'workspace › \[merger\]'
$WS key Tab
$WS wait --cursor 'v merger'
$WS doctor
```

Hand-offs and the text each one shows:

| Step | Surface | Screen |
| --- | --- | --- |
| start | tree | `▌ · session.ts`, the staged diff on the right |
| `j j j` | tree → diff of `auth.ts` | `+const REFRESH_MS = 2 * 60 * 1000;` |
| `Enter` / `Esc` | diff focus and back | bar on `UNSTAGED`, then on `auth.ts` |
| `/merger` `Enter` | tree on `merger`, graph on the right | `▌ v merger`, `merge billing into main` |
| `Tab`, `j`, `j` | graph focus, commit row | `workspace › [merger]`, `▌ @-/  Start reconciliation job` |
| `Enter` | commit files pane | title `graph ___ files ___`, `reconcile.ts`, `1 file in 49b9717` |
| `Esc` ×3 | search cleared, graph, tree | `search cleared`, `Esc   ↑ graph`, title `tree ___ graph ___`, `▌ v merger` |
| `Tab`, `Tab` | graph and back | `workspace › [merger]`, then `▌ v merger` |

**PASS:** every `wait` is ok, the five `grep` lines match their captures (`evidence/NN-mj1-0*.txt`), and the last `doctor` is ok (the TUI is alive, at its size, after the whole path). A FAIL names the first step whose `wait` failed and quotes its saved screen (`screen` in the failure JSON).
