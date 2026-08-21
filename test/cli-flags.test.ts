import assert from 'node:assert';
import { describe, it } from 'node:test';

import { parseArgs } from '../src/cli.js';

describe('parseArgs', () => {
  it('defaults forcePlain, forceJson, and forceTui to false', () => {
    const flags = parseArgs([]);
    assert.equal(flags.forcePlain, false);
    assert.equal(flags.forceJson, false);
    assert.equal(flags.forceTui, false);
  });

  it('sets forcePlain for --plain', () => {
    const flags = parseArgs(['--plain']);
    assert.equal(flags.forcePlain, true);
    assert.equal(flags.forceTui, false);
  });

  it('sets forceJson for --json', () => {
    const flags = parseArgs(['--json']);
    assert.equal(flags.forceJson, true);
    assert.equal(flags.forcePlain, false);
    assert.equal(flags.forceTui, false);
  });

  it('sets forceTui for -i', () => {
    const flags = parseArgs(['-i']);
    assert.equal(flags.forceTui, true);
    assert.equal(flags.forcePlain, false);
  });

  it('sets forceTui for --tui', () => {
    const flags = parseArgs(['--tui']);
    assert.equal(flags.forceTui, true);
  });

  it('keeps -v/-p/-d on report-path flags', () => {
    const flags = parseArgs(['-v', '-p', '-d', '--plain']);
    assert.equal(flags.verbose, true);
    assert.equal(flags.doPull, true);
    assert.equal(flags.doDefaultBranch, true);
    assert.equal(flags.forcePlain, true);
  });

  it('parses -i with other short flags', () => {
    const flags = parseArgs(['-afi']);
    assert.equal(flags.includeAll, true);
    assert.equal(flags.doFetch, true);
    assert.equal(flags.forceTui, true);
  });
});
