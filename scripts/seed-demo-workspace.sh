#!/usr/bin/env bash
# Seed a multi-repo workspace for workspace-status screenshots and video.
# Re-runs wipe and recreate the output directory. Remotes stay local.
set -euo pipefail

usage() {
  cat <<'USAGE'
usage: seed-demo-workspace.sh [output-dir]

Default output dir is tmp/demo-workspace under the repository root.
The directory is wiped and recreated on every run.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi
if [[ $# -gt 1 ]]; then
  usage >&2
  exit 2
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

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

rm -rf "$DEST"
mkdir -p "$DEST/.remotes"

export GIT_AUTHOR_NAME="Demo User"
export GIT_AUTHOR_EMAIL="demo@example.com"
export GIT_COMMITTER_NAME="Demo User"
export GIT_COMMITTER_EMAIL="demo@example.com"
export GIT_CONFIG_COUNT=5
export GIT_CONFIG_KEY_0=user.name
export GIT_CONFIG_VALUE_0="Demo User"
export GIT_CONFIG_KEY_1=user.email
export GIT_CONFIG_VALUE_1="demo@example.com"
export GIT_CONFIG_KEY_2=commit.gpgsign
export GIT_CONFIG_VALUE_2=false
export GIT_CONFIG_KEY_3=init.defaultBranch
export GIT_CONFIG_VALUE_3=main
export GIT_CONFIG_KEY_4=advice.detachedHead
export GIT_CONFIG_VALUE_4=false

# Fixed stamp so every re-run writes the same history.
DEMO_DATE='2026-08-01T12:00:00 +0000'

git_init() {
  mkdir -p "$1"
  git init -q -b main "$1"
}

git_commit() {
  local dir="$1"
  local msg="$2"
  GIT_AUTHOR_DATE="$DEMO_DATE" GIT_COMMITTER_DATE="$DEMO_DATE" \
    git -C "$dir" commit -q -m "$msg"
}

git_add_commit() {
  local dir="$1"
  local msg="$2"
  shift 2
  git -C "$dir" add -- "$@"
  git_commit "$dir" "$msg"
}

attach_origin() {
  local dir="$1"
  local name="$2"
  local remote="$DEST/.remotes/${name}.git"
  git init -q --bare "$remote"
  git -C "$dir" remote add origin "file://${remote}"
  git -C "$dir" push -q -u origin main
}

put() {
  mkdir -p "$(dirname -- "$1")"
  printf '%s\n' "$2" >"$1"
}

# --- app ---
APP="$DEST/app"
git_init "$APP"
put "$APP/README.md" "# Shop checkout

Storefront for the demo shop. Talks to services/api."
put "$APP/.gitignore" ".worktrees/"
put "$APP/package.json" '{
  "name": "shop-app",
  "private": true
}'
put "$APP/src/app.ts" 'import { startCheckout } from "./checkout.ts";
startCheckout();'
put "$APP/src/cart.ts" "export function createCart(): string[] {
  return [];
}"
put "$APP/src/checkout.ts" 'export function startCheckout(): void {
  console.log("checkout");
}'
git_add_commit "$APP" "seed shop checkout app" README.md .gitignore package.json src/app.ts src/cart.ts src/checkout.ts
attach_origin "$APP" app

git -C "$APP" checkout -q -b feature/checkout
put "$APP/src/checkout.ts" 'export function startCheckout(): void {
  console.log("checkout");
  console.log("collect shipping");
}'
git_add_commit "$APP" "collect shipping address on checkout" src/checkout.ts
git -C "$APP" push -q -u origin feature/checkout
put "$APP/src/promo.ts" 'export function applyPromo(code: string): number {
  return code === "SHIPFREE" ? 0 : 5;
}'
git_add_commit "$APP" "add promo helper" src/promo.ts

git -C "$APP" worktree add -q "$APP/.worktrees/feat-login" -b feature/login
put "$APP/.worktrees/feat-login/src/login.ts" 'export function login(email: string): boolean {
  return email.includes("@");
}'
git_add_commit "$APP/.worktrees/feat-login" "add login form helper" src/login.ts
put "$APP/src/checkout.ts" 'export function startCheckout(): void {
  console.log("checkout");
  console.log("collect shipping");
  console.log("offer pickup");
}'
put "$APP/src/app.ts" 'import { startCheckout } from "./checkout.ts";
import { applyPromo } from "./promo.ts";
console.log(applyPromo("SHIPFREE"));
startCheckout();'
git -C "$APP" add src/app.ts
put "$APP/src/draft-banner.ts" 'export const draftBanner = "Free shipping this week";'

# --- services/api ---
API="$DEST/services/api"
git_init "$API"
put "$API/README.md" "# Shop API

HTTP API for catalog and orders."
put "$API/package.json" '{
  "name": "shop-api",
  "private": true
}'
put "$API/src/server.ts" 'import { createServer } from "node:http";
createServer(() => {}).listen(3000);'
put "$API/src/routes/health.ts" 'export function handleHealth(): string {
  return "ok";
}'
put "$API/src/routes/orders.ts" "export function listOrders(): unknown[] {
  return [];
}"
git_add_commit "$API" "seed shop API" README.md package.json src/server.ts src/routes/health.ts src/routes/orders.ts
attach_origin "$API" api

