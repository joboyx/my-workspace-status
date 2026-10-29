//! Groups of PRs for the PRs tab: the group key ladder, the cleaned row
//! title, the next action of a group, the group and row order, and the
//! optional title command.
//!
//! Everything here is pure except [`run_title_command`], which runs the
//! operator's program and blocks. The caller builds one [`PrEntry`] per
//! listed checkout and gets back the ordered [`PrGroup`]s; it never decides
//! a group, an order, or a header text itself.

// Only tests read this until the PRs tab does.
#![cfg_attr(not(test), allow(dead_code))]

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::pr_status::{NextAction, PrState, LONG_LIVED_BRANCHES};
use super::pull_request::{wait_until, PullRequest};

/// Header of the group that holds every PR without a key and without a
/// shared branch. It is always the last group.
pub const OTHER_KEY: &str = "other";

/// Finished groups start folded when more PRs than this are listed.
pub const FOLD_FINISHED_ABOVE: usize = 12;

/// Letters of a key that is a standard or a product name, never a ticket
/// (`CVE-2024-1234`, `UTF-8`, `SHA-256`).
const NOT_KEYS: [&str; 9] = [
    "CVE", "UTF", "SHA", "ISO", "HTTP", "RFC", "TLS", "MD", "GHSA",
];

/// Commit types that a title may start with (`feat(api): …`).
const COMMIT_TYPES: [&str; 11] = [
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// Markers that mark a draft in a title, compared without case.
const DRAFT_MARKERS: [&str; 6] = ["draft:", "[draft]", "(draft)", "wip:", "[wip]", "(wip)"];

/// Longest a title command may run before it is killed.
const TITLE_COMMAND_TIME_LIMIT: Duration = Duration::from_secs(2);

/// Most bytes read from the first line of a title command.
const TITLE_LINE_MAX: u64 = 4096;

/// One listed checkout and the PR of its branch: the input of
/// [`group_prs`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrEntry {
    /// Path of the checkout the PR belongs to.
    pub checkout: PathBuf,
    /// Repository name shown in the row, for example `acme/app`.
    pub repo: String,
    /// The PR of the checkout's current branch.
    pub pr: PullRequest,
    /// The default branch configured for the checkout, if any. A PR whose
    /// base is this branch is never stacked on another PR.
    pub default_branch: Option<String>,
}

/// How a group got its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupKind {
    /// A ticket key (`ABC-123`) found in the branch or the title.
    Ticket,
    /// No key, and two or more listed PRs share one head branch. The group
    /// key is the branch name.
    SharedBranch,
    /// No key and no shared branch; the key is [`OTHER_KEY`].
    Other,
}

/// The PR a row is stacked on: its base branch is that PR's head branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackedOn {
    /// Ref of the base PR (`#318`, `!12`).
    pub pr_ref: String,
    /// Number of the base PR.
    pub number: u64,
    /// State of the base PR; its label follows the ref (`after #318 (CI
    /// failed)`).
    pub state: PrState,
}

/// One row of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrRow {
    /// Path of the checkout the PR belongs to.
    pub checkout: PathBuf,
    /// Repository name shown in the row.
    pub repo: String,
    /// The PR.
    pub pr: PullRequest,
    /// The title to show: no draft marker, no commit type, and not the key
    /// of the group. See [`clean_title`].
    pub title: String,
    /// The known PR of the same repository this one is stacked on.
    pub stacked_on: Option<StackedOn>,
    /// What the author does next, with the stack taken into account; `None`
    /// when merged.
    pub next_action: Option<NextAction>,
}

/// The next action of a group, with the PR it targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextStep {
    /// The action.
    pub action: NextAction,
    /// Ref of the PR the action targets (`#318`, `!12`).
    pub pr_ref: String,
    /// State of that PR; it picks the colour of the header text.
    pub state: PrState,
}

impl NextStep {
    /// Header text: the action and the PR ref, such as `fix CI #318`,
    /// `merge !12`, or `wait for CI #320`. `merge <base> first` already
    /// names a ref, so it stays as it is: `merge #318 first`.
    pub fn text(&self) -> String {
        match self.action {
            NextAction::MergeBaseFirst(_) => self.action.text(),
            _ => format!("{} {}", self.action.text(), self.pr_ref),
        }
    }

    /// True when the move is the author's.
    pub fn needs_you(&self) -> bool {
        self.action.needs_you()
    }
}

/// What the header of a group says after the key and the title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupHeadline {
    /// The group has an open PR: the next action of the group.
    Next(NextStep),
    /// Every PR of the group is merged; holds how many. The UI writes
    /// `✔ all 2 merged`.
    AllMerged(usize),
    /// The [`GroupKind::Other`] group has no header text.
    Bare,
}

/// One group of the PRs tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrGroup {
    /// Ticket key, branch name, or [`OTHER_KEY`].
    pub key: String,
    /// How the key was found.
    pub kind: GroupKind,
    /// The header text after the key and the title.
    pub headline: GroupHeadline,
    /// Rows in display order: repo, then number, a stacked PR right after
    /// its base. Never empty.
    pub rows: Vec<PrRow>,
}

impl PrGroup {
    /// True when every PR of the group is merged. The
    /// [`GroupKind::Other`] group is never finished.
    pub fn is_finished(&self) -> bool {
        matches!(self.headline, GroupHeadline::AllMerged(_))
    }

    /// True when the title command may run for this group: only a ticket
    /// key names something the command can look up.
    pub fn wants_title(&self) -> bool {
        self.kind == GroupKind::Ticket
    }
}

/// Groups `entries` and orders everything for display.
///
/// Group order: a group with a CI failed row first, then the groups that
/// need the author, then the groups that wait, then the finished groups,
/// then [`OTHER_KEY`]; ties by key in natural order ([`natural_key_cmp`]).
/// Row order: repository name, then number, with a stacked PR right after its
/// base when both are in the group.
///
/// A PR is stacked on a known PR of the same repository, whatever group that
/// PR is in; the base branch must pass [`PullRequest::stacked_on`] with the
/// entry's `default_branch`. The repository is read from the PR URL.
pub fn group_prs(entries: Vec<PrEntry>) -> Vec<PrGroup> {
    let derived = stack_and_actions(&entries);
    let ticket_keys: Vec<Option<String>> = entries
        .iter()
        .map(|entry| ticket_key(&entry.pr.head, &entry.pr.title))
        .collect();
    let shared = shared_branches(&entries, &ticket_keys);
    let mut groups: HashMap<(GroupKind, String), Vec<PrRow>> = HashMap::new();
    for ((entry, ticket_key), (stacked_on, next_action)) in
        entries.into_iter().zip(ticket_keys).zip(derived)
    {
        let (kind, key) = match ticket_key {
            Some(key) => (GroupKind::Ticket, key),
            None if shared.contains(&entry.pr.head) => {
                (GroupKind::SharedBranch, entry.pr.head.clone())
            }
            None => (GroupKind::Other, OTHER_KEY.to_string()),
        };
        let own_key = (kind == GroupKind::Ticket).then_some(key.as_str());
        let title = clean_title(&entry.pr.title, own_key);
        groups.entry((kind, key)).or_default().push(PrRow {
            checkout: entry.checkout,
            repo: entry.repo,
            pr: entry.pr,
            title,
            stacked_on,
            next_action,
        });
    }
    let mut groups: Vec<PrGroup> = groups
        .into_iter()
        .map(|((kind, key), rows)| build_group(kind, key, rows))
        .collect();
    groups.sort_by(|a, b| {
        group_rank(a)
            .cmp(&group_rank(b))
            .then_with(|| natural_key_cmp(&a.key, &b.key))
    });
    groups
}

/// Orders group keys: the text before the first `-<digits>`, then those
/// digits as a number, then the rest as text, so `ABC-999` comes before
/// `ABC-1000` and `ABC-007` ties with `ABC-7`. A key without `-<digits>` is
/// all text. Equal parts fall back to the whole key, so the order is total.
fn natural_key_cmp(a: &str, b: &str) -> Ordering {
    let (letters_a, number_a, rest_a) = natural_parts(a);
    let (letters_b, number_b, rest_b) = natural_parts(b);
    letters_a
        .cmp(letters_b)
        .then(number_a.len().cmp(&number_b.len()))
        .then(number_a.cmp(number_b))
        .then(rest_a.cmp(rest_b))
        .then(a.cmp(b))
}

