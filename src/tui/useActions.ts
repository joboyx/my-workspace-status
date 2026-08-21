/**
 * Action dispatch, confirmation flow, and git write operations.
 *
 * Split from useAppState so that state (what is on screen) and behaviour
 * (what keys do) can be read and tested separately.
 */

import { useCallback, useRef, useState } from 'react';
import type { MutableRefObject } from 'react';
import path from 'node:path';
import {
  removeUntrackedFile,
  removeWorktree,
  revertTrackedFile,
  stageFile,
  unstageFile,
} from '../git.js';
import type { RepoSnapshot } from '../types.js';
import { isDefaultBranch } from '../helpers.js';
import {
  canDefaultBranch,
  canPull,
  canPush,
  canRemoveWorktree,
  isRevertible,
} from './actions/gates.js';
import {
  formatPullStatus,
  formatPushStatus,
  formatSwitchStatus,
  tuiPullRepos,
  tuiPushRepos,
  tuiSwitchReposToDefault,
} from './gitActions.js';
import type { Action } from './keys.js';
import { startEdit } from './editorLaunch.js';
import { resolveEditor } from './editor.js';
import type { EditRequest } from './session.js';
import type { CheckoutNode, FileNode, RepoNode, VisibleRow } from './model/types.js';
import { branchPickerPath } from './branches.js';
import { checkoutNodeId } from './model/tree.js';
import type { ActionOpKind, ActionOpProgress } from './opStatus.js';
import { collectBulkGitTargets, collectFiles } from './scope.js';
import { repoNodeId } from './watch.js';

export { isRevertible } from './actions/gates.js';

/**
 * One path in a pending revert confirm — serialisable (no tree node refs).
 */
export type RevertTarget = {
  path: string;
  untracked: boolean;
  renameFrom?: string;
};

/** Counted revert confirm (`x`). */
export type PendingRevertConfirm = {
  kind: 'revert';
  repo: string;
  /** Display label (file path, dir path, or repo path). */
  label: string;
  targets: RevertTarget[];
  trackedCount: number;
  untrackedCount: number;
};

/** Linked worktree remove confirm (`W`). */
export type PendingRemoveWorktreeConfirm = {
  kind: 'removeWorktree';
  path: string;
  branch: string;
  primaryRepo: string;
  mergedIntoDefault: boolean | null;
  dirty: boolean;
  force: boolean;
};

/**
 * Confirmation currently awaiting y / Y / n, or null when no confirmation is open.
 */
export type PendingConfirm = PendingRevertConfirm | PendingRemoveWorktreeConfirm | null;

type GitOpResult = { ok: boolean; error?: string };

/**
 * Paths a git write must be applied to for one file node.
 *
 * A rename has to carry both endpoints: git needs the old path to record the
 * deletion and the new path to record the addition, so staging or reverting
 * only one of them leaves the rename half-applied. Everything else is a single
 * path.
 */
export function opPaths(file: FileNode): string[] {
  const renameFrom = file.renameFrom ?? file.change.oldPath;
  if (renameFrom && renameFrom !== file.path) {
    return [renameFrom, file.path];
  }
  return [file.path];
}

/**
 * Serialise a file node into a revert target for the confirm payload.
 */
export function toRevertTarget(file: FileNode): RevertTarget {
  const renameFrom = file.renameFrom ?? file.change.oldPath;
  const target: RevertTarget = {
    path: file.path,
    untracked: file.untracked,
  };
  if (renameFrom && renameFrom !== file.path) {
    target.renameFrom = renameFrom;
  }
  return target;
}

/**
 * Build the confirm payload for the focused row and revertible files under it.
 */
export function buildPendingConfirm(focused: VisibleRow, files: FileNode[]): PendingRevertConfirm {
  const node = focused.node;
  const repo =
    node.kind === 'repo' || node.kind === 'checkout'
      ? node.path
      : node.kind === 'file' || node.kind === 'dir'
        ? node.repoPath
        : (files[0]?.repoPath ?? '');
  const label = node.kind === 'workspace' || node.kind === 'group' ? repo : node.path;
  const targets = files.map(toRevertTarget);
  return {
    kind: 'revert',
    repo,
    label,
    targets,
    trackedCount: targets.filter((t) => !t.untracked).length,
    untrackedCount: targets.filter((t) => t.untracked).length,
  };
}

