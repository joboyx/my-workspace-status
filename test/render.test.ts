import assert from 'node:assert';
import { describe, it } from 'node:test';

import {
  formatBranchWithMerge,
  formatCheckoutRepoLabel,
  formatMergeMark,
} from '../src/helpers.js';
import { buildSummaryState, buildVerboseRows } from '../src/snapshot.js';
import { renderWorkspaceStatus } from '../src/render.js';
import type { RepoSnapshot } from '../src/types.js';

function snap(partial: Partial<RepoSnapshot> & Pick<RepoSnapshot, 'repo' | 'branch'>): RepoSnapshot {
  return {
    syncStatus: 'up-to-date',
    syncNote: '',
    hasUnstaged: false,
    hasStaged: false,
    hasUntracked: false,
    unstagedInfo: '',
    stagedFiles: '',
    unstagedFiles: '',
    untrackedFiles: '',
    checkoutKind: 'primary',
    mergedIntoDefault: null,
    ...partial,
  };
}

describe('linked / merge label helpers', () => {
  it('formatMergeMark', () => {
    assert.equal(formatMergeMark(true), ' ✅');
    assert.equal(formatMergeMark(false), ' 🌱');
    assert.equal(formatMergeMark(null), '');
  });

  it('formatCheckoutRepoLabel prefixes linked paths', () => {
    assert.equal(formatCheckoutRepoLabel(snap({ repo: 'app', branch: 'main' })), 'app');
    assert.equal(
      formatCheckoutRepoLabel(
        snap({
          repo: 'app/.worktrees/feat',
          branch: 'feature/x',
          checkoutKind: 'linked',
          primaryRepo: 'app',
        }),
      ),
      '🔗 app/.worktrees/feat',
    );
  });

  it('formatBranchWithMerge appends merge marks', () => {
    assert.equal(formatBranchWithMerge('🚧 feature/x', false), '🚧 feature/x 🌱');
    assert.equal(formatBranchWithMerge('🚧 feature/x', true), '🚧 feature/x ✅');
    assert.equal(formatBranchWithMerge('🔥 main', null), '🔥 main');
  });
});