git -C "$API" checkout -q -b feature/orders
put "$API/src/routes/orders.ts" 'export function listOrders(): unknown[] {
  return [];
}
export function createOrder(): string {
  return "ord_1";
}'
git_add_commit "$API" "accept POST /orders" src/routes/orders.ts
git -C "$API" push -q -u origin feature/orders

scratch="$(mktemp -d)"
git clone -q --branch feature/orders "file://${DEST}/.remotes/api.git" "$scratch"
put "$scratch/src/routes/health.ts" 'export function handleHealth(): string {
  return "ok v1.1";
}'
git_add_commit "$scratch" "report API version on health" src/routes/health.ts
git -C "$scratch" push -q origin feature/orders
rm -rf "$scratch"

put "$API/src/routes/orders.ts" 'export function listOrders(): unknown[] {
  return [];
}
export function createOrder(): { id: string; status: string } {
  return { id: "ord_1", status: "created" };
}'
git_add_commit "$API" "return order status on create" src/routes/orders.ts
git -C "$API" fetch -q origin
put "$API/src/server.ts" 'import { createServer } from "node:http";
const port = Number(process.env.PORT ?? 3000);
createServer(() => {}).listen(port);'

# --- lib ---
LIB="$DEST/lib"
git_init "$LIB"
put "$LIB/README.md" "# Shop lib

Shared money and id helpers."
put "$LIB/package.json" '{
  "name": "shop-lib",
  "private": true
}'
put "$LIB/src/index.ts" 'export { formatMoney } from "./money.ts";
export { newId } from "./ids.ts";'
put "$LIB/src/money.ts" "export function formatMoney(cents: number): string {
  return (cents / 100).toFixed(2);
}"
put "$LIB/src/ids.ts" 'export function newId(prefix: string): string {
  return prefix + "_x";
}'
git_add_commit "$LIB" "seed shop lib" README.md package.json src/index.ts src/money.ts src/ids.ts
attach_origin "$LIB" lib

# --- notes ---
NOTES="$DEST/notes"
git_init "$NOTES"
put "$NOTES/README.md" "# Shop notes

Standup and release notes for the demo shop."
put "$NOTES/standup.md" "# Standup

- Checkout collects a shipping address.
- API accepts POST /orders."
put "$NOTES/release-checklist.md" "# Release checklist

- [ ] Cut changelog
- [ ] Tag lib
- [ ] Deploy API"
git_add_commit "$NOTES" "seed shop notes" README.md standup.md release-checklist.md
attach_origin "$NOTES" notes
printf '\n- Promo helper is local-only on app.\n' >>"$NOTES/standup.md"

# --- merger ---
MERGER="$DEST/merger"
git_init "$MERGER"
put "$MERGER/README.md" "# Shop release

Release cut and changelog for the demo shop."
put "$MERGER/changelog.md" "# Changelog

## Unreleased"
put "$MERGER/src/pipeline.ts" 'export function cutNotes(version: string): string {
  return "shop " + version;
}'
git_add_commit "$MERGER" "seed release pipeline" README.md changelog.md src/pipeline.ts
attach_origin "$MERGER" merger
put "$MERGER/src/pipeline.ts" 'export function cutNotes(version: string): string {
  return "shop " + version + "\n";
}'
git_add_commit "$MERGER" "append newline to cut notes" src/pipeline.ts
git -C "$MERGER" push -q origin main

git -C "$MERGER" checkout -q -b feature/billing
put "$MERGER/src/billing.ts" "export function invoiceTotal(cents: number): number {
  return cents;
}"
git_add_commit "$MERGER" "add billing invoice helper" src/billing.ts

git -C "$MERGER" checkout -q main
put "$MERGER/changelog.md" "# Changelog

## Unreleased

- Checkout shipping address"
git_add_commit "$MERGER" "note shipping address in changelog" changelog.md
git -C "$MERGER" push -q origin main

git -C "$MERGER" checkout -q -b feature/release-cut
GIT_AUTHOR_DATE="$DEMO_DATE" GIT_COMMITTER_DATE="$DEMO_DATE" \
  git -C "$MERGER" merge -q --no-ff feature/billing -m "merge billing into release cut"
put "$MERGER/src/rate-limit.ts" "export function allow(ip: string): boolean {
  return ip.length > 0;
}"
git -C "$MERGER" add src/rate-limit.ts
GIT_AUTHOR_DATE="$DEMO_DATE" GIT_COMMITTER_DATE="$DEMO_DATE" \
  git -C "$MERGER" stash push -q -u -m "wip: rate limit"

put "$DEST/.workspace-status-config.json" '{
  "ignoredRepos": ["notes"],
  "maxDepth": 3
}'
put "$DEST/README.md" "# Demo shop workspace

Multi-repo seed for workspace-status screenshots and video."

printf 'Seeded %s\n' "$DEST"
printf 'Run: cd %s && workspace-status   # TTY, or workspace-status --plain\n' "$DEST"