/// Splits `key` at its first `-<digits>`: the text before it, the digits
/// without leading zeros (compare them by length, then as text), and the
/// text after them. `(key, "", "")` when there is none.
fn natural_parts(key: &str) -> (&str, &str, &str) {
    let mut from = 0;
    while let Some(at) = key[from..].find('-') {
        let dash = from + at;
        let digits = key[dash + 1..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits > 0 {
            let end = dash + 1 + digits;
            return (
                &key[..dash],
                key[dash + 1..end].trim_start_matches('0'),
                &key[end..],
            );
        }
        from = dash + 1;
    }
    (key, "", "")
}

/// Number of PRs listed in `groups`: the `n` of the tab label `PRs <n>`.
pub fn listed(groups: &[PrGroup]) -> usize {
    groups.iter().map(|group| group.rows.len()).sum()
}

/// True when finished groups start folded: more than
/// [`FOLD_FINISHED_ABOVE`] PRs are listed.
pub fn finished_start_folded(groups: &[PrGroup]) -> bool {
    listed(groups) > FOLD_FINISHED_ABOVE
}

/// The title of a row: `title` without draft markers, without the commit
/// type, and without `group_key` (the key of a ticket group; `None` for the
/// other kinds). A commit scope that names an area stays as `scope: desc`.
/// When nothing is left, the full title.
///
/// `fix(api,ABC-123): add retry` with the key `ABC-123` gives `api: add
/// retry`; `feat: ABC-123: add retry` gives `add retry`.
pub fn clean_title(title: &str, group_key: Option<&str>) -> String {
    let text = strip_draft_markers(title);
    let text = strip_own_key(text, group_key);
    let cleaned = match commit_header(text) {
        Some(header) => {
            let description = strip_own_key(header.rest, group_key).trim();
            match header.scope.and_then(|scope| area_scope(scope, group_key)) {
                Some(area) if !description.is_empty() => format!("{area}: {description}"),
                _ => description.to_string(),
            }
        }
        None => text.trim().to_string(),
    };
    if cleaned.is_empty() {
        title.trim().to_string()
    } else {
        cleaned
    }
}

/// Runs `cmd <key>` and returns the group title it prints.
///
/// There is no shell: `key` is the one argument, and a key never starts
/// with `-`. Stdin is null and stderr is dropped. The result is the first
/// line of stdout, with control characters as spaces and trimmed, when the
/// command exits 0 within 2 seconds and that line is not empty. Anything
/// else (a missing file, a non-zero exit, no output, a timeout; the command
/// is then killed) gives `None`, with no error.
///
/// This blocks for up to 2 seconds. The caller runs it off the event
/// thread, and never for a group where [`PrGroup::wants_title`] is false;
/// [`TitleCache::fill`] keeps both rules.
pub fn run_title_command(cmd: &Path, key: &str) -> Option<String> {
    run_title_command_within(cmd, key, TITLE_COMMAND_TIME_LIMIT)
}

/// The titles of one poll cycle, by group key. The caller owns it and starts
/// a new one for each cycle, so the command runs at most once per key per
/// cycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TitleCache {
    titles: HashMap<String, Option<String>>,
}

impl TitleCache {
    /// Runs `cmd` for every ticket group in `groups` whose key has no entry
    /// yet. A key without a title is remembered too, so it does not run
    /// again. Blocks; see [`run_title_command`]. A group that is not a
    /// ticket group never runs the command.
    pub fn fill(&mut self, cmd: &Path, groups: &[PrGroup]) {
        for group in groups.iter().filter(|group| group.wants_title()) {
            self.titles
                .entry(group.key.clone())
                .or_insert_with(|| run_title_command(cmd, &group.key));
        }
    }

    /// The title of `key`; `None` when the command gave none or has not run.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.titles.get(key)?.as_deref()
    }
}

/// Group key of a PR, first match wins: a key at the start of a branch path
/// segment (`feature/ABC-123-retry`), a key as the commit scope or in a
/// scope list (`fix(api,ABC-123): …`), or a title prefix `ABC-123: ` or
/// `[ABC-123] `, also after the commit type (`feat: ABC-123: …`). A draft
/// marker before the title is ignored.
fn ticket_key(branch: &str, title: &str) -> Option<String> {
    if let Some(key) = branch_key(branch) {
        return Some(key.to_string());
    }
    let title = strip_draft_markers(title);
    let header = commit_header(title);
    header
        .as_ref()
        .and_then(|header| header.scope)
        .and_then(scope_key)
        .or_else(|| key_prefix(title).map(|(key, _)| key))
        .or_else(|| {
            header
                .as_ref()
                .and_then(|header| key_prefix(header.rest))
                .map(|(key, _)| key)
        })
        .map(str::to_string)
}

/// Length in bytes of the ticket key at the start of `text`:
/// `[A-Z][A-Z0-9]{1,9}-[0-9]+`, unless its letters are one of [`NOT_KEYS`].
fn key_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if !bytes.first()?.is_ascii_uppercase() {
        return None;
    }
    let letters = bytes
        .iter()
        .take(10)
        .take_while(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        .count();
    if letters < 2 || bytes.get(letters) != Some(&b'-') {
        return None;
    }
    let digits = bytes[letters + 1..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 || NOT_KEYS.contains(&&text[..letters]) {
        return None;
    }
    Some(letters + 1 + digits)
}

/// The key at the start of a path segment of `branch`, when the segment
/// goes on with `-`, `_`, `.`, or ends there.
fn branch_key(branch: &str) -> Option<&str> {
    branch.split('/').find_map(|segment| {
        let len = key_len(segment)?;
        matches!(segment[len..].chars().next(), None | Some('-' | '_' | '.'))
            .then(|| &segment[..len])
    })
}

/// The first item of a commit scope that is a key. A scope is a list when
/// it holds `,` or `/`.
fn scope_key(scope: &str) -> Option<&str> {
    scope
        .split([',', '/'])
        .map(str::trim)
        .find(|item| key_len(item) == Some(item.len()))
}

/// The scope without the key `group_key`, for a row title. `None` when
/// nothing is left. Without that key in it, the scope stays as written.
fn area_scope(scope: &str, group_key: Option<&str>) -> Option<String> {
    let scope = scope.trim();
    let area = match group_key {
        Some(key) if scope.split([',', '/']).any(|item| item.trim() == key) => scope
            .split([',', '/'])
            .map(str::trim)
            .filter(|item| !item.is_empty() && *item != key)
            .collect::<Vec<_>>()
            .join(","),
        _ => scope.to_string(),
    };
    (!area.is_empty()).then_some(area)
}

/// A title split as `type(scope)!: rest`.
struct CommitHeader<'a> {
    /// Text between the parentheses.
    scope: Option<&'a str>,
    /// Text after the colon, without leading white space.
    rest: &'a str,
}

/// Splits a conventional commit title. `None` unless it starts with one of
/// [`COMMIT_TYPES`] (any case), an optional `(scope)`, an optional `!`, and
/// a colon.
fn commit_header(title: &str) -> Option<CommitHeader<'_>> {
    let type_len = title
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(title.len());
    if !COMMIT_TYPES
        .iter()
        .any(|kind| kind.eq_ignore_ascii_case(&title[..type_len]))
    {
        return None;
    }
    let mut rest = &title[type_len..];
    let mut scope = None;
    if let Some(inner) = rest.strip_prefix('(') {
        let close = inner.find(')')?;
        scope = Some(&inner[..close]);
        rest = &inner[close + 1..];
    }
    let rest = rest.strip_prefix('!').unwrap_or(rest);
    Some(CommitHeader {
        scope,
        rest: rest.strip_prefix(':')?.trim_start(),
    })
}

/// Splits a title prefix `KEY: ` or `[KEY] ` into the key and the text
/// after it.
fn key_prefix(text: &str) -> Option<(&str, &str)> {
    if let Some(inner) = text.strip_prefix('[') {
        let len = key_len(inner)?;
        let rest = inner[len..].strip_prefix("] ")?;
        return Some((&inner[..len], rest.trim_start()));
    }
    let len = key_len(text)?;
    let rest = text[len..].strip_prefix(": ")?;
    Some((&text[..len], rest.trim_start()))
}

/// `text` without a leading prefix `KEY: ` or `[KEY] ` of `key`.
fn strip_own_key<'a>(text: &'a str, key: Option<&str>) -> &'a str {
    match (key, key_prefix(text)) {
        (Some(key), Some((found, rest))) if found == key => rest,
        _ => text,
    }
}

/// `title` without leading draft markers (`Draft: `, `[Draft] `, `(WIP) `),
/// compared without case.
fn strip_draft_markers(title: &str) -> &str {
    let mut text = title.trim_start();
    while let Some(rest) = DRAFT_MARKERS.iter().find_map(|marker| {
        text.get(..marker.len())
            .filter(|head| head.eq_ignore_ascii_case(marker))
            .map(|_| &text[marker.len()..])
    }) {
        text = rest.trim_start();
    }
    text
}

/// The part of a PR URL before `/pull/<n>` or `/-/merge_requests/<n>`: the
/// repository, for telling two PRs of one repository from the PRs of two.
fn repo_of_url(url: &str) -> &str {
    ["/-/merge_requests/", "/pull/"]
        .iter()
        .find_map(|marker| url.rfind(marker).map(|at| &url[..at]))
        .unwrap_or(url)
}

