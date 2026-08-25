#!/usr/bin/env bash
# Seed a multi-repo workspace for workspace-status screenshots.
# Safe to re-run: wipes DEST (including DEST/.remotes and DEST/.scratch) then reseeds.
#
# Usage: seed-demo-workspace.sh [DEST]
# Default DEST is tmp/demo-workspace under the repository root.
#
# Launch after seeding:
#   cd DEST && WS_STATUS_WATCH_MS=0 WS_STATUS_FETCH_MS=0 workspace-status
#
# Commit timestamps are fixed in Asia/Manila (UTC+8) so the graph stays stable.

set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  cat <<'USAGE'
usage: seed-demo-workspace.sh [DEST]

Default DEST is tmp/demo-workspace under the repository root.
The directory is wiped and recreated on every run.
USAGE
  exit 0
fi
if [[ $# -gt 1 ]]; then
  echo "usage: seed-demo-workspace.sh [DEST]" >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [[ $# -eq 1 ]]; then
  DEST="$1"
  if [[ "$DEST" != /* ]]; then
    DEST="$(pwd)/$DEST"
  fi
else
  DEST="$REPO_ROOT/tmp/demo-workspace"
fi

case "$DEST" in
  /|"$HOME"|"$REPO_ROOT")
    echo "refusing to wipe $DEST" >&2
    exit 1
    ;;
esac

WORKSPACE="$DEST"
REMOTES="$DEST/.remotes"
SCRATCH="$DEST/.scratch"

rm -rf "$WORKSPACE"
mkdir -p "$WORKSPACE" "$REMOTES" "$SCRATCH"

export GIT_AUTHOR_NAME="Demo User"
export GIT_AUTHOR_EMAIL="demo@example.invalid"
export GIT_COMMITTER_NAME="Demo User"
export GIT_COMMITTER_EMAIL="demo@example.invalid"
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_NOSYSTEM=1

# 2026-08-17 09:00:00 +08, then +1h per commit
TS=1786928400
tick() {
  export GIT_AUTHOR_DATE="${TS} +0800"
  export GIT_COMMITTER_DATE="${TS} +0800"
  TS=$((TS + 3600))
}

git_in() {
  git -C "$1" "${@:2}"
}

init_repo() {
  local repo="$1"
  mkdir -p "$repo"
  if ! git init -q -b main "$repo" 2>/dev/null; then
    git init -q "$repo"
    git_in "$repo" checkout -q -b main
  fi
  git_in "$repo" config user.name "Demo User"
  git_in "$repo" config user.email "demo@example.invalid"
  git_in "$repo" config commit.gpgsign false
}

write_file() {
  local path="$1"
  mkdir -p "$(dirname "$path")"
  cat > "$path"
}

commit_all() {
  local repo="$1"
  local msg="$2"
  tick
  git_in "$repo" add -A
  git_in "$repo" commit -q -m "$msg"
}

add_origin() {
  local repo="$1"
  local name="$2"
  local bare="$REMOTES/${name}.git"
  if ! git init -q --bare -b main "$bare" 2>/dev/null; then
    git init -q --bare "$bare"
  fi
  git_in "$repo" remote add origin "$bare"
  git_in "$repo" push -q -u origin HEAD
}

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------
write_file "$WORKSPACE/.workspace-status-config.json" <<'EOF'
{
  "ignoredRepos": ["notes"],
  "editor": "vim"
}
EOF

# ---------------------------------------------------------------------------
# app — dirty feature branch, ahead of origin, mixed file states + worktree
# ---------------------------------------------------------------------------
APP="$WORKSPACE/app"
init_repo "$APP"

write_file "$APP/README.md" <<'EOF'
# app

Demo web client used by the workspace-status TUI screenshots.
EOF

write_file "$APP/package.json" <<'EOF'
{
  "name": "app",
  "private": true,
  "type": "module",
  "version": "0.1.0"
}
EOF

write_file "$APP/.gitignore" <<'EOF'
.worktrees/
EOF

write_file "$APP/src/config.ts" <<'EOF'
export const API_BASE = "/api";
export const SESSION_TTL_MS = 30 * 60 * 1000;
EOF

write_file "$APP/src/auth.ts" <<'EOF'
export type Session = {
  userId: string;
  token: string;
  expiresAt: number;
};

export function isExpired(session: Session, now = Date.now()): boolean {
  return now >= session.expiresAt;
}
EOF

write_file "$APP/src/session.ts" <<'EOF'
import { type Session, isExpired } from "./auth.ts";

export function requireSession(session: Session | null): Session {
  if (!session || isExpired(session)) {
    throw new Error("unauthorized");
  }
  return session;
}
EOF

commit_all "$APP" "seed app client"
add_origin "$APP" "app"

git_in "$APP" checkout -q -b feature/auth-refresh

write_file "$APP/src/auth.ts" <<'EOF'
export type Session = {
  userId: string;
  token: string;
  expiresAt: number;
};

const REFRESH_MS = 5 * 60 * 1000;

export function isExpired(session: Session, now = Date.now()): boolean {
  return now >= session.expiresAt;
}

export function needsRefresh(session: Session, now = Date.now()): boolean {
  return session.expiresAt - now < REFRESH_MS;
}
EOF

commit_all "$APP" "Add token refresh window"
git_in "$APP" push -q -u origin HEAD

# Local-only commit so the branch is ahead of origin.
write_file "$APP/src/session.ts" <<'EOF'
import { type Session, isExpired, needsRefresh } from "./auth.ts";

export function requireSession(session: Session | null): Session {
  if (!session || isExpired(session)) {
    throw new Error("session expired");
  }
  return session;
}

export function shouldRefresh(session: Session): boolean {
  return needsRefresh(session);
}
EOF

commit_all "$APP" "Tighten session expiry"

# Stash a WIP so app has stash@{0} (stash menu + graph spur).
write_file "$APP/src/tokenCache.ts" <<'EOF'
const cache = new Map<string, string>();

export function rememberToken(userId: string, token: string): void {
  cache.set(userId, token);
}
EOF
tick
git_in "$APP" add src/tokenCache.ts
git_in "$APP" stash push -q -u -m "WIP: in-memory token cache"

# Unstaged M
write_file "$APP/src/auth.ts" <<'EOF'
export type Session = {
  userId: string;
  token: string;
  expiresAt: number;
};

const REFRESH_MS = 2 * 60 * 1000;

export function isExpired(session: Session, now = Date.now()): boolean {
  return now >= session.expiresAt;
}

export function needsRefresh(session: Session, now = Date.now()): boolean {
  return session.expiresAt - now < REFRESH_MS;
}

export function withRefreshedExpiry(
  session: Session,
  ttlMs: number,
  now = Date.now(),
): Session {
  return { ...session, expiresAt: now + ttlMs };
}
EOF

# Staged M
write_file "$APP/src/session.ts" <<'EOF'
import { type Session, isExpired, needsRefresh } from "./auth.ts";

export function requireSession(session: Session | null): Session {
  if (!session || isExpired(session)) {
    throw new Error("session expired — sign in again");
  }
  return session;
}

export function shouldRefresh(session: Session): boolean {
  return needsRefresh(session);
}

export function touch(session: Session, ttlMs: number, now = Date.now()): Session {
  return { ...session, expiresAt: now + ttlMs };
}
EOF
git_in "$APP" add src/session.ts

# Untracked
write_file "$APP/src/login.ts" <<'EOF'
export type LoginForm = {
  email: string;
  password: string;
};

export function emptyLoginForm(): LoginForm {
  return { email: "", password: "" };
}
EOF

# Linked worktree on another branch (slightly dirty).
mkdir -p "$APP/.worktrees"
git_in "$APP" worktree add -q -b feature/login-page "$APP/.worktrees/feat-login" main
write_file "$APP/.worktrees/feat-login/src/LoginPage.tsx" <<'EOF'
export function LoginPage() {
  return "<form>email / password</form>";
}
EOF

# ---------------------------------------------------------------------------
# services/api — dirty feature branch, diverged from origin
# ---------------------------------------------------------------------------
API="$WORKSPACE/services/api"
init_repo "$API"

write_file "$API/README.md" <<'EOF'
# api

Demo HTTP service for workspace-status screenshots.
EOF

write_file "$API/src/server.ts" <<'EOF'
import { handleUsers } from "./routes/users.ts";

export function listen(port = 3000): void {
  console.log(`api listening on ${port}`);
  handleUsers;
}
EOF

write_file "$API/src/routes/users.ts" <<'EOF'
export function handleUsers(): string {
  return "ok";
}
EOF

commit_all "$API" "seed api service"
add_origin "$API" "services_api"

git_in "$API" checkout -q -b feature/rate-limit

write_file "$API/src/rateLimit.ts" <<'EOF'
const hits = new Map<string, number>();

export function allow(key: string, max = 60): boolean {
  const n = (hits.get(key) ?? 0) + 1;
  hits.set(key, n);
  return n <= max;
}
EOF

write_file "$API/src/server.ts" <<'EOF'
import { handleUsers } from "./routes/users.ts";
import { allow } from "./rateLimit.ts";

export function listen(port = 3000): void {
  if (!allow("listen")) {
    throw new Error("rate limited");
  }
  console.log(`api listening on ${port}`);
  handleUsers;
}
EOF

commit_all "$API" "Add per-route rate limiter"
git_in "$API" push -q -u origin HEAD

# Origin advances (local will be behind after fetch).
ORIGIN_API="$SCRATCH/api-origin"
git clone -q "$REMOTES/services_api.git" "$ORIGIN_API"
git_in "$ORIGIN_API" config user.name "Demo User"
git_in "$ORIGIN_API" config user.email "demo@example.invalid"
git_in "$ORIGIN_API" checkout -q feature/rate-limit
write_file "$ORIGIN_API/src/routes/users.ts" <<'EOF'
export function handleUsers(): string {
  return "ok";
}

export function handleUserById(id: string): { id: string } {
  return { id };
}
EOF
commit_all "$ORIGIN_API" "Add user-by-id route"
git_in "$ORIGIN_API" push -q origin HEAD

# Local commit on a different file → diverged after fetch.
write_file "$API/src/rateLimit.ts" <<'EOF'
const hits = new Map<string, number>();

export function allow(key: string, max = 30): boolean {
  const n = (hits.get(key) ?? 0) + 1;
  hits.set(key, n);
  return n <= max;
}

export function reset(key: string): void {
  hits.delete(key);
}
EOF
commit_all "$API" "Lower default rate limit"
git_in "$API" fetch -q origin

# Unstaged dirty
write_file "$API/src/server.ts" <<'EOF'
import { handleUsers } from "./routes/users.ts";
import { allow } from "./rateLimit.ts";

export function listen(port = 3000): void {
  if (!allow(`listen:${port}`)) {
    throw new Error("rate limited");
  }
  console.log(`api listening on ${port}`);
  handleUsers;
}
EOF

# ---------------------------------------------------------------------------
# lib — clean default-branch repo (folded under No updates)
# ---------------------------------------------------------------------------
LIB="$WORKSPACE/lib"
init_repo "$LIB"

write_file "$LIB/README.md" <<'EOF'
# lib

Shared helpers. Clean on main so it sits under No updates.
EOF

write_file "$LIB/src/index.ts" <<'EOF'
export function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n));
}
EOF

commit_all "$LIB" "seed lib"
add_origin "$LIB" "lib"

# ---------------------------------------------------------------------------
# notes — ignored dirty repo (`.` reveals it)
# ---------------------------------------------------------------------------
NOTES="$WORKSPACE/notes"
init_repo "$NOTES"

write_file "$NOTES/inbox.md" <<'EOF'
# Inbox

- screenshot the TUI tree
EOF
commit_all "$NOTES" "seed notes"
write_file "$NOTES/inbox.md" <<'EOF'
# Inbox

- screenshot the TUI tree
- capture git graph elbows + stash diamond
- capture help, search, stash menu
EOF

# ---------------------------------------------------------------------------
# merger — non-default branch, merge diamond + stash spur
# ---------------------------------------------------------------------------
MERGER="$WORKSPACE/merger"
init_repo "$MERGER"

write_file "$MERGER/README.md" <<'EOF'
# merger

Billing + invoices history used to screenshot the multi-lane graph.
EOF

write_file "$MERGER/.gitignore" <<'EOF'
.worktrees/
EOF

write_file "$MERGER/src/core.ts" <<'EOF'
export type Money = { cents: number; currency: "USD" };

export function zero(): Money {
  return { cents: 0, currency: "USD" };
}
EOF
commit_all "$MERGER" "root"

git_in "$MERGER" checkout -q -b feature/billing
write_file "$MERGER/src/billing.ts" <<'EOF'
import { type Money } from "./core.ts";

export function charge(amount: Money): Money {
  return amount;
}
EOF
commit_all "$MERGER" "left: add billing"

git_in "$MERGER" checkout -q main
write_file "$MERGER/src/invoices.ts" <<'EOF'
import { type Money, zero } from "./core.ts";

export function emptyInvoice(): { total: Money; lines: never[] } {
  return { total: zero(), lines: [] };
}
EOF
commit_all "$MERGER" "right: add invoices"

tick
git_in "$MERGER" merge --no-ff -m "merge billing into main" feature/billing

git_in "$MERGER" checkout -q -b feature/reconciliation
write_file "$MERGER/src/reconcile.ts" <<'EOF'
import { type Money } from "./core.ts";

export function balanced(left: Money, right: Money): boolean {
  return left.cents === right.cents && left.currency === right.currency;
}
EOF
commit_all "$MERGER" "Start reconciliation job"

write_file "$MERGER/src/wip-totals.ts" <<'EOF'
import { type Money } from "./core.ts";

export function sum(items: Money[]): number {
  return items.reduce((n, m) => n + m.cents, 0);
}
EOF
tick
git_in "$MERGER" add src/wip-totals.ts
git_in "$MERGER" stash push -q -u -m "WIP: reconcile totals"

add_origin "$MERGER" "merger"
# Stay on the feature branch (non-default) so the repo stays visible.
git_in "$MERGER" push -q -u origin feature/reconciliation
git_in "$MERGER" checkout -q feature/reconciliation

# Linked extra on the current branch (same HEAD as the primary). `--force`
# is required because git refuses a second checkout of a live branch.
mkdir -p "$MERGER/.worktrees"
git_in "$MERGER" worktree add -f -q "$MERGER/.worktrees/recon" feature/reconciliation

echo "seeded $WORKSPACE"
echo "cd $WORKSPACE && WS_STATUS_WATCH_MS=0 WS_STATUS_FETCH_MS=0 workspace-status"
