import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  buildCommitFileNodes,
  flattenCommitFiles,
} from '../src/tui/commitFiles/buildCommitFileTree.js';
import { unfoldForestAncestors } from '../src/tui/model/fold.js';
import { resolveListFocus } from '../src/tui/session.js';
import type { FileChange } from '../src/types.js';

const changes: FileChange[] = [
  { path: 'src/a.ts', unstagedStatus: 'M' },
  { path: 'src/util/b.ts', unstagedStatus: 'A' },
  { path: 'root.txt', unstagedStatus: 'D' },
];

describe('buildCommitFileNodes', () => {
  it('tree mode nests dirs', () => {
    const nodes = buildCommitFileNodes('demo', changes, true);
    const kinds = nodes.map((n) => n.kind).sort();
    assert.ok(kinds.includes('dir') || kinds.includes('file'));
    const flat = flattenCommitFiles(nodes, new Set(), true);
    assert.ok(flat.some((r) => r.node.kind === 'file' && r.node.path === 'src/a.ts'));
  });

  it('flat mode is file-only list', () => {
    const nodes = buildCommitFileNodes('demo', changes, false);
    assert.ok(nodes.every((n) => n.kind === 'file'));
    assert.equal(nodes.length, 3);
  });

  it('default treeMode true yields dir nodes for nested paths', () => {
    const nodes = buildCommitFileNodes('demo', changes, true);
    assert.ok(nodes.some((n) => n.kind === 'dir'));
  });
});

describe('unfoldForestAncestors', () => {
  it('opens folded parent dirs so a nested file is visible', () => {
    const nodes = buildCommitFileNodes('demo', changes, true);
    const folds = new Set(['dir:demo:src']);
    const hidden = flattenCommitFiles(nodes, folds, true);
    assert.ok(!hidden.some((r) => r.id === 'file:demo:src/a.ts'));

    const revealed = unfoldForestAncestors(nodes, folds, 'file:demo:src/a.ts');
    assert.ok(!revealed.has('dir:demo:src'));
    const visible = flattenCommitFiles(nodes, revealed, true);
    assert.ok(visible.some((r) => r.id === 'file:demo:src/a.ts'));
  });

  it('returns the same set when the id is missing', () => {
    const nodes = buildCommitFileNodes('demo', changes, true);
    const folds = new Set(['dir:demo:src']);
    assert.equal(unfoldForestAncestors(nodes, folds, 'file:demo:gone.ts'), folds);
  });

  it('restores list focus onto a file that was hidden under a fold', () => {
    const nodes = buildCommitFileNodes('demo', changes, true);
    const folds = new Set(['dir:demo:src']);
    const nextFolds = unfoldForestAncestors(nodes, folds, 'file:demo:src/util/b.ts');
    const painted = flattenCommitFiles(nodes, nextFolds, true);
    const restored = resolveListFocus(painted, 'file:demo:src/util/b.ts', 0);
    assert.equal(painted[restored.cursor]?.id, 'file:demo:src/util/b.ts');
    assert.equal(restored.focusId, 'file:demo:src/util/b.ts');
  });
});
