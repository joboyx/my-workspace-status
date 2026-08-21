import {
  computeRefsFingerprint,
  execGit,
  gitLogCommitsByIds,
  gitLogGraphWindow,
  listRefs,
  listStashes,
  repoHasPorcelainChanges,
} from '../../git.js';
import {
  DEFAULT_GRAPH_WINDOW,
  type GraphCommit,
  type GraphModel,
} from './types.js';

/**
 * Resolve HEAD SHA, or null when unavailable (empty repo / error).
 */
async function resolveHeadId(repoPath: string): Promise<string | null> {
  try {
    const id = (await execGit(['rev-parse', 'HEAD'], repoPath)).trim();
    return id.length > 0 ? id : null;
  } catch {
    return null;
  }
}

/**
 * Load commits, refs, stashes, HEAD, and uncommitted state for one repo window.
 */
export async function loadGraphModel(
  repoPath: string,
  opts: { skip?: number; limit?: number } = {},
): Promise<GraphModel> {
  const skip = opts.skip ?? 0;
  const limit = opts.limit ?? DEFAULT_GRAPH_WINDOW;

  const [logPage, refs, stashes, refsFingerprint, hasChanges, headId] =
    await Promise.all([
      gitLogGraphWindow(repoPath, { skip, limit }),
      listRefs(repoPath),
      listStashes(repoPath),
      computeRefsFingerprint(repoPath),
      repoHasPorcelainChanges(repoPath),
      resolveHeadId(repoPath),
    ]);

  const byId = new Map<string, typeof refs>();
  for (const ref of refs) {
    const list = byId.get(ref.commitId) ?? [];
    list.push(ref);
    byId.set(ref.commitId, list);
  }

  const windowCommits: GraphCommit[] = logPage.commits.map((c) => ({
    ...c,
    refs: byId.get(c.id) ?? [],
  }));

  // Stash^1 may sit outside the log window (or only be reachable via the
  // stash reflog). Fetch those commits so park-on-parent can join instead
  // of stacking lone ◇ after uncommitted. Keep full git `%P` on extras —
  // `graphLayoutCommits` drops ids that are not in the merged layout set.
  // Appended after the window prefix; skip/limit/hasMore/windowCount stay
  // on the log page.
  const inWindow = new Set(windowCommits.map((c) => c.id));
  const extraCommits: GraphCommit[] = [];
  const missingParents = [
    ...new Set(
      stashes
        .map((s) => s.parentId)
        .filter((id) => id.length > 0 && !inWindow.has(id)),
    ),
  ];
  if (missingParents.length > 0) {
    const extra = await gitLogCommitsByIds(repoPath, missingParents);
    for (const raw of extra) {
      if (inWindow.has(raw.id)) continue;
      extraCommits.push({
        ...raw,
        refs: byId.get(raw.id) ?? [],
      });
      inWindow.add(raw.id);
    }
  }

  return {
    repoPath,
    commits: [...windowCommits, ...extraCommits],
    stashes,
    uncommitted: { kind: 'uncommitted', hasChanges },
    headId,
    refsFingerprint,
    skip,
    limit,
    hasMore: logPage.truncated,
    windowCount: windowCommits.length,
  };
}