/// For each entry, the PR it is stacked on and its next action. The base
/// is looked up among the PRs of the same repository, from any checkout.
fn stack_and_actions(entries: &[PrEntry]) -> Vec<(Option<StackedOn>, Option<NextAction>)> {
    let mut by_repo: HashMap<&str, Vec<&PullRequest>> = HashMap::new();
    for entry in entries {
        by_repo
            .entry(repo_of_url(&entry.pr.url))
            .or_default()
            .push(&entry.pr);
    }
    entries
        .iter()
        .map(|entry| {
            let known = &by_repo[repo_of_url(&entry.pr.url)];
            let base = entry
                .pr
                .stacked_on(known.iter().copied(), entry.default_branch.as_deref());
            let stacked_on = base.map(|base| StackedOn {
                pr_ref: base.ref_text(),
                number: base.number,
                state: base.state,
            });
            (stacked_on, entry.pr.next_action(base))
        })
        .collect()
}

/// Head branches that make a shared-branch group: not one of
/// [`LONG_LIVED_BRANCHES`], and
/// the head of two or more listed PRs that have no ticket key
/// (`ticket_keys[i]` is the key of `entries[i]`). The same PR listed for two
/// checkouts counts once.
fn shared_branches(entries: &[PrEntry], ticket_keys: &[Option<String>]) -> HashSet<String> {
    let mut prs_by_head: HashMap<&str, HashSet<(&str, u64)>> = HashMap::new();
    for (entry, ticket_key) in entries.iter().zip(ticket_keys) {
        let head = entry.pr.head.as_str();
        if ticket_key.is_none() && !head.is_empty() && !LONG_LIVED_BRANCHES.contains(&head) {
            prs_by_head
                .entry(head)
                .or_default()
                .insert((repo_of_url(&entry.pr.url), entry.pr.number));
        }
    }
    prs_by_head
        .into_iter()
        .filter(|(_, prs)| prs.len() >= 2)
        .map(|(head, _)| head.to_string())
        .collect()
}

fn build_group(kind: GroupKind, key: String, mut rows: Vec<PrRow>) -> PrGroup {
    rows.sort_by(|a, b| {
        a.repo
            .cmp(&b.repo)
            .then(a.pr.number.cmp(&b.pr.number))
            .then_with(|| a.checkout.cmp(&b.checkout))
    });
    let rows = stacked_after_base(rows);
    let headline = headline(kind, &rows);
    PrGroup {
        key,
        kind,
        headline,
        rows,
    }
}

/// Moves each stacked row right after its base row, when the base is in
/// `rows`. Other rows keep their order. Rows that stack on each other in a
/// loop keep their order, after the rest.
fn stacked_after_base(rows: Vec<PrRow>) -> Vec<PrRow> {
    let parents: Vec<Option<usize>> = rows
        .iter()
        .map(|row| {
            let base = row.stacked_on.as_ref()?;
            let repo = repo_of_url(&row.pr.url);
            rows.iter().position(|other| {
                other.pr.number == base.number && repo_of_url(&other.pr.url) == repo
            })
        })
        .collect();
    let mut order = Vec::with_capacity(rows.len());
    let mut placed = vec![false; rows.len()];
    for start in (0..rows.len()).filter(|&row| parents[row].is_none()) {
        place_with_children(start, &parents, &mut placed, &mut order);
    }
    for start in 0..rows.len() {
        place_with_children(start, &parents, &mut placed, &mut order);
    }
    let mut rows: Vec<Option<PrRow>> = rows.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| rows[index].take())
        .collect()
}

/// Appends `row` to `order`, then the rows stacked on it, depth first.
fn place_with_children(
    row: usize,
    parents: &[Option<usize>],
    placed: &mut [bool],
    order: &mut Vec<usize>,
) {
    if std::mem::replace(&mut placed[row], true) {
        return;
    }
    order.push(row);
    for child in (0..parents.len()).filter(|&child| parents[child] == Some(row)) {
        place_with_children(child, parents, placed, order);
    }
}

/// The header text of a group made of `rows`, which are already ordered.
///
/// The next action is `fix CI` from any row, else `fix conflicts`, `address
/// review`, then `merge <base> first` or `merge` (one slot, so the first
/// such row in row order) from any row, in that priority; else the action of
/// the first open row. `AllMerged` counts distinct PRs: one PR seen from two
/// checkouts is one.
fn headline(kind: GroupKind, rows: &[PrRow]) -> GroupHeadline {
    if kind == GroupKind::Other {
        return GroupHeadline::Bare;
    }
    let open: Vec<(&PrRow, &NextAction)> = rows
        .iter()
        .filter_map(|row| Some((row, row.next_action.as_ref()?)))
        .collect();
    let needs_you = open
        .iter()
        .filter(|(_, action)| action.needs_you())
        .min_by_key(|(_, action)| needs_you_priority(action));
    match needs_you.or(open.first()) {
        Some((row, action)) => GroupHeadline::Next(NextStep {
            action: (*action).clone(),
            pr_ref: row.pr.ref_text(),
            state: row.pr.state,
        }),
        None => GroupHeadline::AllMerged(distinct_prs(rows)),
    }
}

/// Number of different PRs in `rows`: a PR listed for two checkouts counts
/// once.
fn distinct_prs(rows: &[PrRow]) -> usize {
    rows.iter()
        .map(|row| (repo_of_url(&row.pr.url), row.pr.number))
        .collect::<HashSet<_>>()
        .len()
}

/// Rank of an action the author must take; the lowest rank leads the
/// header. `merge <base> first` and `merge` share a rank, so the row order
/// decides between them.
fn needs_you_priority(action: &NextAction) -> u8 {
    match action {
        NextAction::FixCi => 0,
        NextAction::FixConflicts => 1,
        NextAction::AddressReview => 2,
        NextAction::MergeBaseFirst(_) | NextAction::Merge => 3,
        NextAction::Undraft | NextAction::WaitForCi | NextAction::GetReview => 4,
    }
}

/// Position of a group in the list: CI failed, needs the author, waiting,
/// finished, other.
fn group_rank(group: &PrGroup) -> u8 {
    match &group.headline {
        GroupHeadline::Next(step) if step.action == NextAction::FixCi => 0,
        GroupHeadline::Next(step) if step.needs_you() => 1,
        GroupHeadline::Next(_) => 2,
        GroupHeadline::AllMerged(_) => 3,
        GroupHeadline::Bare => 4,
    }
}

/// [`run_title_command`] with the time limit `limit`.
///
/// The command's process group sits in the exit-kill registry of git runs
/// ([`crate::git::TrackedGroup`]) while it runs, so quitting `ws` ends a
/// command that hangs. Once `ws` is exiting no command starts.
fn run_title_command_within(cmd: &Path, key: &str, limit: Duration) -> Option<String> {
    if crate::git::exiting() {
        return None;
    }
    let deadline = Instant::now() + limit;
    let mut child = spawn_title_command(cmd, key, deadline)?;
    let tracked = crate::git::TrackedGroup::register(child.id());
    // A stop that ran between the check above and the registration did
    // not see this group: end it here.
    if crate::git::exiting() {
        kill_and_reap(child);
        return None;
    }
    let Some(pipe) = child.stdout.take() else {
        kill_and_reap(child);
        return None;
    };
    let first_line = read_first_line(pipe);
    let Some(status) = wait_until(&mut child, deadline, &AtomicBool::new(false)) else {
        kill_and_reap(child);
        return None;
    };
    // Reaped: the group no longer needs a stop.
    drop(tracked);
    if !status.success() {
        return None;
    }
    // A grandchild that keeps the pipe open must not stall us either.
    let grace = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(200));
    title_from_line(&first_line.recv_timeout(grace).ok()?)
}