/**
 * Build the remove-worktree confirm for a linked checkout or flat linked repo.
 */
export function buildRemoveWorktreeConfirm(
  node: CheckoutNode | RepoNode,
): PendingRemoveWorktreeConfirm {
  const dirty = node.changeCount > 0;
  return {
    kind: 'removeWorktree',
    path: node.path,
    branch: node.branch,
    primaryRepo: node.primaryRepo ?? '',
    mergedIntoDefault: node.mergedIntoDefault,
    dirty,
    force: dirty,
  };
}

/**
 * Paths to re-read after a successful linked worktree remove.
 * Always includes the linked path (dropped when missing). Refreshes the
 * primary only when it was already in the current snapshots list — never
 * invent/append a primary that a named filter had excluded.
 */
export function refreshPathsAfterRemoveWorktree(
  linkedPath: string,
  primaryRepo: string,
  snapshotRepos: readonly string[],
): string[] {
  const paths = [linkedPath];
  if (primaryRepo && snapshotRepos.includes(primaryRepo)) {
    paths.push(primaryRepo);
  }
  return paths;
}

/**
 * Whether confirm should delete untracked targets.
 *
 * `Y` always deletes. Plain `y` deletes only when the sole target is untracked
 * (preserving single-file untracked meaning — the only revert is delete).
 */
export function shouldDeleteUntracked(targets: RevertTarget[], clean: boolean): boolean {
  if (clean) return true;
  return targets.length === 1 && targets[0].untracked;
}

/**
 * Paths a revert must restore for one serialised target (rename-aware).
 */
function targetPaths(target: RevertTarget): string[] {
  if (target.renameFrom && target.renameFrom !== target.path) {
    return [target.renameFrom, target.path];
  }
  return [target.path];
}

/**
 * Status-bar line after a successful bulk/single revert.
 */
export function formatRevertStatus(
  pending: PendingRevertConfirm,
  deletedUntracked: boolean,
): string {
  const tracked = pending.trackedCount;
  const untracked = pending.untrackedCount;
  if (pending.targets.length === 1) {
    const t = pending.targets[0];
    if (t.untracked && deletedUntracked) return `Deleted ${t.path}`;
    if (!t.untracked) return `Reverted ${t.path}`;
  }
  const parts: string[] = [];
  if (tracked > 0) {
    parts.push(tracked === 1 ? 'Reverted 1 file' : `Reverted ${tracked} files`);
  }
  if (deletedUntracked && untracked > 0) {
    parts.push(untracked === 1 ? 'deleted 1 untracked' : `deleted ${untracked} untracked`);
  } else if (!deletedUntracked && untracked > 0 && tracked === 0) {
    return untracked === 1 ? 'Kept 1 untracked' : `Kept ${untracked} untracked`;
  }
  return parts.length > 0 ? parts.join(', ') : 'Reverted';
}

async function runOnPaths(
  paths: string[],
  op: (filePath: string) => Promise<GitOpResult>,
): Promise<GitOpResult> {
  for (const p of paths) {
    const result = await op(p);
    if (!result.ok) return result;
  }
  return { ok: true };
}

/**
 * Everything the action layer needs from the state layer, passed explicitly
 * rather than reached for through module state.
 */