describe('renderWorkspaceStatus', () => {
  it('renders verbose rows and summary sections from snapshots', () => {
    const snapshots: RepoSnapshot[] = [
      snap({ repo: 'clean-main', branch: 'main' }),
      snap({
        repo: 'feature-service',
        branch: 'feature/ABCD-1234-output',
        syncStatus: 'ahead',
        syncNote: 'ahead by 1 commits',
        hasUnstaged: true,
        hasStaged: true,
        hasUntracked: true,
        unstagedInfo: 'modified',
        stagedFiles: 'A\tstaged.txt',
        unstagedFiles: 'M\ttracked.txt',
        untrackedFiles: 'notes.txt',
      }),
      snap({
        repo: 'bugfix-service',
        branch: 'bugfix/ABCD-5678-fix',
        syncStatus: 'behind',
        syncNote: 'behind by 2 commits',
      }),
    ];

    const verbose = buildVerboseRows(snapshots);
    const summary = buildSummaryState(snapshots);

    assert.deepStrictEqual(
      renderWorkspaceStatus({ snapshots, summary, verbose, showVerbose: true }),
      [
        'Repo                  Branch                       Sync         Files          ',
        'clean-main            🔥 main                      ✅ current   💾 clean       ',
        'bugfix-service        🐛 bugfix/ABCD-5678-fix      ⬇️ behind 2  💾 clean       ',
        'feature-service       🚧 feature/ABCD-1234-output  ⬆️ ahead 1   ⚠️ staged+dirty  (modified)',
        '',
        'File changes',
        '  📦 feature-service (ABCD-1234)',
        '     ├─ 🟢A notes.txt',
        '     ├─ 🟢A staged.txt',
        '     └─ 🟡M tracked.txt',
        '',
        '🔄 Sync status (2):',
        '  ⬇️ behind:',
        '    - bugfix-service [bugfix/ABCD-5678-fix] (ABCD-5678) - behind by 2 commits',
        '  ⬆️ ahead:',
        '    - feature-service [feature/ABCD-1234-output] (ABCD-1234) - ahead by 1 commits',
        '',
        '🌿 Branches (2):',
        '  🚧 feature:',
        '    - feature-service (ABCD-1234)',
        '  🐛 bugfix:',
        '    - bugfix-service (ABCD-5678)',
      ],
    );
  });

  it('renders Files header, 🔗 linked repo, merge marks, and Linked summary', () => {
    const snapshots: RepoSnapshot[] = [
      snap({ repo: 'app', branch: 'main' }),
      snap({
        repo: 'app/.worktrees/NDRMD-1422-asr',
        branch: 'feature/NDRMD-1422-asr',
        checkoutKind: 'linked',
        primaryRepo: 'app',
        mergedIntoDefault: false,
      }),
    ];

    const verbose = buildVerboseRows(snapshots);
    const summary = buildSummaryState(snapshots);
    const lines = renderWorkspaceStatus({ snapshots, summary, verbose, showVerbose: true });

    assert.ok(lines[0]?.includes('Files'), `expected Files header, got: ${lines[0]}`);
    assert.ok(!lines[0]?.includes('Worktree'), 'header must not say Worktree');
    assert.ok(
      lines.some((l) => l.includes('🔗 app/.worktrees/NDRMD-1422-asr')),
      'expected 🔗 linked repo in verbose table',
    );
    assert.ok(
      lines.some((l) => l.includes('🚧 feature/NDRMD-1422-asr 🌱')),
      'expected merge mark on verbose branch cell',
    );
    assert.ok(
      lines.some((l) => l.startsWith('🔗 Linked worktrees (1):')),
      'expected Linked worktrees summary section',
    );
    assert.ok(
      lines.some((l) => l.includes('🔗 app/.worktrees/NDRMD-1422-asr (NDRMD-1422) 🌱')),
      'expected linked summary line with ticket and merge mark',
    );
  });

  it('keeps linked worktrees adjacent after primary in plain verbose buckets', () => {
    const snapshots: RepoSnapshot[] = [
      snap({
        repo: 'rsps-api-extra',
        branch: 'feature/extra',
        hasUnstaged: true,
        unstagedFiles: 'M\ta.ts',
      }),
      snap({
        repo: 'rsps-api/.worktrees/feat',
        branch: 'feature/feat',
        checkoutKind: 'linked',
        primaryRepo: 'rsps-api',
        mergedIntoDefault: false,
        hasUnstaged: true,
        unstagedFiles: 'M\tb.ts',
      }),
      snap({
        repo: 'rsps-api',
        branch: 'feature/primary',
        hasUnstaged: true,
        unstagedFiles: 'M\tc.ts',
      }),
    ];

    const { changeRepos } = buildVerboseRows(snapshots);
    assert.deepEqual(
      changeRepos.map((r) => r.repo),
      ['rsps-api', '🔗 rsps-api/.worktrees/feat', 'rsps-api-extra'],
    );
  });

  it('separates multiple repo file trees with repo headers and one blank line', () => {
    const snapshots: RepoSnapshot[] = [
      snap({
        repo: 'dotfiles',
        branch: 'feature/JBY-019-status-ui',
        hasUnstaged: true,
        unstagedFiles: 'M\tai/agents/codex/config.toml|||M\tai/agents/cursor/hooks.json',
      }),
      snap({
        repo: 'notes',
        branch: 'main',
        hasUntracked: true,
        untrackedFiles: 'tmp/monitor-order-vbrqjnfplq.log',
      }),
    ];

    assert.deepStrictEqual(
      renderWorkspaceStatus({
        snapshots,
        summary: buildSummaryState(snapshots),
        verbose: buildVerboseRows(snapshots),
        showVerbose: false,
      }).slice(0, 13),
      [
        'File changes',
        '  📦 dotfiles (JBY-019)',
        '     └─ ai/agents',
        '        ├─ codex',
        '        │  └─ 🟡M config.toml',
        '        └─ cursor',
        '           └─ 🟡M hooks.json',
        '',
        '  📦 notes',
        '     └─ tmp',
        '        └─ 🟢A monitor-order-vbrqjnfplq.log',
        '',
        '🌿 Branches (1):',
      ],
    );
  });

  it('renders clean summary without a leading blank line', () => {
    const snapshots: RepoSnapshot[] = [snap({ repo: 'clean-main', branch: 'main' })];

    assert.deepStrictEqual(
      renderWorkspaceStatus({
        snapshots,
        summary: buildSummaryState(snapshots),
        verbose: buildVerboseRows(snapshots),
        showVerbose: false,
      }),
      ['✅ All repos clean and up-to-date'],
    );
  });

  it('does not claim all-clean when no repos were discovered', () => {
    const snapshots: RepoSnapshot[] = [];
    assert.deepStrictEqual(
      renderWorkspaceStatus({
        snapshots,
        summary: buildSummaryState(snapshots),
        verbose: buildVerboseRows(snapshots),
        showVerbose: false,
      }),
      ['ℹ️ No git repos found'],
    );
  });

  it('lists unborn and failed repos under Attention instead of all-clean', () => {
    const snapshots: RepoSnapshot[] = [
      snap({
        repo: 'broken',
        branch: '(unknown)',
        syncStatus: 'no-upstream',
        syncNote: 'status failed',
      }),
      snap({
        repo: 'unborn',
        branch: 'main',
        syncStatus: 'no-upstream',
        syncNote: 'no commits yet',
      }),
    ];

    assert.deepStrictEqual(
      renderWorkspaceStatus({
        snapshots,
        summary: buildSummaryState(snapshots),
        verbose: buildVerboseRows(snapshots),
        showVerbose: false,
      }),
      [
        '⚠️ Attention (2):',
        '    - broken [(unknown)] - status failed',
        '    - unborn [main] - no commits yet',
      ],
    );
  });

  it('lists release and unknown non-default branches in the summary', () => {
    const snapshots: RepoSnapshot[] = [
      snap({
        repo: 'billing-service',
        branch: 'release/2026-07-20_CR2026053300',
      }),
      snap({
        repo: 'edge-proxy',
        branch: 'hotfix/urgent',
      }),
      snap({
        repo: 'workflow-api',
        branch: 'develop',
      }),
    ];

    assert.deepStrictEqual(
      renderWorkspaceStatus({
        snapshots,
        summary: buildSummaryState(snapshots),
        verbose: buildVerboseRows(snapshots),
        showVerbose: false,
      }),
      [
        '🌿 Branches (2):',
        '  🚀 release:',
        '    - billing-service [release/2026-07-20_CR2026053300]',
        '  🌿 unknown:',
        '    - edge-proxy [hotfix/urgent]',
      ],
    );
  });
});