fn spawn_title_command(cmd: &Path, key: &str, deadline: Instant) -> Option<Child> {
    loop {
        let mut command = Command::new(cmd);
        command
            .arg(key)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Its own process group, so a timeout can end the command and
        // everything it started.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        match command.spawn() {
            Ok(child) => return Some(child),
            // In a multi-threaded process, a fork on another thread can hold
            // the script's write descriptor open until its own exec, so this
            // exec fails with ETXTBSY for a moment. Try again until the
            // deadline.
            Err(error)
                if error.kind() == ErrorKind::ExecutableFileBusy && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

/// Ends `child` and everything in its process group, then reaps it. The
/// group exists because [`spawn_title_command`] started the child with
/// `process_group(0)`. Elsewhere only the child is killed.
fn kill_and_reap(mut child: Child) {
    #[cfg(unix)]
    crate::git::signal_group(i32::try_from(child.id()).unwrap_or(0), libc::SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
}

/// Reads the first line of `pipe` (at most [`TITLE_LINE_MAX`] bytes) on its
/// own thread, then keeps reading and drops the rest, so a full pipe never
/// stalls the command and it never meets a closed pipe before it exits.
fn read_first_line(pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut line = Vec::new();
        let _ = (&mut reader)
            .take(TITLE_LINE_MAX)
            .read_until(b'\n', &mut line);
        let _ = tx.send(line);
        let _ = io::copy(&mut reader, &mut io::sink());
    });
    rx
}

/// The title in the first line of `bytes`: control characters as spaces,
/// trimmed. `None` when that line is empty.
fn title_from_line(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let title = text.lines().next()?.replace(char::is_control, " ");
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::pr_status::PrNotes;

    const APP: &str = "https://github.com/acme/app/pull";
    const MERGER: &str = "https://github.com/acme/merger/pull";
    const API: &str = "https://gitlab.com/acme/services/api/-/merge_requests";

    /// An entry for `number` of the repository behind `url_base`, in the
    /// checkout `/ws/<repo>`.
    fn entry(
        repo: &str,
        url_base: &str,
        number: u64,
        head: &str,
        title: &str,
        state: PrState,
    ) -> PrEntry {
        let mut pr = PullRequest::fixture(number, &format!("{url_base}/{number}"), state);
        pr.head = head.into();
        pr.title = title.into();
        PrEntry {
            checkout: PathBuf::from(format!("/ws/{repo}")),
            repo: repo.into(),
            pr,
            default_branch: None,
        }
    }

    fn app(number: u64, head: &str, title: &str, state: PrState) -> PrEntry {
        entry("acme/app", APP, number, head, title, state)
    }

    fn keys(groups: &[PrGroup]) -> Vec<&str> {
        groups.iter().map(|group| group.key.as_str()).collect()
    }

    fn numbers(group: &PrGroup) -> Vec<u64> {
        group.rows.iter().map(|row| row.pr.number).collect()
    }

    fn header(group: &PrGroup) -> String {
        match &group.headline {
            GroupHeadline::Next(step) => step.text(),
            other => panic!("not an open group: {other:?}"),
        }
    }

    // ---- key ladder -------------------------------------------------

    #[test]
    fn key_ladder_branch_rule() {
        for (branch, key) in [
            ("feature/ABC-123-retry", Some("ABC-123")),
            ("ABC-123", Some("ABC-123")),
            ("ABC-123_retry", Some("ABC-123")),
            ("ABC-123.retry", Some("ABC-123")),
            ("feature/ABC-123", Some("ABC-123")),
            ("bugfix/ABC-123/retry", Some("ABC-123")),
            ("user/ABC-123/ABC-125-x", Some("ABC-123")),
            ("feature/A1-2-x", Some("A1-2")),
            ("feature/ABCDEFGHIJ-1-x", Some("ABCDEFGHIJ-1")),
            // The key is not at the start of its segment.
            ("feature/fix-ABC-123", None),
            ("feature/xABC-123", None),
            // The key does not end at a boundary.
            ("feature/ABC-123x", None),
            ("feature/ABC-123+1", None),
            // Wrong shape.
            ("feature/abc-123-retry", None),
            ("feature/ABC-retry", None),
            ("feature/A-1", None),
            ("feature/ABCDEFGHIJK-1", None),
            ("main", None),
            ("", None),
        ] {
            assert_eq!(branch_key(branch), key, "branch {branch:?}");
        }
    }

    #[test]
    fn standard_prefixes_never_count_as_keys() {
        for letters in NOT_KEYS {
            assert_eq!(
                ticket_key(&format!("fix/{letters}-2024-1234"), ""),
                None,
                "{letters} in a branch"
            );
            assert_eq!(
                ticket_key("topic", &format!("fix({letters}-1): x")),
                None,
                "{letters} in a scope"
            );
            assert_eq!(
                ticket_key("topic", &format!("{letters}-1: x")),
                None,
                "{letters} as a prefix"
            );
            assert_eq!(
                ticket_key("topic", &format!("[{letters}-1] x")),
                None,
                "{letters} in brackets"
            );
        }
        assert_eq!(ticket_key("fix/CVE-2024-1234", "fix: patch"), None);
        assert_eq!(ticket_key("topic", "UTF-8: handle it"), None);
        assert_eq!(ticket_key("topic", "chore(deps): bump SHA-256 dep"), None);
        // A longer name that only starts with the letters is a key.
        assert_eq!(
            ticket_key("feature/CVEX-1-x", ""),
            Some("CVEX-1".to_string())
        );
        assert_eq!(ticket_key("feature/MDX-7", ""), Some("MDX-7".to_string()));
    }

    #[test]
    fn key_ladder_commit_scope_rule() {
        for (title, key) in [
            ("feat(ABC-123): add retry", Some("ABC-123")),
            ("fix(api,ABC-123): add retry", Some("ABC-123")),
            ("fix(api, ABC-123): add retry", Some("ABC-123")),
            ("fix(api/ABC-123): add retry", Some("ABC-123")),
            ("fix(ABC-125,ABC-123): add retry", Some("ABC-125")),
            ("FIX(ABC-123)!: add retry", Some("ABC-123")),
            ("feat(ABC-123) add retry", None),
            ("feat(api): add retry", None),
            ("feat(api-ABC-123): add retry", None),
            ("feat(ABC-123x): add retry", None),
            ("feat(): add retry", None),
            ("improve(ABC-123): add retry", None),
            ("(ABC-123): add retry", None),
        ] {
            assert_eq!(
                ticket_key("topic", title),
                key.map(str::to_string),
                "title {title:?}"
            );
        }
    }

    #[test]
    fn key_ladder_title_prefix_rule() {
        for (title, key) in [
            ("ABC-123: add retry", Some("ABC-123")),
            ("[ABC-123] add retry", Some("ABC-123")),
            ("feat: ABC-123: add retry", Some("ABC-123")),
            ("feat: [ABC-123] add retry", Some("ABC-123")),
            ("fix(api): ABC-123: add retry", Some("ABC-123")),
            ("Draft: ABC-123: add retry", Some("ABC-123")),
            ("[Draft] [ABC-123] add retry", Some("ABC-123")),
            ("WIP: feat(ABC-123): add retry", Some("ABC-123")),
            ("ABC-123 add retry", None),
            ("ABC-123:add retry", None),
            ("[ABC-123]add retry", None),
            ("add retry for ABC-123", None),
            ("Reopen ABC-123: add retry", None),
        ] {
            assert_eq!(
                ticket_key("topic", title),
                key.map(str::to_string),
                "title {title:?}"
            );
        }
    }

    #[test]
    fn key_ladder_first_rule_wins() {
        // Branch beats scope, scope beats prefix.
        assert_eq!(
            ticket_key("feature/ABC-123-x", "fix(ABC-125): y"),
            Some("ABC-123".to_string())
        );
        assert_eq!(
            ticket_key("topic", "fix(ABC-125): ABC-123: y"),
            Some("ABC-125".to_string())
        );
        // A branch that holds no key falls through to the title.
        assert_eq!(
            ticket_key("feature/fix-ABC-123", "ABC-125: y"),
            Some("ABC-125".to_string())
        );
        // The first segment with a key wins.
        assert_eq!(
            ticket_key("ABC-125/ABC-123-x", ""),
            Some("ABC-125".to_string())
        );
    }

    // ---- title cleanup ----------------------------------------------

    #[test]
    fn clean_title_strips_draft_marker_type_and_own_key() {
        let key = Some("ABC-123");
        for (title, cleaned) in [
            ("feat(ABC-123): add retry", "add retry"),
            ("feat: ABC-123: add retry", "add retry"),
            ("feat: [ABC-123] add retry", "add retry"),
            ("ABC-123: add retry", "add retry"),
            ("[ABC-123] add retry", "add retry"),
            ("[ABC-123] feat: add retry", "add retry"),
            ("Draft: feat(ABC-123): add retry", "add retry"),
            ("[Draft] [WIP] fix: add retry", "add retry"),
            ("wip: ABC-123: add retry", "add retry"),
            ("(DRAFT) ABC-123: add retry", "add retry"),
            ("  add retry  ", "add retry"),
            ("add retry", "add retry"),
        ] {
            assert_eq!(clean_title(title, key), cleaned, "title {title:?}");
        }
    }

    #[test]
    fn clean_title_keeps_an_area_scope_as_scope_colon_description() {
        let key = Some("ABC-123");
        for (title, cleaned) in [
            ("fix(api): add retry", "api: add retry"),
            ("fix(api,ABC-123): add retry", "api: add retry"),
            ("fix(ABC-123,api): add retry", "api: add retry"),
            ("fix(api/ABC-123/ui): add retry", "api,ui: add retry"),
            ("fix(api/v2): add retry", "api/v2: add retry"),
            ("fix(api,ABC-125): add retry", "api,ABC-125: add retry"),
            ("fix(ABC-125): add retry", "ABC-125: add retry"),
            ("feat(api)!: ABC-123: add retry", "api: add retry"),
        ] {
            assert_eq!(clean_title(title, key), cleaned, "title {title:?}");
        }
    }

    #[test]
    fn clean_title_without_a_ticket_key_keeps_other_keys_and_the_scope() {
        assert_eq!(
            clean_title("fix(ABC-123): add retry", None),
            "ABC-123: add retry"
        );
        assert_eq!(
            clean_title("ABC-123: add retry", None),
            "ABC-123: add retry"
        );
        assert_eq!(clean_title("Draft: feat: add retry", None), "add retry");
        // Another key is not the group's own key.
        assert_eq!(
            clean_title("ABC-125: add retry", Some("ABC-123")),
            "ABC-125: add retry"
        );
    }

    #[test]
    fn clean_title_falls_back_to_the_full_title() {
        let key = Some("ABC-123");
        assert_eq!(clean_title("feat(ABC-123):", key), "feat(ABC-123):");
        assert_eq!(clean_title("ABC-123: ", key), "ABC-123:");
        assert_eq!(clean_title("Draft: ", key), "Draft:");
        assert_eq!(clean_title("", key), "");
        assert_eq!(clean_title("   ", key), "");
    }

    #[test]
    fn clean_title_only_strips_a_known_commit_type() {
        assert_eq!(clean_title("Note: add retry", None), "Note: add retry");
        assert_eq!(clean_title("Fix: add retry", None), "add retry");
        assert_eq!(
            clean_title("docs(readme): add retry", None),
            "readme: add retry"
        );
    }

    // ---- grouping ---------------------------------------------------

    #[test]
    fn groups_are_made_by_key_shared_branch_and_other() {
        let groups = group_prs(vec![
            app(1, "feature/ABC-123-retry", "add retry", PrState::InReview),
            entry(
                "acme/merger",
                MERGER,
                2,
                "topic",
                "fix(ABC-123): x",
                PrState::InReview,
            ),
            app(3, "shared-topic", "one", PrState::InReview),
            entry(
                "acme/merger",
                MERGER,
                4,
                "shared-topic",
                "two",
                PrState::InReview,
            ),
            app(5, "lonely", "keyless", PrState::InReview),
            app(
                6,
                "develop",
                "keyless on a common branch",
                PrState::InReview,
            ),
        ]);
        let by_kind: Vec<(&str, GroupKind, Vec<u64>)> = groups
            .iter()
            .map(|group| (group.key.as_str(), group.kind, numbers(group)))
            .collect();
        assert_eq!(
            by_kind,
            vec![
                ("ABC-123", GroupKind::Ticket, vec![1, 2]),
                ("shared-topic", GroupKind::SharedBranch, vec![3, 4]),
                ("other", GroupKind::Other, vec![5, 6]),
            ]
        );
    }

    #[test]
    fn a_shared_branch_needs_two_listed_prs_and_a_branch_that_is_not_common() {
        for branch in LONG_LIVED_BRANCHES {
            let groups = group_prs(vec![
                app(1, branch, "one", PrState::InReview),
                entry("acme/merger", MERGER, 2, branch, "two", PrState::InReview),
            ]);
            assert_eq!(keys(&groups), vec!["other"], "branch {branch}");
        }
        // One PR alone.
        let groups = group_prs(vec![app(1, "topic", "one", PrState::InReview)]);
        assert_eq!(keys(&groups), vec!["other"]);
        // No head branch from the forge.
        let groups = group_prs(vec![
            app(1, "", "one", PrState::InReview),
            entry("acme/merger", MERGER, 2, "", "two", PrState::InReview),
        ]);
        assert_eq!(keys(&groups), vec!["other"]);
    }

    #[test]
    fn one_pr_listed_for_two_checkouts_is_not_a_shared_branch() {
        let mut worktree = app(1, "topic", "one", PrState::InReview);
        worktree.checkout = PathBuf::from("/ws/app-topic");
        let groups = group_prs(vec![app(1, "topic", "one", PrState::InReview), worktree]);
        assert_eq!(keys(&groups), vec!["other"]);
        assert_eq!(numbers(&groups[0]), vec![1, 1], "both checkouts are rows");
    }

    #[test]
    fn a_shared_branch_spans_repositories_and_forges() {
        let groups = group_prs(vec![
            app(7, "topic", "one", PrState::InReview),
            entry(
                "acme/services/api",
                API,
                12,
                "topic",
                "two",
                PrState::InReview,
            ),
        ]);
        assert_eq!(keys(&groups), vec!["topic"]);
        assert_eq!(groups[0].kind, GroupKind::SharedBranch);
        assert_eq!(groups[0].rows.len(), 2);
    }

    #[test]
    fn a_keyed_pr_does_not_count_toward_a_shared_branch() {
        // Same head branch, but the title of one names a key: the other has
        // no partner, so it is not a shared-branch group of one row.
        let groups = group_prs(vec![
            app(1, "topic", "ABC-123: one", PrState::InReview),
            entry("acme/merger", MERGER, 2, "topic", "two", PrState::InReview),
        ]);
        assert_eq!(keys(&groups), vec!["ABC-123", "other"]);
        // With a second keyless PR on the branch, they do share it.
        let groups = group_prs(vec![
            app(1, "topic", "ABC-123: one", PrState::InReview),
            entry("acme/merger", MERGER, 2, "topic", "two", PrState::InReview),
            entry(
                "acme/merger",
                MERGER,
                3,
                "topic",
                "three",
                PrState::InReview,
            ),
        ]);
        assert_eq!(keys(&groups), vec!["ABC-123", "topic"]);
        assert_eq!(numbers(&groups[1]), vec![2, 3]);
    }

    #[test]
    fn a_group_named_other_by_branch_stays_apart_from_the_other_group() {
        let groups = group_prs(vec![
            app(1, "other", "one", PrState::InReview),
            entry("acme/merger", MERGER, 2, "other", "two", PrState::InReview),
            app(3, "lonely", "three", PrState::InReview),
        ]);
        let kinds: Vec<(&str, GroupKind)> = groups
            .iter()
            .map(|group| (group.key.as_str(), group.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("other", GroupKind::SharedBranch),
                ("other", GroupKind::Other)
            ]
        );
    }

    #[test]
    fn rows_carry_the_cleaned_title_per_group_kind() {
        let groups = group_prs(vec![
            app(
                1,
                "feature/ABC-123-x",
                "feat(ABC-123): add retry",
                PrState::InReview,
            ),
            app(2, "topic", "Draft: fix(api): one", PrState::Draft),
            entry(
                "acme/merger",
                MERGER,
                3,
                "topic",
                "ABC-5 two",
                PrState::Draft,
            ),
            app(4, "solo", "chore: tidy", PrState::InReview),
        ]);
        let mut titles = Vec::new();
        for group in &groups {
            for row in &group.rows {
                titles.push((group.key.as_str(), row.title.as_str()));
            }
        }
        assert_eq!(
            titles,
            vec![
                ("ABC-123", "add retry"),
                ("topic", "api: one"),
                ("topic", "ABC-5 two"),
                ("other", "tidy"),
            ]
        );
    }

    // ---- next action of a group -------------------------------------

    fn with_notes(mut entry: PrEntry, notes: PrNotes) -> PrEntry {
        entry.pr.notes = notes;
        entry
    }

    #[test]
    fn header_text_is_the_action_and_the_pr_ref() {
        for (state, number, url, text) in [
            (PrState::CiFailed, 318, APP, "fix CI #318"),
            (PrState::Ready, 319, APP, "merge #319"),
            (PrState::Ready, 12, API, "merge !12"),
            (PrState::CiRunning, 320, APP, "wait for CI #320"),
            (PrState::InReview, 321, APP, "get review #321"),
            (PrState::Draft, 322, APP, "undraft #322"),
        ] {
            let groups = group_prs(vec![entry(
                "acme/app",
                url,
                number,
                "feature/ABC-123-x",
                "t",
                state,
            )]);
            assert_eq!(header(&groups[0]), text, "{state:?}");
        }
        let groups = group_prs(vec![with_notes(
            app(7, "feature/ABC-123-x", "t", PrState::InReview),
            PrNotes {
                conflicts: true,
                ..Default::default()
            },
        )]);
        assert_eq!(header(&groups[0]), "fix conflicts #7");
        let groups = group_prs(vec![with_notes(
            app(8, "feature/ABC-123-x", "t", PrState::InReview),
            PrNotes {
                changes_requested: true,
                ..Default::default()
            },
        )]);
        assert_eq!(header(&groups[0]), "address review #8");
    }

    #[test]
    fn header_merge_base_first_names_the_base_ref_once() {
        let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
        child.pr.base = "feature/ABC-123-a".into();
        let base = app(318, "feature/ABC-123-a", "t", PrState::InReview);
        let groups = group_prs(vec![child, base]);
        let GroupHeadline::Next(step) = &groups[0].headline else {
            panic!("open group");
        };
        assert_eq!(step.text(), "merge #318 first");
        assert_eq!(step.pr_ref, "#319", "it targets the stacked PR");
        assert!(step.needs_you());
    }

    #[test]
    fn header_next_action_priority_is_fix_ci_then_conflicts_then_review_then_merge() {
        let conflicts = PrNotes {
            conflicts: true,
            ..Default::default()
        };
        let review = PrNotes {
            changes_requested: true,
            ..Default::default()
        };
        // Row order is 1, 2, 3, 4, 5; the lowest priority number leads.
        let rows = |states: [(PrState, PrNotes); 5]| {
            let entries = states
                .into_iter()
                .enumerate()
                .map(|(index, (state, notes))| {
                    with_notes(
                        app(index as u64 + 1, "feature/ABC-123-x", "t", state),
                        notes,
                    )
                })
                .collect();
            group_prs(entries)
        };
        let none = PrNotes::default();
        let groups = rows([
            (PrState::Ready, none),
            (PrState::InReview, review),
            (PrState::InReview, conflicts),
            (PrState::Draft, none),
            (PrState::CiFailed, none),
        ]);
        assert_eq!(
            header(&groups[0]),
            "fix CI #5",
            "CI failed from a later row"
        );
        let groups = rows([
            (PrState::Ready, none),
            (PrState::InReview, review),
            (PrState::InReview, conflicts),
            (PrState::Draft, none),
            (PrState::CiRunning, none),
        ]);
        assert_eq!(header(&groups[0]), "fix conflicts #3");
        let groups = rows([
            (PrState::Ready, none),
            (PrState::InReview, review),
            (PrState::Draft, none),
            (PrState::CiRunning, none),
            (PrState::InReview, none),
        ]);
        assert_eq!(header(&groups[0]), "address review #2");
        let groups = rows([
            (PrState::Draft, none),
            (PrState::Ready, none),
            (PrState::CiRunning, none),
            (PrState::Ready, none),
            (PrState::InReview, none),
        ]);
        assert_eq!(
            header(&groups[0]),
            "merge #2",
            "first ready row in row order"
        );
    }

    #[test]
    fn header_merge_and_merge_base_first_share_one_slot_in_row_order() {
        // The base sorts first, so the header targets the base: `merge #318`,
        // not the child's `merge #318 first`.
        let base = app(318, "feature/ABC-123-a", "t", PrState::Ready);
        let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
        child.pr.base = "feature/ABC-123-a".into();
        let groups = group_prs(vec![child.clone(), base]);
        assert_eq!(numbers(&groups[0]), vec![318, 319]);
        assert_eq!(header(&groups[0]), "merge #318");
        let GroupHeadline::Next(step) = &groups[0].headline else {
            panic!("open group");
        };
        assert_eq!(step.pr_ref, "#318", "it targets the base");
        // With a base that is not ready, the child's action is the one.
        let base = app(318, "feature/ABC-123-a", "t", PrState::InReview);
        let groups = group_prs(vec![child, base]);
        assert_eq!(header(&groups[0]), "merge #318 first");
    }

    #[test]
    fn header_without_a_needs_you_row_is_the_first_open_rows_action() {
        let groups = group_prs(vec![
            app(1, "feature/ABC-123-x", "t", PrState::Merged),
            app(2, "feature/ABC-123-x", "t", PrState::CiRunning),
            app(3, "feature/ABC-123-x", "t", PrState::Draft),
            app(4, "feature/ABC-123-x", "t", PrState::InReview),
        ]);
        assert_eq!(header(&groups[0]), "wait for CI #2");
        let GroupHeadline::Next(step) = &groups[0].headline else {
            panic!("open group");
        };
        assert_eq!(step.state, PrState::CiRunning, "the state of the target");
        assert!(!step.needs_you());
    }

    #[test]
    fn a_group_of_merged_prs_is_finished_and_counts_them() {
        let groups = group_prs(vec![
            app(1, "feature/ABC-123-x", "t", PrState::Merged),
            app(2, "feature/ABC-123-y", "t", PrState::Merged),
        ]);
        assert_eq!(groups[0].headline, GroupHeadline::AllMerged(2));
        assert!(groups[0].is_finished());
        // One open row makes it an open group; merged rows are not targets.
        let groups = group_prs(vec![
            app(1, "feature/ABC-123-x", "t", PrState::Merged),
            app(2, "feature/ABC-123-y", "t", PrState::InReview),
        ]);
        assert!(!groups[0].is_finished());
        assert_eq!(header(&groups[0]), "get review #2");
    }

    #[test]
    fn all_merged_counts_distinct_prs_not_checkouts() {
        // #1 is listed for two checkouts: one PR.
        let mut worktree = app(1, "feature/ABC-123-x", "t", PrState::Merged);
        worktree.checkout = PathBuf::from("/ws/app-wt");
        let groups = group_prs(vec![
            app(1, "feature/ABC-123-x", "t", PrState::Merged),
            worktree,
            app(2, "feature/ABC-123-y", "t", PrState::Merged),
        ]);
        assert_eq!(groups[0].rows.len(), 3, "every checkout is a row");
        assert_eq!(groups[0].headline, GroupHeadline::AllMerged(2));
        // The same number in another repository is another PR.
        let groups = group_prs(vec![
            app(1, "feature/ABC-123-x", "t", PrState::Merged),
            entry(
                "acme/merger",
                MERGER,
                1,
                "feature/ABC-123-x",
                "t",
                PrState::Merged,
            ),
        ]);
        assert_eq!(groups[0].headline, GroupHeadline::AllMerged(2));
    }

    #[test]
    fn the_other_group_has_no_header_text_and_is_never_finished() {
        let groups = group_prs(vec![app(1, "solo", "t", PrState::Merged)]);
        assert_eq!(groups[0].kind, GroupKind::Other);
        assert_eq!(groups[0].headline, GroupHeadline::Bare);
        assert!(!groups[0].is_finished());
    }

    // ---- order ------------------------------------------------------

    #[test]
    fn groups_sort_ci_failed_then_needs_you_then_waiting_then_finished_then_other() {
        let groups = group_prs(vec![
            // other, merged
            app(1, "solo", "t", PrState::Merged),
            // finished
            app(2, "feature/ABC-100-a", "t", PrState::Merged),
            // waiting
            app(3, "feature/ABC-101-a", "t", PrState::InReview),
            // needs you: merge
            app(4, "feature/ABC-102-a", "t", PrState::Ready),
            // CI failed
            app(5, "feature/ABC-103-a", "t", PrState::CiFailed),
            // needs you: a later key than the first needs-you group
            app(6, "feature/ABC-099-a", "t", PrState::Ready),
            // CI failed in one row of a group with a ready row
            app(7, "feature/ABC-104-a", "t", PrState::Ready),
            app(8, "feature/ABC-104-b", "t", PrState::CiFailed),
            // waiting, earlier key
            app(9, "feature/ABC-098-a", "t", PrState::Draft),
        ]);
        assert_eq!(
            keys(&groups),
            vec![
                "ABC-103", "ABC-104", "ABC-099", "ABC-102", "ABC-098", "ABC-101", "ABC-100",
                "other"
            ]
        );
    }

    #[test]
    fn groups_with_the_same_rank_sort_by_key_ascending() {
        let groups = group_prs(vec![
            app(1, "feature/ABC-125-a", "t", PrState::Ready),
            app(2, "feature/ABC-123-a", "t", PrState::Ready),
            app(3, "shared", "t", PrState::Ready),
            entry("acme/merger", MERGER, 4, "shared", "t", PrState::Ready),
        ]);
        assert_eq!(keys(&groups), vec!["ABC-123", "ABC-125", "shared"]);
    }

    #[test]
    fn group_keys_sort_in_natural_order_not_text_order() {
        // Not zero-padded, so a text sort would put ABC-1000 and ABC-100
        // before ABC-9.
        let groups = group_prs(vec![
            app(1, "feature/ABC-1000-a", "t", PrState::InReview),
            app(2, "feature/ABC-999-a", "t", PrState::InReview),
            app(3, "feature/ABC-10-a", "t", PrState::InReview),
            app(4, "feature/ABC-9-a", "t", PrState::InReview),
            app(5, "feature/ABC-100-a", "t", PrState::InReview),
            app(6, "feature/AB-5-a", "t", PrState::InReview),
            app(7, "feature/ABD-1-a", "t", PrState::InReview),
        ]);
        assert_eq!(
            keys(&groups),
            vec!["AB-5", "ABC-9", "ABC-10", "ABC-100", "ABC-999", "ABC-1000", "ABD-1"]
        );
        // The same inside one rank: a CI failed group still leads.
        let groups = group_prs(vec![
            app(1, "feature/ABC-1000-a", "t", PrState::CiFailed),
            app(2, "feature/ABC-999-a", "t", PrState::CiFailed),
            app(3, "feature/ABC-20-a", "t", PrState::Ready),
        ]);
        assert_eq!(keys(&groups), vec!["ABC-999", "ABC-1000", "ABC-20"]);
    }

    #[test]
    fn natural_key_order_compares_letters_then_number_then_rest() {
        use std::cmp::Ordering::{Equal, Greater, Less};
        for (a, b, order) in [
            ("ABC-999", "ABC-1000", Less),
            ("ABC-9", "ABC-10", Less),
            ("ABC-10", "ABC-9", Greater),
            ("AB-100", "ABC-1", Less),
            ("ABC-007", "ABC-8", Less),
            ("ABC-7", "ABC-7", Equal),
            // The same number: the whole key decides, so the order is total.
            ("ABC-007", "ABC-7", Less),
            // A rest after the number is compared as text.
            ("topic-2-a", "topic-2-b", Less),
            ("topic-2-b", "topic-10-a", Less),
            // No number: the key is text, before the same text with one.
            ("shared", "shared-1", Less),
            ("my-topic", "my-topic-3", Less),
            ("my-topic-3", "my-topic-12", Less),
            // A digit run longer than any integer type still compares.
            (
                "x-99999999999999999999999",
                "x-100000000000000000000000",
                Less,
            ),
        ] {
            assert_eq!(natural_key_cmp(a, b), order, "{a} vs {b}");
            assert_eq!(natural_key_cmp(b, a), order.reverse(), "{b} vs {a}");
        }
    }

    #[test]
    fn rows_sort_by_repo_then_number() {
        let groups = group_prs(vec![
            entry(
                "acme/merger",
                MERGER,
                1,
                "feature/ABC-123-a",
                "t",
                PrState::InReview,
            ),
            app(30, "feature/ABC-123-b", "t", PrState::InReview),
            app(4, "feature/ABC-123-c", "t", PrState::InReview),
            entry(
                "acme/services/api",
                API,
                2,
                "feature/ABC-123-d",
                "t",
                PrState::InReview,
            ),
        ]);
        let rows: Vec<(&str, u64)> = groups[0]
            .rows
            .iter()
            .map(|row| (row.repo.as_str(), row.pr.number))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("acme/app", 4),
                ("acme/app", 30),
                ("acme/merger", 1),
                ("acme/services/api", 2)
            ]
        );
    }

    #[test]
    fn a_stacked_row_sorts_right_after_its_base() {
        // #10 is stacked on #12, #12 on #11.
        let mut ten = app(10, "feature/ABC-123-c", "t", PrState::InReview);
        ten.pr.base = "feature/ABC-123-b".into();
        let mut twelve = app(12, "feature/ABC-123-b", "t", PrState::InReview);
        twelve.pr.base = "feature/ABC-123-a".into();
        let eleven = app(11, "feature/ABC-123-a", "t", PrState::InReview);
        let plain = app(13, "feature/ABC-123-x", "t", PrState::InReview);
        let groups = group_prs(vec![ten, twelve, eleven, plain]);
        assert_eq!(numbers(&groups[0]), vec![11, 12, 10, 13]);
        let stacked: Vec<Option<&str>> = groups[0]
            .rows
            .iter()
            .map(|row| row.stacked_on.as_ref().map(|base| base.pr_ref.as_str()))
            .collect();
        assert_eq!(stacked, vec![None, Some("#11"), Some("#12"), None]);
    }

    #[test]
    fn a_stack_across_groups_keeps_the_row_order_and_the_stack_mark() {
        // The base is in another group (its title names another key).
        let base = app(318, "topic-a", "ABC-125: base", PrState::CiFailed);
        let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
        child.pr.base = "topic-a".into();
        let groups = group_prs(vec![base, child]);
        let child_group = groups.iter().find(|group| group.key == "ABC-123").unwrap();
        let row = &child_group.rows[0];
        assert_eq!(
            row.stacked_on,
            Some(StackedOn {
                pr_ref: "#318".into(),
                number: 318,
                state: PrState::CiFailed
            })
        );
        assert_eq!(
            row.next_action,
            Some(NextAction::MergeBaseFirst("#318".into()))
        );
    }

    #[test]
    fn a_stack_is_found_in_the_same_repository_only() {
        let base = entry(
            "acme/merger",
            MERGER,
            318,
            "feature/ABC-123-a",
            "t",
            PrState::InReview,
        );
        let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
        child.pr.base = "feature/ABC-123-a".into();
        let groups = group_prs(vec![base, child]);
        let child_row = groups[0]
            .rows
            .iter()
            .find(|row| row.pr.number == 319)
            .unwrap();
        assert_eq!(child_row.stacked_on, None);
        assert_eq!(child_row.next_action, Some(NextAction::Merge));
    }

    #[test]
    fn a_merged_base_keeps_the_stack_mark_but_not_merge_first() {
        let base = app(318, "feature/ABC-123-a", "t", PrState::Merged);
        let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
        child.pr.base = "feature/ABC-123-a".into();
        let groups = group_prs(vec![base, child]);
        let row = groups[0]
            .rows
            .iter()
            .find(|row| row.pr.number == 319)
            .unwrap();
        assert_eq!(row.stacked_on.as_ref().unwrap().state, PrState::Merged);
        assert_eq!(row.next_action, Some(NextAction::Merge));
        assert_eq!(numbers(&groups[0]), vec![318, 319]);
    }

    #[test]
    fn the_configured_default_branch_is_never_a_stack_base() {
        // `release` is open as a PR into `main`; `#11` targets `release`.
        let release = app(10, "release", "ABC-123: release", PrState::InReview);
        let mut child = app(11, "feature/ABC-123-a", "t", PrState::Ready);
        child.pr.base = "release".into();
        let child_row = |default_branch: Option<&str>| {
            let mut child = child.clone();
            child.default_branch = default_branch.map(str::to_string);
            let groups = group_prs(vec![release.clone(), child]);
            assert_eq!(numbers(&groups[0]), vec![10, 11]);
            let row = groups[0].rows[1].clone();
            (header(&groups[0]), row)
        };

        // Not configured and not a long-lived name: `release` is a base.
        let (header, row) = child_row(None);
        assert_eq!(header, "merge #10 first");
        assert_eq!(row.stacked_on.map(|base| base.pr_ref), Some("#10".into()));
        assert_eq!(
            row.next_action,
            Some(NextAction::MergeBaseFirst("#10".into()))
        );

        // Configured as the default branch: no stack; rows and header say so.
        let (header, row) = child_row(Some("release"));
        assert_eq!(header, "merge #11");
        assert_eq!(row.stacked_on, None);
        assert_eq!(row.next_action, Some(NextAction::Merge));

        // Another default branch changes nothing for this base.
        let (_, row) = child_row(Some("main"));
        assert!(row.stacked_on.is_some());
    }

    #[test]
    fn a_pr_into_a_long_lived_branch_is_not_stacked_on_a_pr_from_it() {
        // `develop → main` is open; `#11` targets `develop`.
        let develop = app(10, "develop", "ABC-123: promote", PrState::InReview);
        let mut child = app(11, "feature/ABC-123-a", "t", PrState::Ready);
        child.pr.base = "develop".into();
        for default_branch in [None, Some("develop".to_string()), Some("main".to_string())] {
            let mut child = child.clone();
            child.default_branch = default_branch.clone();
            let groups = group_prs(vec![develop.clone(), child]);
            let row = groups[0]
                .rows
                .iter()
                .find(|row| row.pr.number == 11)
                .unwrap();
            assert_eq!(row.stacked_on, None, "default {default_branch:?}");
            assert_eq!(row.next_action, Some(NextAction::Merge));
            assert_eq!(header(&groups[0]), "merge #11");
        }
    }

    #[test]
    fn rows_that_stack_on_each_other_in_a_loop_still_all_show() {
        let mut a = app(1, "branch-a", "ABC-123: a", PrState::InReview);
        a.pr.base = "branch-b".into();
        let mut b = app(2, "branch-b", "ABC-123: b", PrState::InReview);
        b.pr.base = "branch-a".into();
        let groups = group_prs(vec![a, b]);
        assert_eq!(numbers(&groups[0]), vec![1, 2]);
    }

    #[test]
    fn listed_counts_every_row_and_the_fold_default_starts_above_twelve() {
        let make = |count: u64| {
            let mut entries: Vec<PrEntry> = (1..=count)
                .map(|number| app(number, "feature/ABC-123-x", "t", PrState::Merged))
                .collect();
            entries.push(app(1000, "solo", "t", PrState::InReview));
            group_prs(entries)
        };
        let twelve = make(11);
        assert_eq!(listed(&twelve), 12);
        assert!(!finished_start_folded(&twelve));
        let thirteen = make(12);
        assert_eq!(listed(&thirteen), 13);
        assert!(finished_start_folded(&thirteen));
        assert_eq!(listed(&[]), 0);
        assert!(!finished_start_folded(&[]));
    }

    // ---- title command ----------------------------------------------

    #[cfg(unix)]
    mod title_command {
        use super::*;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        /// A temp dir with one executable `script` that holds `body`.
        struct Script {
            dir: PathBuf,
            path: PathBuf,
        }

        impl Script {
            fn new(body: &str) -> Self {
                let dir = crate::testutil::unique_dir("ws-pr-title");
                fs::create_dir_all(&dir).unwrap();
                let path = dir.join("script");
                fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
                Self { dir, path }
            }
        }

        impl Drop for Script {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }

        fn run(body: &str, key: &str) -> Option<String> {
            run_title_command(&Script::new(body).path, key)
        }

        #[test]
        fn the_first_stdout_line_is_the_title() {
            assert_eq!(
                run("echo \"Retry the upload ($1)\"", "ABC-123"),
                Some("Retry the upload (ABC-123)".to_string())
            );
            assert_eq!(
                run("printf 'First\\nSecond\\n'", "ABC-123"),
                Some("First".to_string())
            );
            assert_eq!(
                run("printf '  padded \\r\\n'", "ABC-123"),
                Some("padded".to_string())
            );
            assert_eq!(
                run("printf 'No newline'", "ABC-123"),
                Some("No newline".to_string())
            );
        }

        #[test]
        fn the_key_is_the_only_argument_and_no_shell_reads_it() {
            let body = "printf '%s|%s' \"$#\" \"$1\"";
            assert_eq!(run(body, "ABC-123"), Some("1|ABC-123".to_string()));
            assert_eq!(
                run(body, "$(echo hi);ls"),
                Some("1|$(echo hi);ls".to_string())
            );
        }

        #[test]
        fn control_characters_become_spaces() {
            assert_eq!(
                run("printf 'A\\033[1mB\\tC\\n'", "ABC-123"),
                Some("A [1mB C".to_string())
            );
        }

        #[test]
        fn a_non_zero_exit_gives_no_title_even_with_output() {
            assert_eq!(run("echo Title; exit 1", "ABC-123"), None);
            assert_eq!(run("echo Title; exit 127", "ABC-123"), None);
        }

        #[test]
        fn empty_output_gives_no_title() {
            assert_eq!(run("exit 0", "ABC-123"), None);
            assert_eq!(run("echo", "ABC-123"), None);
            assert_eq!(run("echo '   '", "ABC-123"), None);
            // Only the first line counts, so an empty one is no title.
            assert_eq!(run("printf '\\nSecond\\n'", "ABC-123"), None);
        }

        #[test]
        fn stderr_is_ignored() {
            assert_eq!(
                run("echo warning >&2; echo Title", "ABC-123"),
                Some("Title".to_string())
            );
            assert_eq!(run("echo Title >&2", "ABC-123"), None);
        }

        #[test]
        fn a_missing_or_non_executable_file_gives_no_title() {
            let script = Script::new("echo Title");
            assert_eq!(
                run_title_command(&script.dir.join("no-such-file"), "ABC-123"),
                None
            );
            fs::set_permissions(&script.path, fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(run_title_command(&script.path, "ABC-123"), None);
            assert_eq!(run_title_command(&script.dir, "ABC-123"), None);
        }

        #[test]
        fn a_command_that_runs_too_long_is_killed_and_gives_no_title() {
            let script = Script::new("echo Title; exec sleep 5");
            let started = Instant::now();
            let title =
                run_title_command_within(&script.path, "ABC-123", Duration::from_millis(300));
            assert_eq!(title, None);
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "stopped at the limit, took {:?}",
                started.elapsed()
            );
        }

        /// True while `pid` exists and is not a zombie: a killed orphan stays
        /// a zombie until init reaps it, and init may never reap in a
        /// container.
        fn process_runs(pid: libc::pid_t) -> bool {
            // SAFETY: signal 0 only checks that the process exists.
            if unsafe { libc::kill(pid, 0) } != 0 {
                return false;
            }
            // Linux: the state letter follows the `(comm)` field.
            match fs::read_to_string(format!("/proc/{pid}/stat")) {
                Ok(stat) => stat
                    .rsplit_once(')')
                    .is_none_or(|(_, rest)| !rest.trim_start().starts_with('Z')),
                Err(_) => true,
            }
        }

        #[test]
        fn a_timeout_ends_the_whole_process_group() {
            let dir = crate::testutil::unique_dir("ws-pr-title-group");
            fs::create_dir_all(&dir).unwrap();
            let pid_file = dir.join("grandchild.pid");
            // No `exec`: the shell stays the child, and `sleep` runs below
            // it. Killing only the shell would leave both sleeps behind.
            let script = Script::new(&format!(
                "sleep 30 &\necho $! > '{pid}.tmp'\nmv '{pid}.tmp' '{pid}'\nsleep 5\necho late",
                pid = pid_file.display()
            ));
            // A busy host may not start the script within the short limit;
            // retry with a longer one so the group check always runs.
            for limit in [500, 3000].map(Duration::from_millis) {
                let started = Instant::now();
                let title = run_title_command_within(&script.path, "ABC-123", limit);
                assert_eq!(title, None, "the title came after the limit");
                assert!(started.elapsed() < limit + Duration::from_secs(3));
                let Some(pid) = fs::read_to_string(&pid_file)
                    .ok()
                    .and_then(|text| text.trim().parse::<libc::pid_t>().ok())
                else {
                    continue;
                };
                let gone_by = Instant::now() + Duration::from_secs(3);
                while process_runs(pid) {
                    assert!(Instant::now() < gone_by, "the grandchild still runs");
                    std::thread::sleep(Duration::from_millis(20));
                }
                let _ = fs::remove_dir_all(&dir);
                return;
            }
            panic!("the script never started");
        }

        #[test]
        fn a_running_title_command_sits_in_the_exit_kill_registry() {
            let dir = crate::testutil::unique_dir("ws-pr-title-registry");
            fs::create_dir_all(&dir).unwrap();
            let pid_file = dir.join("command.pid");
            // `$$` is the shell: the child and its process group leader.
            let script = Script::new(&format!(
                "echo $$ > '{pid}.tmp'\nmv '{pid}.tmp' '{pid}'\nsleep 1\necho Title",
                pid = pid_file.display()
            ));
            let path = script.path.clone();
            let run = std::thread::spawn(move || {
                run_title_command_within(&path, "ABC-123", Duration::from_secs(10))
            });
            let started_by = Instant::now() + Duration::from_secs(5);
            let pid = loop {
                if let Some(pid) = fs::read_to_string(&pid_file)
                    .ok()
                    .and_then(|text| text.trim().parse::<i32>().ok())
                {
                    break pid;
                }
                assert!(Instant::now() < started_by, "the script never started");
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(
                crate::git::is_tracked(pid),
                "a quit stops the running command's group"
            );
            assert_eq!(run.join().unwrap(), Some("Title".to_string()));
            assert!(!crate::git::is_tracked(pid), "reaped: its slot is free");
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_command_inside_the_limit_still_gives_its_title() {
            let script = Script::new("sleep 0.2; echo Title");
            assert_eq!(
                run_title_command_within(&script.path, "ABC-123", Duration::from_secs(5)),
                Some("Title".to_string())
            );
        }

        #[test]
        fn a_command_with_a_lot_of_output_is_not_stalled() {
            // About 1 MB after the first line: far more than a pipe holds.
            let body = "echo Title; i=0; while [ $i -lt 20000 ]; do \
                        echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; \
                        i=$((i+1)); done";
            assert_eq!(run(body, "ABC-123"), Some("Title".to_string()));
        }

        #[test]
        fn a_very_long_first_line_is_cut() {
            let title = run("head -c 100000 /dev/zero | tr '\\0' 'x'; echo", "ABC-123").unwrap();
            assert_eq!(title.len(), TITLE_LINE_MAX as usize);
        }

        fn one_pr(branch: &str, number: u64, title: &str) -> Vec<PrEntry> {
            vec![app(number, branch, title, PrState::InReview)]
        }

        #[test]
        fn the_cache_runs_the_command_once_per_key_and_only_for_ticket_groups() {
            let dir = crate::testutil::unique_dir("ws-pr-title-calls");
            fs::create_dir_all(&dir).unwrap();
            let calls = dir.join("calls");
            let script = Script::new(&format!(
                "echo \"$1\" >> '{}'; if [ \"$1\" = ABC-125 ]; then exit 1; fi; echo \"Title of $1\"",
                calls.display()
            ));
            let mut entries = one_pr("feature/ABC-123-x", 1, "t");
            entries.extend(one_pr("feature/ABC-125-x", 2, "t"));
            entries.extend(one_pr("shared-topic", 3, "t"));
            entries.push(entry(
                "acme/merger",
                MERGER,
                4,
                "shared-topic",
                "t",
                PrState::InReview,
            ));
            entries.extend(one_pr("solo", 5, "t"));
            let groups = group_prs(entries);
            assert_eq!(
                keys(&groups),
                vec!["ABC-123", "ABC-125", "shared-topic", "other"]
            );

            let mut cache = TitleCache::default();
            cache.fill(&script.path, &groups);
            cache.fill(&script.path, &groups);

            assert_eq!(cache.get("ABC-123"), Some("Title of ABC-123"));
            assert_eq!(cache.get("ABC-125"), None, "the command failed for it");
            assert_eq!(cache.get("shared-topic"), None);
            assert_eq!(cache.get("other"), None);
            assert_eq!(cache.get("ABC-999"), None, "never asked");
            let mut called: Vec<String> = fs::read_to_string(&calls)
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect();
            called.sort();
            assert_eq!(
                called,
                vec!["ABC-123", "ABC-125"],
                "once per key, no shared branch, no other"
            );
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_new_cache_runs_the_command_again() {
            let dir = crate::testutil::unique_dir("ws-pr-title-cycles");
            fs::create_dir_all(&dir).unwrap();
            let calls = dir.join("calls");
            let script = Script::new(&format!("echo \"$1\" >> '{}'; echo T", calls.display()));
            let groups = group_prs(one_pr("feature/ABC-123-x", 1, "t"));
            TitleCache::default().fill(&script.path, &groups);
            TitleCache::default().fill(&script.path, &groups);
            assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 2);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