export interface ActionDeps {
  cwd: string;
  /** Row under the cursor; write actions operate on it. */
  focused: VisibleRow | null;
  /** Re-reads one repo after a write and reports the given success message. */
  refreshRepoOnly: (repo: string, message: string) => Promise<void>;
  /** Opens or closes the confirm keymap, which the state layer owns. */
  setConfirmMode: (on: boolean) => void;
  /** Shared gate so refresh and write ops cannot overlap. */
  busyRef: MutableRefObject<boolean>;
  /**
   * Records which file the user asked to edit. Used only for blocking TTY
   * editors: `dispatch` returns `'quit'` so the mount ends in the same
   * keypress, and `run.ts` opens `$EDITOR` from `pendingEdit` after unmount.
   * GUI editors spawn detached and never call this.
   */
  onEditRequest: (request: EditRequest) => void;
  /**
   * Workspace config `editor` string. Resolved with `$EDITOR` / `$VISUAL`
   * at dispatch time so a GUI vs TTY choice matches `run.ts`.
   */
  editor?: string;
  /** Snapshot repo paths relative to cwd (background fetch). Keyboard `f` / `p` / `P` / `d` use `snapshots` via `collectBulkGitTargets`. */
  repoPaths: readonly string[];
  /** Live snapshots — sync status / branch for `p` and `d` scoping. */
  snapshots: readonly RepoSnapshot[];
  /** Workspace ignore list. Hidden ignored paths are skipped on bulk git ops. */
  ignoredRepos: ReadonlySet<string>;
  /** Session `.` / `-a` flag. When true, ignored repos behave like visible rows. */
  showIgnored: boolean;
  /** Shared fetch runner from `useFetch` (manual `f`). */
  runFetch: (repos: readonly string[], opts?: { manual?: boolean }) => void;
  /**
   * Re-reads the given repos after a multi-repo write (pull / default-branch)
   * and applies them in one tree rebuild.
   */
  refreshRepos: (repos: readonly string[]) => Promise<void>;
  /**
   * Opens the local branch picker for a repo path (relative to cwd).
   * Owned by the state layer (branchMode + picker React state).
   */
  openBranchPicker: (repoPath: string) => void;
  /** Stamp tree node ids into the shared flash map (B9 op-row flash). */
  flashNodes: (ids: string[]) => void;
}

/**
 * Public surface of the action layer.
 */
export interface ActionsApi {
  /**
   * Runs one action; unknown action types are left to the state layer.
   * Returns `'quit'` when a blocking TTY edit must unmount in the same keypress.
   */
  dispatch: (action: Action) => 'quit' | void;
  pendingConfirm: PendingConfirm;
  statusMessage: string;
  setStatusMessage: (msg: string) => void;
  /** In-progress pull / push / default-branch for the top-chrome op-status slot. */
  actionOp: ActionOpKind | null;
  /** Settled/total repos for {@link actionOp} (same shape as fetch progress). */
  actionOpProgress: ActionOpProgress | null;
}

/**
 * Own the write actions (stage / unstage / revert), the confirmation flow, and
 * the status message they report through.
 */
