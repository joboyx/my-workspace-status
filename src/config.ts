/**
 * workspace-status JSON configuration loading.
 */

import * as fs from 'fs';
import * as path from 'path';
import { normalizeFilterRepo } from './helpers.js';
import type { WorkspaceStatusConfig } from './types.js';

export const CONFIG_FILENAME = '.workspace-status-config.json';

/** Default discovery depth: cwd children, grandchildren, and great-grandchildren. */
export const DEFAULT_MAX_DEPTH = 3;

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function normalizeIgnoredRepos(ignoredRepos: string[]): string[] {
  return [
    ...new Set(
      ignoredRepos
        .map((repo) => normalizeFilterRepo(repo.trim()))
        .filter(Boolean),
    ),
  ].sort();
}

/**
 * Normalize `defaultBranches` map keys (repo paths) and values (branch names).
 * Empty branch names are dropped; later duplicate keys after normalize win.
 */
export function normalizeDefaultBranches(
  defaultBranches: Record<string, string>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [rawRepo, rawBranch] of Object.entries(defaultBranches)) {
    if (typeof rawRepo !== 'string' || typeof rawBranch !== 'string') continue;
    const repo = normalizeFilterRepo(rawRepo.trim());
    const branch = rawBranch.trim();
    if (!repo || !branch) continue;
    out[repo] = branch;
  }
  return out;
}

/**
 * Build a full config with defaults applied.
 */
export function workspaceStatusConfig(
  overrides: Partial<WorkspaceStatusConfig> = {},
): WorkspaceStatusConfig {
  return {
    ignoredRepos: [],
    maxDepth: DEFAULT_MAX_DEPTH,
    defaultBranches: {},
    ...overrides,
  };
}

function parseMaxDepth(value: unknown): number {
  if (value === undefined) return DEFAULT_MAX_DEPTH;
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 1) {
    throw new Error(`${CONFIG_FILENAME} maxDepth must be a positive integer`);
  }
  return value;
}

function parseEditor(value: unknown): string | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'string') {
    throw new Error(`${CONFIG_FILENAME} editor must be a string`);
  }
  const trimmed = value.trim();
  return trimmed || undefined;
}

function parseDefaultBranches(value: unknown): Record<string, string> {
  if (value === undefined) return {};
  if (!isObject(value)) {
    throw new Error(`${CONFIG_FILENAME} defaultBranches must be an object`);
  }
  for (const [repo, branch] of Object.entries(value)) {
    if (typeof branch !== 'string') {
      throw new Error(
        `${CONFIG_FILENAME} defaultBranches values must be strings (key: ${repo})`,
      );
    }
  }
  return normalizeDefaultBranches(value as Record<string, string>);
}

/**
 * Look up a configured default-branch override for a repo path.
 */
export function defaultBranchOverrideFor(
  repoPath: string,
  defaultBranches: Record<string, string>,
): string | undefined {
  const normalized = normalizeFilterRepo(repoPath);
  const branch = defaultBranches[normalized];
  return branch || undefined;
}

/**
 * Load workspace-status config from the workspace root.
 * Missing config means no repos are ignored and maxDepth defaults to 3.
 */
export async function loadWorkspaceStatusConfig(cwd: string): Promise<WorkspaceStatusConfig> {
  const configPath = path.join(cwd, CONFIG_FILENAME);
  try {
    await fs.promises.access(configPath);
  } catch {
    return workspaceStatusConfig();
  }

  const parsed = JSON.parse(await fs.promises.readFile(configPath, 'utf-8')) as unknown;
  if (!isObject(parsed) || !Array.isArray(parsed.ignoredRepos)) {
    throw new Error(`${CONFIG_FILENAME} must contain an ignoredRepos string array`);
  }

  const invalidRepo = parsed.ignoredRepos.find((repo) => typeof repo !== 'string');
  if (invalidRepo !== undefined) {
    throw new Error(`${CONFIG_FILENAME} ignoredRepos must contain only strings`);
  }

  const editor = parseEditor(parsed.editor);
  return workspaceStatusConfig({
    ignoredRepos: normalizeIgnoredRepos(parsed.ignoredRepos),
    maxDepth: parseMaxDepth(parsed.maxDepth),
    defaultBranches: parseDefaultBranches(parsed.defaultBranches),
    ...(editor !== undefined ? { editor } : {}),
  });
}
