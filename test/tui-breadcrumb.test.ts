import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  breadcrumbChromePlain,
  breadcrumbPlain,
} from '../src/tui/Breadcrumb.js';
import type { NavState } from '../src/tui/nav/stack.js';

describe('breadcrumbPlain', () => {
  it('renders workspace-only left focus', () => {
    const nav: NavState = {
      stack: [{ kind: 'workspace' }],
      focusPane: 'left',
    };
    assert.equal(breadcrumbPlain(nav, 'workspace'), 'workspace · left');
  });

  it('includes drilled segments and right focus', () => {
    const nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: '/ws/demo-services', commitId: 'abcdef0' },
      ],
      focusPane: 'right',
    };
    assert.match(breadcrumbPlain(nav, 'workspace'), /demo-services/);
    assert.match(breadcrumbPlain(nav, 'workspace'), / · right$/);
  });
});

describe('breadcrumbChromePlain', () => {
  it('appends trailing op status opposite the path', () => {
    const nav: NavState = {
      stack: [{ kind: 'workspace' }],
      focusPane: 'left',
    };
    assert.equal(
      breadcrumbChromePlain(nav, 'workspace', 'Fetching 3/10…'),
      'workspace · left Fetching 3/10…',
    );
    assert.equal(breadcrumbChromePlain(nav, 'workspace'), 'workspace · left');
  });
});