export function useActions(deps: ActionDeps): ActionsApi {
  const {
    cwd,
    focused,
    refreshRepoOnly,
    setConfirmMode,
    busyRef,
    onEditRequest,
    editor: editorConfig,
    snapshots,
    ignoredRepos,
    showIgnored,
    runFetch,
    refreshRepos,
    openBranchPicker,
    flashNodes,
  } = deps;

  const [statusMessage, setStatusMessage] = useState('');
  const [pendingConfirm, setPendingConfirm] = useState<PendingConfirm>(null);
  const [actionOp, setActionOp] = useState<ActionOpKind | null>(null);
  const [actionOpProgress, setActionOpProgress] = useState<ActionOpProgress | null>(null);
  const pendingConfirmRef = useRef<PendingConfirm>(null);

  pendingConfirmRef.current = pendingConfirm;

  const focusedFile = (): FileNode | null => {
    const node = focused?.node;
    return node?.kind === 'file' ? node : null;
  };

  const runWrite = useCallback(
    (
      label: string,
      work: () => Promise<void>,
      opts?: { actionOp?: ActionOpKind; actionOpTotal?: number },
    ) => {
      if (busyRef.current) {
        setStatusMessage('Busy…');
        return;
      }
      busyRef.current = true;
      if (opts?.actionOp) {
        setActionOp(opts.actionOp);
        setActionOpProgress({ done: 0, total: opts.actionOpTotal ?? 0 });
      }
      void (async () => {
        try {
          await work();
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          setStatusMessage(`${label} failed: ${msg.slice(0, 80)}`);
        } finally {
          busyRef.current = false;
          if (opts?.actionOp) {
            setActionOp(null);
            setActionOpProgress(null);
          }
        }
      })();
    },
    [busyRef],
  );

  const dispatch = useCallback(
    (action: Action): 'quit' | void => {
      switch (action.type) {
        case 'stage': {
          const files = focused
            ? collectFiles(focused).filter((f) => f.unstaged || f.untracked)
            : [];
          if (files.length === 0) {
            const kind = focused?.node.kind;
            setStatusMessage(
              kind === 'file' || kind === 'dir' || kind === 'repo' || kind === 'checkout'
                ? 'Nothing to stage'
                : 'Focus a file, dir, checkout, or repo to stage',
            );
            return;
          }
          const byRepo = new Map<string, FileNode[]>();
          for (const file of files) {
            const list = byRepo.get(file.repoPath) ?? [];
            list.push(file);
            byRepo.set(file.repoPath, list);
          }
          runWrite('Stage', async () => {
            const touched: string[] = [];
            for (const [repo, repoFiles] of byRepo) {
              const repoDir = path.join(cwd, repo);
              for (const file of repoFiles) {
                const result = await runOnPaths(opPaths(file), (p) => stageFile(repoDir, p));
                if (!result.ok) {
                  setStatusMessage(result.error ?? 'Stage failed');
                  if (touched.length > 0) await refreshRepos(touched);
                  return;
                }
                if (!touched.includes(repo)) touched.push(repo);
              }
            }
            const n = files.length;
            const msg = n === 1 ? `Staged ${files[0].path}` : `Staged ${n} files`;
            const repos = [...byRepo.keys()];
            if (repos.length === 1) {
              await refreshRepoOnly(repos[0], msg);
            } else {
              await refreshRepos(repos);
              setStatusMessage(msg);
            }
          });
          return;
        }
        case 'unstage': {
          const files = focused ? collectFiles(focused).filter((f) => f.staged) : [];
          if (files.length === 0) {
            const kind = focused?.node.kind;
            setStatusMessage(
              kind === 'file' || kind === 'dir' || kind === 'repo' || kind === 'checkout'
                ? 'Nothing to unstage'
                : 'Focus a file, dir, checkout, or repo to unstage',
            );
            return;
          }
          const byRepo = new Map<string, FileNode[]>();
          for (const file of files) {
            const list = byRepo.get(file.repoPath) ?? [];
            list.push(file);
            byRepo.set(file.repoPath, list);
          }
          runWrite('Unstage', async () => {
            const touched: string[] = [];
            for (const [repo, repoFiles] of byRepo) {
              const repoDir = path.join(cwd, repo);
              for (const file of repoFiles) {
                const result = await runOnPaths(opPaths(file), (p) => unstageFile(repoDir, p));
                if (!result.ok) {
                  setStatusMessage(result.error ?? 'Unstage failed');
                  if (touched.length > 0) await refreshRepos(touched);
                  return;
                }
                if (!touched.includes(repo)) touched.push(repo);
              }
            }
            const n = files.length;
            const msg = n === 1 ? `Unstaged ${files[0].path}` : `Unstaged ${n} files`;
            const repos = [...byRepo.keys()];
            if (repos.length === 1) {
              await refreshRepoOnly(repos[0], msg);
            } else {
              await refreshRepos(repos);
              setStatusMessage(msg);
            }
          });
          return;
        }
        case 'revert': {
          const files = focused ? collectFiles(focused).filter(isRevertible) : [];
          if (files.length === 0) {
            const kind = focused?.node.kind;
            if (
              kind === 'file' &&
              focusedFile()?.staged &&
              !focusedFile()?.unstaged &&
              !focusedFile()?.untracked
            ) {
              setStatusMessage('Nothing to discard (staged only)');
              return;
            }
            setStatusMessage(
              kind === 'file' || kind === 'dir' || kind === 'repo' || kind === 'checkout'
                ? 'Nothing to discard'
                : 'Focus a file, dir, checkout, or repo to revert',
            );
            return;
          }
          if (!focused) return;
          setPendingConfirm(buildPendingConfirm(focused, files));
          setConfirmMode(true);
          setStatusMessage('');
          return;
        }
        case 'edit': {
          const file = focusedFile();
          if (!file) {
            setStatusMessage('Focus a file to edit');
            return;
          }
          /**
           * GUI editors stay mounted (detached spawn). TTY editors record the
           * file and unmount in the same keypress. Writing
           * `ExitReason.type === 'edit'` from here would leave a window where
           * Ctrl+C after `e` still launched the editor; `pendingEdit` + quit
           * avoids that. `run.ts` opens the TTY `$EDITOR` after `waitUntilExit`.
           */
          return startEdit({
            editor: resolveEditor(process.env, editorConfig),
            request: { repoPath: file.repoPath, filePath: file.path },
            cwd,
            onEditRequest,
            onDetachedError: (message) => {
              setStatusMessage(`Failed to launch editor: ${message}`);
            },
          });
        }
        case 'confirmYes':
        case 'confirmYesClean': {
          const pending = pendingConfirmRef.current;
          if (!pending) {
            setPendingConfirm(null);
            setConfirmMode(false);
            return;
          }
          // Busy check before clearing — otherwise runWrite refuses and the
          // confirm payload is gone with no way to retry y/Y.
          if (busyRef.current) {
            setStatusMessage('Busy…');
            return;
          }
          setPendingConfirm(null);
          setConfirmMode(false);

          if (pending.kind === 'removeWorktree') {
            const primaryAbs = path.join(cwd, pending.primaryRepo);
            const wtAbs = path.join(cwd, pending.path);
            runWrite('Remove worktree', async () => {
              const result = await removeWorktree(primaryAbs, wtAbs, {
                force: pending.force,
              });
              if (!result.ok) {
                setStatusMessage(result.error ?? 'Remove worktree failed');
                return;
              }
              // Drop linked path (refresh returns null). Refresh primary only if
              // it was already listed — never append a filtered-out primary.
              await refreshRepos(
                refreshPathsAfterRemoveWorktree(
                  pending.path,
                  pending.primaryRepo,
                  snapshots.map((s) => s.repo),
                ),
              );
              setStatusMessage(`Removed worktree ${pending.path}`);
            });
            return;
          }

          const clean = action.type === 'confirmYesClean';
          const deleteUntracked = shouldDeleteUntracked(pending.targets, clean);
          const repoDir = path.join(cwd, pending.repo);
          runWrite('Revert', async () => {
            let touched = false;
            for (const target of pending.targets) {
              let result: GitOpResult;
              if (target.untracked) {
                if (!deleteUntracked) continue;
                result = await removeUntrackedFile(repoDir, target.path);
              } else {
                result = await runOnPaths(targetPaths(target), (p) =>
                  revertTrackedFile(repoDir, p),
                );
              }
              if (!result.ok) {
                setStatusMessage(result.error ?? 'Revert failed');
                if (touched) await refreshRepos([pending.repo]);
                return;
              }
              touched = true;
            }
            await refreshRepoOnly(pending.repo, formatRevertStatus(pending, deleteUntracked));
          });
          return;
        }
        case 'confirmNo':
          setPendingConfirm(null);
          setConfirmMode(false);
          setStatusMessage('Cancelled');
          return;
        case 'fetch': {
          const node = focused?.node;
          if (!node || node.kind === 'group') {
            return;
          }
          const repos = collectBulkGitTargets(focused, snapshots, ignoredRepos, showIgnored);
          if (repos.length === 0) {
            setStatusMessage('Nothing to fetch');
            return;
          }
          runFetch(repos, { manual: true });
          return;
        }
        case 'pull': {
          const node = focused?.node;
          if (
            !node ||
            (node.kind !== 'workspace' && node.kind !== 'repo' && node.kind !== 'checkout')
          ) {
            return;
          }
          if (!canPull(focused, snapshots, ignoredRepos, showIgnored)) {
            setStatusMessage('Nothing to pull');
            return;
          }
          const repos = collectBulkGitTargets(focused, snapshots, ignoredRepos, showIgnored).filter(
            (path) => {
              const snap = snapshots.find((s) => s.repo === path);
              if (snap) return snap.syncStatus === 'behind';
              return node.kind === 'checkout' || node.kind === 'repo';
            },
          );
          if (repos.length === 0) {
            setStatusMessage('Nothing to pull');
            return;
          }
          runWrite(
            'Pull',
            async () => {
              const result = await tuiPullRepos(cwd, repos, {
                onProgress: (done, total) => setActionOpProgress({ done, total }),
              });
              await refreshRepos(repos);
              flashNodes(repos.flatMap((repo) => [repoNodeId(repo), checkoutNodeId(repo)]));
              setStatusMessage(formatPullStatus(result.ok, result.failed, repos.length));
            },
            { actionOp: 'pull', actionOpTotal: repos.length },
          );
          return;
        }
        case 'push': {
          const node = focused?.node;
          if (!node || (node.kind !== 'repo' && node.kind !== 'checkout')) {
            return;
          }
          if (!canPush(focused, snapshots, ignoredRepos, showIgnored)) {
            setStatusMessage('Nothing to push');
            return;
          }
          const repos = collectBulkGitTargets(focused, snapshots, ignoredRepos, showIgnored).filter(
            (path) => {
              const snap = snapshots.find((s) => s.repo === path);
              if (!snap) return node.kind === 'checkout' || node.kind === 'repo';
              return (
                snap.syncStatus === 'ahead' ||
                snap.syncStatus === 'diverged' ||
                snap.syncStatus === 'no-upstream'
              );
            },
          );
          if (repos.length === 0) {
            setStatusMessage('Nothing to push');
            return;
          }
          runWrite(
            'Push',
            async () => {
              const result = await tuiPushRepos(cwd, repos, {
                onProgress: (done, total) => setActionOpProgress({ done, total }),
              });
              await refreshRepos(repos);
              flashNodes(repos.flatMap((repo) => [repoNodeId(repo), checkoutNodeId(repo)]));
              setStatusMessage(formatPushStatus(result.ok, result.failed, repos.length));
            },
            { actionOp: 'push', actionOpTotal: repos.length },
          );
          return;
        }
        case 'defaultBranch': {
          const node = focused?.node;
          if (
            !node ||
            (node.kind !== 'workspace' && node.kind !== 'repo' && node.kind !== 'checkout')
          ) {
            return;
          }
          if (!canDefaultBranch(focused, snapshots, ignoredRepos, showIgnored)) {
            setStatusMessage('Nothing to switch');
            return;
          }
          const tasks = collectBulkGitTargets(
            focused,
            snapshots,
            ignoredRepos,
            showIgnored,
          ).flatMap((path) => {
            const snap = snapshots.find((s) => s.repo === path);
            if (!snap) {
              return [{ repoPath: path, currentBranch: '' }];
            }
            if (
              node.kind !== 'checkout' &&
              isDefaultBranch(snap.branch, snap.defaultBranchOverride)
            ) {
              return [];
            }
            return [
              {
                repoPath: snap.repo,
                currentBranch: snap.branch,
                defaultBranchOverride: snap.defaultBranchOverride,
              },
            ];
          });
          if (tasks.length === 0) {
            setStatusMessage('Nothing to switch');
            return;
          }
          runWrite(
            'Default branch',
            async () => {
              const outcomes = await tuiSwitchReposToDefault(cwd, tasks, {
                onProgress: (done, total) => setActionOpProgress({ done, total }),
              });
              const repos = tasks.map((t) => t.repoPath);
              await refreshRepos(repos);
              flashNodes(repos.flatMap((repo) => [repoNodeId(repo), checkoutNodeId(repo)]));
              setStatusMessage(formatSwitchStatus(outcomes));
            },
            { actionOp: 'defaultBranch', actionOpTotal: tasks.length },
          );
          return;
        }
        case 'branch': {
          const pickerPath = branchPickerPath(focused);
          if (!pickerPath) {
            setStatusMessage('Focus a checkout or repo to switch branch');
            return;
          }
          openBranchPicker(pickerPath);
          return;
        }
        case 'removeWorktree': {
          const node = focused?.node;
          if (
            !canRemoveWorktree(focused) ||
            !node ||
            (node.kind !== 'checkout' && node.kind !== 'repo')
          ) {
            setStatusMessage('Focus a linked worktree to remove');
            return;
          }
          if (!node.primaryRepo) {
            setStatusMessage('Linked worktree missing primary path');
            return;
          }
          setPendingConfirm(buildRemoveWorktreeConfirm(node));
          setConfirmMode(true);
          setStatusMessage('');
          return;
        }
        default:
          return;
      }
    },
    // focusedFile closes over `focused`, so it must be a dependency.
    [
      cwd,
      focused,
      onEditRequest,
      editorConfig,
      refreshRepoOnly,
      refreshRepos,
      openBranchPicker,
      runFetch,
      runWrite,
      flashNodes,
      setConfirmMode,
      snapshots,
      ignoredRepos,
      showIgnored,
    ],
  );

  return {
    dispatch,
    pendingConfirm,
    statusMessage,
    setStatusMessage,
    actionOp,
    actionOpProgress,
  };
}
