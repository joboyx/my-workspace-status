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

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::pr_status::{NextAction, PrState};
use super::pull_request::PullRequest;

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

/// Head branches that never make a shared-branch group.
const COMMON_BRANCHES: [&str; 7] = [
    "main",
    "master",
    "develop",
    "dev",
    "trunk",
    "staging",
    "production",
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
/// then [`OTHER_KEY`]; ties by key ascending. Row order: repository name,
/// then number, with a stacked PR right after its base when both are in the
/// group.
///
/// A PR is stacked on a known PR of the same repository, whatever group that
/// PR is in. The repository is read from the PR URL.
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
    groups.sort_by(|a, b| group_rank(a).cmp(&group_rank(b)).then(a.key.cmp(&b.key)));
    groups
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
#[derive(Debug, Default)]
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
            let base = entry.pr.stacked_on(known.iter().copied());
            let stacked_on = base.map(|base| StackedOn {
                pr_ref: base.ref_text(),
                number: base.number,
                state: base.state,
            });
            (stacked_on, entry.pr.next_action(base))
        })
        .collect()
}

/// Head branches that make a shared-branch group: not a common branch, and
/// the head of two or more listed PRs that have no ticket key
/// (`ticket_keys[i]` is the key of `entries[i]`). The same PR listed for two
/// checkouts counts once.
fn shared_branches(entries: &[PrEntry], ticket_keys: &[Option<String>]) -> HashSet<String> {
    let mut prs_by_head: HashMap<&str, HashSet<(&str, u64)>> = HashMap::new();
    for (entry, ticket_key) in entries.iter().zip(ticket_keys) {
        let head = entry.pr.head.as_str();
        if ticket_key.is_none() && !head.is_empty() && !COMMON_BRANCHES.contains(&head) {
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
/// review`, `merge <base> first`, `merge` from any row, in that priority and
/// then in row order; else the action of the first open row.
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
        None => GroupHeadline::AllMerged(rows.len()),
    }
}

/// Rank of an action the author must take; the lowest rank leads the
/// header.
fn needs_you_priority(action: &NextAction) -> u8 {
    match action {
        NextAction::FixCi => 0,
        NextAction::FixConflicts => 1,
        NextAction::AddressReview => 2,
        NextAction::MergeBaseFirst(_) => 3,
        NextAction::Merge => 4,
        NextAction::Undraft | NextAction::WaitForCi | NextAction::GetReview => 5,
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

fn run_title_command_within(cmd: &Path, key: &str, limit: Duration) -> Option<String> {
    let deadline = Instant::now() + limit;
    let mut child = spawn_title_command(cmd, key, deadline)?;
    let Some(pipe) = child.stdout.take() else {
        kill_and_reap(child);
        return None;
    };
    let first_line = read_first_line(pipe);
    let Some(status) = wait_until(&mut child, deadline) else {
        kill_and_reap(child);
        return None;
    };
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
        let spawned = Command::new(cmd)
            .arg(key)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => return Some(child),
            // The script is still open for writing, for example while an
            // editor saves it. Try again until the deadline.
            Err(error)
                if error.kind() == ErrorKind::ExecutableFileBusy && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

fn kill_and_reap(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Poll `child` until it exits (`Some`), or until `deadline` passes or its
/// state cannot be read (`None`).
fn wait_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(10)));
    }
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
