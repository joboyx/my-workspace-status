import assert from 'node:assert';
import { describe, it } from 'node:test';
import { isValidBranchName } from '../src/tui/createBranchName.js';

describe('isValidBranchName', () => {
  it('accepts feature/foo', () => assert.equal(isValidBranchName('feature/foo'), true));
  it('rejects empty and spaces', () => {
    assert.equal(isValidBranchName(''), false);
    assert.equal(isValidBranchName('  '), false);
    assert.equal(isValidBranchName('a b'), false);
  });
  it('rejects leading dash', () => assert.equal(isValidBranchName('-bad'), false));
});
