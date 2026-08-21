import assert from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  allocateChromeRow,
  formatActionOpProgress,
  formatTopOpStatus,
  isOpStatusError,
} from '../src/tui/opStatus.js';

describe('formatActionOpProgress', () => {
  it('matches fetch done/total wording', () => {
    assert.equal(formatActionOpProgress('pull', 2, 18), 'Pulling 2/18…');
    assert.equal(formatActionOpProgress('push', 1, 3), 'Pushing 1/3…');
    assert.equal(formatActionOpProgress('defaultBranch', 0, 5), 'Switching 0/5…');
  });

  it('omits the count when total is unknown', () => {
    assert.equal(formatActionOpProgress('pull'), 'Pulling…');
    assert.equal(formatActionOpProgress('push', 0, 0), 'Pushing…');
    assert.equal(formatActionOpProgress('defaultBranch'), 'Switching…');
  });
});

describe('isOpStatusError', () => {
  it('detects failed/error like the old StatusBar cue', () => {
    assert.equal(isOpStatusError('fetch: 2 failed'), true);
    assert.equal(isOpStatusError('Checkout error: dirty'), true);
    assert.equal(isOpStatusError('Fetched'), false);
    assert.equal(isOpStatusError('fetched 1m ago · Busy…'), false);
  });
});

describe('formatTopOpStatus', () => {
  it('prefers in-progress action over fetch line', () => {
    assert.equal(
      formatTopOpStatus({
        actionOp: 'pull',
        actionOpProgress: { done: 3, total: 10 },
        fetchStatusLine: 'Fetching 3/10…',
      }),
      'Pulling 3/10…',
    );
    assert.equal(
      formatTopOpStatus({
        actionOp: 'defaultBranch',
        actionOpProgress: { done: 1, total: 4 },
        fetchStatusLine: 'fetched just now',
      }),
      'Switching 1/4…',
    );
  });

  it('falls back to fetch status when idle', () => {
    assert.equal(
      formatTopOpStatus({
        actionOp: null,
        fetchStatusLine: 'Fetching 3/10…',
      }),
      'Fetching 3/10…',
    );
    assert.equal(
      formatTopOpStatus({
        actionOp: null,
        fetchStatusLine: 'fetched 1m ago',
      }),
      'fetched 1m ago',
    );
    assert.equal(formatTopOpStatus({ actionOp: null, fetchStatusLine: '' }), '');
  });

  it('appends toasts after action/fetch, joined with ·', () => {
    assert.equal(
      formatTopOpStatus({
        actionOp: 'pull',
        actionOpProgress: { done: 1, total: 2 },
        fetchStatusLine: 'Fetching 1/2…',
        toasts: ['Busy…'],
      }),
      'Pulling 1/2… · Busy…',
    );
    assert.equal(
      formatTopOpStatus({
        actionOp: null,
        fetchStatusLine: 'fetched 1m ago',
        toasts: ['Refreshed workspace'],
      }),
      'fetched 1m ago · Refreshed workspace',
    );
    assert.equal(
      formatTopOpStatus({
        actionOp: null,
        fetchStatusLine: '',
        toasts: ['Staged', 'Fetched'],
      }),
      'Staged · Fetched',
    );
  });

  it('keeps at most 3 toasts', () => {
    assert.equal(
      formatTopOpStatus({
        actionOp: null,
        fetchStatusLine: 'fetched just now',
        toasts: ['a', 'b', 'c', 'd'],
      }),
      'fetched just now · a · b · c',
    );
    assert.equal(
      formatTopOpStatus({
        actionOp: null,
        fetchStatusLine: '',
        toasts: ['one', 'two', 'three', 'four'],
      }),
      'one · two · three',
    );
  });
});

describe('allocateChromeRow', () => {
  it('gives full width to breadcrumb when op status is empty', () => {
    assert.deepEqual(allocateChromeRow(80, 0), {
      breadcrumbMax: 80,
      opStatusMax: 0,
    });
  });

  it('reserves trailing op status and truncates breadcrumb first', () => {
    assert.deepEqual(allocateChromeRow(40, 14), {
      breadcrumbMax: 25,
      opStatusMax: 14,
    });
  });

  it('clamps op status when wider than the row', () => {
    assert.deepEqual(allocateChromeRow(10, 20), {
      breadcrumbMax: 0,
      opStatusMax: 10,
    });
  });

  it('Breadcrumb passes breadcrumbMax 0 through (no || undefined coerce)', () => {
    // allocateChromeRow returns 0 when status wins; truthy coerce would drop the clamp.
    const { breadcrumbMax } = allocateChromeRow(10, 20);
    assert.equal(breadcrumbMax, 0);
    const src = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../src/tui/Breadcrumb.tsx'),
      'utf8',
    );
    assert.match(src, /width=\{breadcrumbMax\}/);
    assert.doesNotMatch(src, /breadcrumbMax\s*\|\|/);
  });
});

describe('op-status chrome wiring', () => {
  it('Breadcrumb colors error op-status with PALETTE.deleted', () => {
    const src = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../src/tui/Breadcrumb.tsx'),
      'utf8',
    );
    assert.match(src, /isOpStatusError\(opStatusLine\)/);
    assert.match(src, /PALETTE\.deleted/);
  });

  it('useAppState omits statusMessage toasts while overlays are open', () => {
    const src = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../src/tui/useAppState.ts'),
      'utf8',
    );
    assert.match(src, /overlayActive/);
    assert.match(src, /toasts:\s*overlayActive\s*\?\s*\[\]/);
    assert.match(src, /branchPicker\s*!=\s*null/);
    assert.match(src, /createBranchOverlay\s*!=\s*null/);
    assert.match(src, /graphBranchPicker\s*!=\s*null/);
  });

  it('useAppState excludes CTRL_C_EXIT_PROMPT from breadcrumb toasts', () => {
    const src = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../src/tui/useAppState.ts'),
      'utf8',
    );
    assert.match(src, /import\s*\{\s*CTRL_C_EXIT_PROMPT\s*\}\s*from\s*'\.\/ctrlCExit\.js'/);
    assert.match(src, /statusMessage\s*!==\s*CTRL_C_EXIT_PROMPT/);
  });

  it('App pins CTRL_C_EXIT_PROMPT outside overlay pickers', () => {
    const src = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../src/tui/App.tsx'),
      'utf8',
    );
    assert.match(src, /exitPromptPinned/);
    assert.match(src, /statusMessage\s*===\s*CTRL_C_EXIT_PROMPT/);
    assert.match(src, /!state\.branchPicker/);
    assert.match(src, /!state\.createBranchOverlay/);
    assert.match(src, /!state\.stashMenuOps/);
    assert.match(src, /!state\.graphBranchPicker/);
  });

  it('useActions tracks actionOpProgress and passes onProgress into batches', () => {
    const src = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../src/tui/useActions.ts'),
      'utf8',
    );
    assert.match(src, /actionOpProgress/);
    assert.match(src, /setActionOpProgress\(\{ done: 0, total: opts\.actionOpTotal/);
    assert.match(src, /tuiPullRepos\(cwd,\s*repos,\s*\{\s*onProgress:/);
    assert.match(src, /tuiPushRepos\(cwd,\s*repos,\s*\{\s*onProgress:/);
    assert.match(src, /tuiSwitchReposToDefault\(cwd,\s*tasks,\s*\{\s*onProgress:/);
  });

  it('useAppState passes actionOpProgress into formatTopOpStatus', () => {
    const src = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../src/tui/useAppState.ts'),
      'utf8',
    );
    assert.match(src, /actionOpProgress/);
    assert.match(src, /formatTopOpStatus\(\{[\s\S]*actionOpProgress/);
  });
});
