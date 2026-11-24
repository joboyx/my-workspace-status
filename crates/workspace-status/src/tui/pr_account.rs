//! Pick the `gh` account that runs the PR lookups of a GitHub repository.
//!
//! `gh` runs as the active account of a host. When the user has several
//! logged-in accounts on one host, a private repository that only another
//! account can see fails with `Could not resolve to a Repository`.
//! [`PrAccounts::run`] wraps every repo-scoped `gh` call of
//! [`super::pull_request`] and picks the account in this order:
//!
//! 1. **Env token.** A non-empty `GH_TOKEN` / `GITHUB_TOKEN` (github.com and
//!    `*.ghe.com`) or `GH_ENTERPRISE_TOKEN` / `GITHUB_ENTERPRISE_TOKEN`
//!    (other hosts) in the environment of `ws`: the call runs as is, with no
//!    account listing, retry, or token ([`EnvTokens`]).
//! 2. **Fallback.** The call runs as the active account. When that fails
//!    because the account cannot see the repository ([`cannot_see_repo`]),
//!    the other logged-in accounts of the host (`gh auth status --json
//!    hosts`, [`parse_auth_status`]) are tried once each, in the order `gh`
//!    lists them. The first login that works is cached for `host/owner`, in
//!    memory and in the UI-state file
//!    ([`super::ui_state_store::save_pr_account_cache`]); later calls for
//!    that owner run with it at once. A cached login that can no longer see
//!    the repository, or that is no longer logged in, is dropped and the
//!    fallback runs again.
//!
//! Only a "cannot see the repository" error starts a retry. A timeout, a
//! network error, a rate limit, a bad-credentials error of the account
//! itself, or a successful answer (an empty PR list too) is returned as it
//! is.
//!
//! A token comes from `gh auth token --hostname <host> --user <login>`,
//! read again for each call that needs it, and goes only into the env of
//! the one child `gh` process ([`GhToken`]). It is never kept, put on argv,
//! in a reason text, or in a file. A failed or empty `gh auth token` for a
//! cached login means "no longer logged in"; a quit cancel (no reason)
//! leaves the cache as it is. GitLab calls pass
//! through unchanged: `glab` has one account per host.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use serde::Deserialize;

use super::pull_request::{parse_remote, Forge, ForgeRepo};
use super::ui_state_store::{load_pr_account_cache, save_pr_account_cache};

/// The token of one logged-in `gh` account, for the env of one child
/// process. Its `Debug` form hides the secret, and it has no `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct GhToken {
    /// The env var `gh` reads for the host ([`token_env_var`]).
    var: &'static str,
    /// The token text, trimmed.
    secret: String,
}

impl GhToken {
    /// The env var name and the token text to set on the child process.
    pub fn env(&self) -> (&'static str, &str) {
        (self.var, &self.secret)
    }
}

impl fmt::Debug for GhToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GhToken")
            .field("var", &self.var)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// A forge CLI runner with an optional token: argv (program first) and the
/// token of the account to run as (`None`: the CLI's own login) in; stdout
/// of a zero exit, or the failure reason when there is one, out.
pub type TokenRunner<'a> =
    &'a dyn Fn(&[String], Option<&GhToken>) -> Result<String, Option<String>>;

/// True for a host whose `gh` token env vars are `GH_TOKEN` /
/// `GITHUB_TOKEN`: github.com and GitHub Enterprise Cloud (`*.ghe.com`).
/// Any other host is a GitHub Enterprise Server host.
fn is_github_cloud(host: &str) -> bool {
    host == "github.com" || host.ends_with(".ghe.com")
}

/// The env var a token for `host` goes into: `GH_TOKEN` for github.com
/// and `*.ghe.com`, else `GH_ENTERPRISE_TOKEN` (the `gh` rules).
pub fn token_env_var(host: &str) -> &'static str {
    if is_github_cloud(host) {
        "GH_TOKEN"
    } else {
        "GH_ENTERPRISE_TOKEN"
    }
}

/// Which `gh` token env vars are set, non-empty, in the environment of
/// `ws`. A set token makes `gh` ignore its stored logins, so the account
/// logic stays off for the hosts it covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnvTokens {
    /// `GH_TOKEN` or `GITHUB_TOKEN`: github.com and `*.ghe.com`.
    pub cloud: bool,
    /// `GH_ENTERPRISE_TOKEN` or `GITHUB_ENTERPRISE_TOKEN`: other hosts.
    pub enterprise: bool,
}

impl EnvTokens {
    /// Read the token env vars through `get` (`std::env::var` in
    /// production). Blank values count as unset.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let set = |names: [&str; 2]| {
            names
                .into_iter()
                .any(|name| get(name).is_some_and(|value| !value.trim().is_empty()))
        };
        Self {
            cloud: set(["GH_TOKEN", "GITHUB_TOKEN"]),
            enterprise: set(["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]),
        }
    }

    /// True when a set env token covers `host`.
    pub fn covers(self, host: &str) -> bool {
        if is_github_cloud(host) {
            self.cloud
        } else {
            self.enterprise
        }
    }
}

/// `host/owner` of `repo`, lower case: the key of the account cache.
pub fn owner_key(repo: &ForgeRepo) -> String {
    let owner = repo.path.split('/').next().unwrap_or_default();
    format!("{}/{owner}", repo.host).to_ascii_lowercase()
}

/// The logged-in accounts of one host, from `gh auth status --json hosts`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostAccounts {
    /// The active login, when its state is `success`.
    pub active: Option<String>,
    /// Logins that are not active and whose state is `success`, in listed
    /// order: the fallback candidates.
    pub others: Vec<String>,
}

/// One account entry of `gh auth status --json hosts`.
#[derive(Deserialize)]
struct AuthEntry {
    #[serde(default)]
    login: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    state: String,
}

/// `gh auth status --json hosts` output.
#[derive(Deserialize)]
struct AuthStatus {
    #[serde(default)]
    hosts: HashMap<String, Vec<AuthEntry>>,
}

/// The accounts of `host` in `gh auth status --json hosts` output. `None`
/// when the JSON does not parse or lists no account for `host`.
pub fn parse_auth_status(json: &str, host: &str) -> Option<HostAccounts> {
    let status: AuthStatus = serde_json::from_str(json).ok()?;
    let entries = status.hosts.get(host)?;
    let mut accounts = HostAccounts::default();
    for entry in entries {
        if entry.state != "success" || entry.login.is_empty() {
            continue;
        }
        if entry.active {
            accounts.active = Some(entry.login.clone());
        } else {
            accounts.others.push(entry.login.clone());
        }
    }
    Some(accounts)
}

/// True when `answer` failed because the account cannot see the
/// repository: GitHub's GraphQL `Could not resolve to a Repository`, or the
/// SAML single sign-on block `Resource protected by organization SAML
/// enforcement` (another account may be authorized for that
/// organization). Every call this module wraps is a repo-scoped GraphQL
/// call.
pub fn cannot_see_repo(answer: &Result<String, Option<String>>) -> bool {
    matches!(answer, Err(Some(reason))
        if reason.contains("Could not resolve to a Repository")
            || reason.contains("Resource protected by organization SAML enforcement"))
}

/// Outcome of one `gh auth token` call.
enum TokenFetch {
    /// The token of the login.
    Token(GhToken),
    /// The call failed with a reason or printed nothing: the login is not
    /// logged in on that host.
    NotLoggedIn,
    /// The call failed with no reason (a quit cancel): nothing is known.
    Unknown,
}

/// `gh auth status` argv for the accounts of `host` as JSON.
pub fn auth_status_argv(host: &str) -> Vec<String> {
    [
        "gh",
        "auth",
        "status",
        "--hostname",
        host,
        "--json",
        "hosts",
    ]
    .map(str::to_string)
    .to_vec()
}

/// `gh auth token` argv for the token of `login` on `host`.
pub fn auth_token_argv(host: &str, login: &str) -> Vec<String> {
    ["gh", "auth", "token", "--hostname", host, "--user", login]
        .map(str::to_string)
        .to_vec()
}

/// Failure reason when no logged-in account could see `path`: `no
/// logged-in gh account can see acme-corp/secret (tried octo-main,
/// octo-work)`.
pub fn no_account_reason(path: &str, tried: &[String]) -> String {
    format!(
        "no logged-in gh account can see {path} (tried {})",
        tried.join(", ")
    )
}

/// The account rules of the PR lookups and the `host/owner` → login cache,
/// shared by every PR worker job. See the module docs for the order.
///
/// No token is kept: each call that runs as another account reads its
/// token again with `gh auth token`, so `gh` can refresh an expiring token
/// and a logout takes effect at once.
#[derive(Default)]
pub struct PrAccounts {
    /// Token env vars set in the environment of `ws`.
    env_tokens: EnvTokens,
    /// UI-state file that persists the cache; `None` keeps it in memory.
    store: Option<PathBuf>,
    /// `host/owner` → the login that could see that owner's repos.
    cache: Mutex<BTreeMap<String, String>>,
}

impl PrAccounts {
    /// Account rules with the env token flags and the cache that the
    /// UI-state file `store` holds (none for `None`).
    pub fn new(env_tokens: EnvTokens, store: Option<PathBuf>) -> Self {
        let cache = store
            .as_deref()
            .map(load_pr_account_cache)
            .unwrap_or_default();
        Self {
            env_tokens,
            store,
            cache: Mutex::new(cache),
        }
    }

    /// Run `argv` for the repository behind `remote_url` through `run`, as
    /// the account the module rules pick. A call that is not `gh`, or a
    /// remote that is not GitHub, runs as is.
    pub fn run(
        &self,
        remote_url: Option<&str>,
        argv: &[String],
        run: TokenRunner,
    ) -> Result<String, Option<String>> {
        let repo = remote_url
            .and_then(parse_remote)
            .filter(|repo| repo.forge == Forge::GitHub);
        let Some(repo) = repo.filter(|_| argv.first().is_some_and(|p| p == "gh")) else {
            return run(argv, None);
        };
        if self.env_tokens.covers(&repo.host) {
            return run(argv, None);
        }
        let key = owner_key(&repo);
        let cached = self.lock().get(&key).cloned();
        let mut stale = None;
        // True when a `gh` call ran as the stale login (it was logged in).
        let mut stale_ran = false;
        if let Some(login) = cached {
            match self.token(&repo.host, &login, run) {
                TokenFetch::Token(token) => {
                    let answer = run(argv, Some(&token));
                    if !cannot_see_repo(&answer) {
                        return answer;
                    }
                    stale_ran = true;
                }
                TokenFetch::NotLoggedIn => {}
                // A quit: keep the cache and the file as they are.
                TokenFetch::Unknown => return Err(None),
            }
            stale = Some(login);
        }
        let (answer, winner) = self.fall_back(&repo, stale.as_deref(), stale_ran, argv, run);
        // One write at most: the new login replaces the stale one, or the
        // stale one goes when no other login won.
        match winner {
            Some(login) => self.set_cache(&key, Some(login)),
            None if stale.is_some() => self.set_cache(&key, None),
            None => {}
        }
        answer
    }

    /// Rule 2: the active account, then each other logged-in account once,
    /// except `stale`, the cached login that just failed. `stale_ran` is true
    /// when a `gh` call ran as `stale`, so the failure reason lists it; a
    /// stale login that was only logged out is not listed. Returns the
    /// answer and the other login that worked, when one did.
    fn fall_back(
        &self,
        repo: &ForgeRepo,
        stale: Option<&str>,
        stale_ran: bool,
        argv: &[String],
        run: TokenRunner,
    ) -> (Result<String, Option<String>>, Option<String>) {
        let answer = run(argv, None);
        if !cannot_see_repo(&answer) {
            return (answer, None);
        }
        let Some(accounts) = run(&auth_status_argv(&repo.host), None)
            .ok()
            .and_then(|json| parse_auth_status(&json, &repo.host))
        else {
            return (answer, None);
        };
        let others: Vec<String> = accounts
            .others
            .into_iter()
            .filter(|login| Some(login.as_str()) != stale)
            .collect();
        // Only the active account ran: report its error as a one-account
        // host does.
        if others.is_empty() && !stale_ran {
            return (answer, None);
        }
        let mut tried: Vec<String> = accounts.active.into_iter().collect();
        tried.extend(stale.filter(|_| stale_ran).map(str::to_string));
        for login in others {
            let token = match self.token(&repo.host, &login, run) {
                TokenFetch::Token(token) => token,
                TokenFetch::NotLoggedIn => continue,
                TokenFetch::Unknown => return (Err(None), None),
            };
            let next = run(argv, Some(&token));
            if cannot_see_repo(&next) {
                tried.push(login);
                continue;
            }
            let winner = next.is_ok().then_some(login);
            return (next, winner);
        }
        (Err(Some(no_account_reason(&repo.path, &tried))), None)
    }

    /// The token of `login` on `host` from `gh auth token` (stdout,
    /// trimmed), read again on each call.
    fn token(&self, host: &str, login: &str, run: TokenRunner) -> TokenFetch {
        match run(&auth_token_argv(host, login), None) {
            Ok(secret) if !secret.trim().is_empty() => TokenFetch::Token(GhToken {
                var: token_env_var(host),
                secret: secret.trim().to_string(),
            }),
            Ok(_) | Err(Some(_)) => TokenFetch::NotLoggedIn,
            Err(None) => TokenFetch::Unknown,
        }
    }

    /// Set (`Some`) or drop (`None`) the cached login of `key`, and save the
    /// cache to the UI-state file when it changed. A failed save keeps the
    /// change for the session.
    fn set_cache(&self, key: &str, login: Option<String>) {
        let mut cache = self.lock();
        let changed = match login {
            Some(login) => cache.insert(key.to_string(), login.clone()) != Some(login),
            None => cache.remove(key).is_some(),
        };
        // Save under the lock so two workers cannot write their snapshots
        // out of order.
        if let (true, Some(store)) = (changed, self.store.as_deref()) {
            let _ = save_pr_account_cache(store, &cache);
        }
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, String>> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The `host/owner` → login cache.
    #[cfg(test)]
    pub(crate) fn cache(&self) -> BTreeMap<String, String> {
        self.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::unique_dir;
    use std::cell::RefCell;

    const AUTH_STATUS: &str = r#"{"hosts":{"github.com":[{"login":"octo-main","active":true,"state":"success","host":"github.com"},{"login":"octo-work","active":false,"state":"success","host":"github.com"},{"login":"octo-old","active":false,"state":"error","host":"github.com"}]}}"#;

    const NOT_VISIBLE: &str =
        "GraphQL: Could not resolve to a Repository with the name 'acme-corp/secret'. (repository)";

    const REMOTE: &str = "git@github.com:acme-corp/secret.git";

    fn repo(host: &str, path: &str) -> ForgeRepo {
        ForgeRepo {
            forge: Forge::GitHub,
            host: host.into(),
            path: path.into(),
        }
    }

    fn list_argv() -> Vec<String> {
        super::super::pull_request::github_list_argv(
            &repo("github.com", "acme-corp/secret"),
            "feature",
        )
    }

    /// A fake `gh`: `auth status` prints `auth`, `auth token` prints
    /// `tok-<login>` for a login in `logged_in`, and a PR call works for a
    /// login in `sees` (the active one when no token is given) and fails
    /// with [`NOT_VISIBLE`] for the others. `pr_error` makes every PR call
    /// fail with that reason instead. Each call is logged as `<args> as
    /// <login>`.
    struct FakeGh {
        auth: &'static str,
        active: &'static str,
        logged_in: Vec<&'static str>,
        sees: Vec<&'static str>,
        pr_error: Option<&'static str>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeGh {
        fn new(sees: &[&'static str]) -> Self {
            Self {
                auth: AUTH_STATUS,
                active: "octo-main",
                logged_in: vec!["octo-main", "octo-work"],
                sees: sees.to_vec(),
                pr_error: None,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn run(&self, argv: &[String], token: Option<&GhToken>) -> Result<String, Option<String>> {
            let who = token.map_or(self.active.to_string(), |token| {
                let (var, secret) = token.env();
                assert_eq!(var, "GH_TOKEN");
                secret.strip_prefix("tok-").expect("fake token").to_string()
            });
            self.calls
                .borrow_mut()
                .push(format!("{} as {who}", argv[1..3].join(" ")));
            match (argv[1].as_str(), argv[2].as_str()) {
                ("auth", "status") => Ok(self.auth.to_string()),
                ("auth", "token") => {
                    let user = argv.last().expect("login").as_str();
                    if self.logged_in.contains(&user) {
                        Ok(format!("tok-{user}\n"))
                    } else {
                        Err(Some(format!(
                            "no oauth token found for github.com account {user}"
                        )))
                    }
                }
                _ => match self.pr_error {
                    Some(reason) => Err(Some(reason.to_string())),
                    None if self.sees.contains(&who.as_str()) => Ok("[]".to_string()),
                    None => Err(Some(NOT_VISIBLE.to_string())),
                },
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow_mut().drain(..).collect()
        }
    }

    fn run_with(
        accounts: &PrAccounts,
        gh: &FakeGh,
        remote: &str,
    ) -> Result<String, Option<String>> {
        accounts.run(Some(remote), &list_argv(), &|argv, token| {
            gh.run(argv, token)
        })
    }

    #[test]
    fn parse_auth_status_keeps_logged_in_accounts_that_are_not_active() {
        assert_eq!(
            parse_auth_status(AUTH_STATUS, "github.com"),
            Some(HostAccounts {
                active: Some("octo-main".into()),
                others: vec!["octo-work".into()],
            })
        );
        assert_eq!(parse_auth_status(AUTH_STATUS, "github.example.com"), None);
        assert_eq!(parse_auth_status("not json", "github.com"), None);
    }

    #[test]
    fn the_owner_key_is_lower_case_host_and_owner() {
        assert_eq!(
            owner_key(&repo("github.com", "Acme-Corp/Secret")),
            "github.com/acme-corp"
        );
    }

    #[test]
    fn env_tokens_follow_the_gh_host_rules() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            EnvTokens::from_env(move |name| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            })
        };
        assert_eq!(env(&[]), EnvTokens::default());
        assert_eq!(env(&[("GH_TOKEN", "  ")]), EnvTokens::default(), "blank");
        let cloud = env(&[("GITHUB_TOKEN", "x")]);
        assert!(cloud.covers("github.com"));
        assert!(cloud.covers("acme.ghe.com"));
        assert!(!cloud.covers("github.example.com"));
        let enterprise = env(&[("GH_ENTERPRISE_TOKEN", "x")]);
        assert!(!enterprise.covers("github.com"));
        assert!(enterprise.covers("github.example.com"));
        assert_eq!(token_env_var("github.com"), "GH_TOKEN");
        assert_eq!(token_env_var("github.example.com"), "GH_ENTERPRISE_TOKEN");
    }

    #[test]
    fn an_env_token_runs_the_call_as_is() {
        let gh = FakeGh::new(&["octo-work"]);
        let accounts = PrAccounts::new(
            EnvTokens {
                cloud: true,
                enterprise: false,
            },
            None,
        );
        assert_eq!(
            run_with(&accounts, &gh, REMOTE),
            Err(Some(NOT_VISIBLE.into()))
        );
        assert_eq!(gh.calls(), ["pr list as octo-main"], "no listing, no token");
        assert!(accounts.cache().is_empty());
    }

    #[test]
    fn only_a_hidden_repo_or_a_saml_block_starts_a_retry() {
        for (reason, retry) in [
            (NOT_VISIBLE, true),
            (
                "GraphQL: Resource protected by organization SAML enforcement. You must grant \
                 your Personal Access token access to this organization.",
                true,
            ),
            ("timed out after 20 s", false),
            ("GraphQL: API rate limit exceeded for user ID 1.", false),
            (
                "HTTP 401: Bad credentials (https://api.github.com/graphql)",
                false,
            ),
            (
                "GraphQL: Could not resolve to a PullRequest with the number of 12. \
                 (repository.pullRequest)",
                false,
            ),
            (
                "HTTP 404: Not Found (https://api.github.com/repos/acme-corp/secret)",
                false,
            ),
        ] {
            assert_eq!(
                cannot_see_repo(&Err(Some(reason.to_string()))),
                retry,
                "{reason}"
            );
        }
        assert!(!cannot_see_repo(&Err(None)), "no reason");
        assert!(!cannot_see_repo(&Ok("[]".into())), "an answer");
    }

    #[test]
    fn a_hidden_repo_retries_the_next_account_and_caches_it_for_the_owner() {
        let gh = FakeGh::new(&["octo-work"]);
        let accounts = PrAccounts::default();
        assert_eq!(run_with(&accounts, &gh, REMOTE), Ok("[]".into()));
        assert_eq!(
            gh.calls(),
            [
                "pr list as octo-main",
                "auth status as octo-main",
                "auth token as octo-main",
                "pr list as octo-work",
            ]
        );
        assert_eq!(
            accounts.cache(),
            BTreeMap::from([("github.com/acme-corp".to_string(), "octo-work".to_string())])
        );
        let other_repo = "git@github.com:acme-corp/tools.git";
        assert_eq!(run_with(&accounts, &gh, other_repo), Ok("[]".into()));
        assert_eq!(
            gh.calls(),
            ["auth token as octo-main", "pr list as octo-work"],
            "the owner's cached account runs at once, with its token read again"
        );
    }

    #[test]
    fn timeouts_and_other_errors_never_retry() {
        for reason in [
            "timed out after 20 s",
            "HTTP 401: Bad credentials (https://api.github.com/graphql)",
            "error connecting to api.github.com",
            "GraphQL: API rate limit exceeded for user ID 1.",
            "gh not found",
        ] {
            let mut gh = FakeGh::new(&[]);
            gh.pr_error = Some(reason);
            let accounts = PrAccounts::default();
            assert_eq!(run_with(&accounts, &gh, REMOTE), Err(Some(reason.into())));
            assert_eq!(gh.calls(), ["pr list as octo-main"], "{reason}");
        }
        let gh = FakeGh::new(&["octo-main"]);
        assert_eq!(
            run_with(&PrAccounts::default(), &gh, REMOTE),
            Ok("[]".into())
        );
        assert_eq!(
            gh.calls(),
            ["pr list as octo-main"],
            "an empty answer is final"
        );
    }

    #[test]
    fn when_no_account_can_see_the_repo_the_reason_lists_the_tried_logins() {
        let gh = FakeGh::new(&[]);
        let accounts = PrAccounts::default();
        let answer = run_with(&accounts, &gh, REMOTE);
        assert_eq!(
            answer,
            Err(Some(
                "no logged-in gh account can see acme-corp/secret (tried octo-main, octo-work)"
                    .into()
            ))
        );
        let reason = answer.unwrap_err().unwrap();
        assert!(!reason.contains("tok-"), "no token text: {reason}");
        assert!(accounts.cache().is_empty());
    }

    #[test]
    fn a_single_account_reports_the_error_as_before() {
        let mut gh = FakeGh::new(&[]);
        gh.auth =
            r#"{"hosts":{"github.com":[{"login":"octo-main","active":true,"state":"success"}]}}"#;
        assert_eq!(
            run_with(&PrAccounts::default(), &gh, REMOTE),
            Err(Some(NOT_VISIBLE.into()))
        );
        assert_eq!(
            gh.calls(),
            ["pr list as octo-main", "auth status as octo-main"]
        );
    }

    /// `gh auth status` with three logged-in accounts on github.com.
    const THREE_ACCOUNTS: &str = r#"{"hosts":{"github.com":[{"login":"octo-main","active":true,"state":"success"},{"login":"octo-work","active":false,"state":"success"},{"login":"octo-admin","active":false,"state":"success"}]}}"#;

    #[test]
    fn other_accounts_are_tried_in_the_order_gh_lists_them() {
        let mut gh = FakeGh::new(&["octo-admin"]);
        gh.auth = THREE_ACCOUNTS;
        gh.logged_in.push("octo-admin");
        let accounts = PrAccounts::default();
        let remote = "git@github.com-octo-admin:acme-corp/secret.git";
        assert_eq!(run_with(&accounts, &gh, remote), Ok("[]".into()));
        assert_eq!(
            gh.calls(),
            [
                "pr list as octo-main",
                "auth status as octo-main",
                "auth token as octo-main",
                "pr list as octo-work",
                "auth token as octo-main",
                "pr list as octo-admin",
            ],
            "the SSH host alias does not change the order"
        );
    }

    #[test]
    fn a_saml_block_retries_the_next_account() {
        const SAML: &str = "GraphQL: Resource protected by organization SAML enforcement. \
                            You must grant your Personal Access token access to this organization.";
        let saml = |argv: &[String], token: Option<&GhToken>| {
            let gh = FakeGh::new(&["octo-work"]);
            match (argv[1].as_str(), token) {
                ("pr", None) => Err(Some(SAML.to_string())),
                _ => gh.run(argv, token),
            }
        };
        let accounts = PrAccounts::default();
        assert_eq!(
            accounts.run(Some(REMOTE), &list_argv(), &saml),
            Ok("[]".into())
        );
        assert_eq!(
            accounts.cache(),
            BTreeMap::from([("github.com/acme-corp".to_string(), "octo-work".to_string())])
        );
    }

    #[test]
    fn a_stale_cached_account_is_dropped_and_the_fallback_runs_again() {
        let mut gh = FakeGh::new(&["octo-admin"]);
        gh.auth = THREE_ACCOUNTS;
        gh.logged_in.push("octo-admin");
        let accounts = PrAccounts::default();
        accounts.set_cache("github.com/acme-corp", Some("octo-work".into()));
        assert_eq!(run_with(&accounts, &gh, REMOTE), Ok("[]".into()));
        assert_eq!(
            gh.calls(),
            [
                "auth token as octo-main",
                "pr list as octo-work",
                "pr list as octo-main",
                "auth status as octo-main",
                "auth token as octo-main",
                "pr list as octo-admin",
            ],
            "the stale login is not tried twice"
        );
        assert_eq!(
            accounts.cache(),
            BTreeMap::from([("github.com/acme-corp".to_string(), "octo-admin".to_string())])
        );
    }

    #[test]
    fn a_stale_cached_account_with_no_winner_is_dropped() {
        let dir = unique_dir("ws-pr-account-stale");
        let store = dir.join("ui-state.json");
        let gh = FakeGh::new(&[]);
        let accounts = PrAccounts::new(EnvTokens::default(), Some(store.clone()));
        accounts.set_cache("github.com/acme-corp", Some("octo-work".into()));
        assert_eq!(
            run_with(&accounts, &gh, REMOTE),
            Err(Some(
                "no logged-in gh account can see acme-corp/secret (tried octo-main, octo-work)"
                    .into()
            ))
        );
        assert_eq!(
            gh.calls(),
            [
                "auth token as octo-main",
                "pr list as octo-work",
                "pr list as octo-main",
                "auth status as octo-main",
            ]
        );
        assert!(accounts.cache().is_empty());
        assert!(load_pr_account_cache(&store).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_logged_out_cached_login_is_not_listed_as_tried() {
        let mut gh = FakeGh::new(&[]);
        gh.auth = THREE_ACCOUNTS;
        gh.logged_in = vec!["octo-main", "octo-admin"];
        let accounts = PrAccounts::default();
        accounts.set_cache("github.com/acme-corp", Some("octo-work".into()));
        assert_eq!(
            run_with(&accounts, &gh, REMOTE),
            Err(Some(
                "no logged-in gh account can see acme-corp/secret (tried octo-main, octo-admin)"
                    .into()
            ))
        );
        assert_eq!(
            gh.calls(),
            [
                "auth token as octo-main",
                "pr list as octo-main",
                "auth status as octo-main",
                "auth token as octo-main",
                "pr list as octo-admin",
            ],
            "no pr call ran as octo-work"
        );
        assert!(accounts.cache().is_empty(), "dropped");

        // With no other account left, the active account's error stands.
        let mut gh = FakeGh::new(&[]);
        gh.logged_in = vec!["octo-main"];
        let accounts = PrAccounts::default();
        accounts.set_cache("github.com/acme-corp", Some("octo-work".into()));
        assert_eq!(
            run_with(&accounts, &gh, REMOTE),
            Err(Some(NOT_VISIBLE.into()))
        );
        assert!(accounts.cache().is_empty(), "dropped");
    }

    #[test]
    fn a_quit_while_reading_a_cached_token_keeps_the_cache_and_the_file() {
        let dir = unique_dir("ws-pr-account-quit");
        let store = dir.join("ui-state.json");
        let cache = BTreeMap::from([("github.com/acme-corp".to_string(), "octo-work".to_string())]);
        save_pr_account_cache(&store, &cache).unwrap();
        let before = std::fs::read(&store).unwrap();
        let accounts = PrAccounts::new(EnvTokens::default(), Some(store.clone()));
        let calls = RefCell::new(Vec::new());
        let cancelled = |argv: &[String], _: Option<&GhToken>| {
            calls.borrow_mut().push(argv[1..3].join(" "));
            Err(None)
        };
        assert_eq!(
            accounts.run(Some(REMOTE), &list_argv(), &cancelled),
            Err(None)
        );
        assert_eq!(calls.into_inner(), ["auth token"], "nothing else runs");
        assert_eq!(accounts.cache(), cache);
        assert_eq!(std::fs::read(&store).unwrap(), before, "file unchanged");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_cache_persists_and_a_logged_out_login_is_dropped() {
        let dir = unique_dir("ws-pr-account-store");
        let store = dir.join("ui-state.json");
        let gh = FakeGh::new(&["octo-work"]);
        let accounts = PrAccounts::new(EnvTokens::default(), Some(store.clone()));
        assert_eq!(run_with(&accounts, &gh, REMOTE), Ok("[]".into()));
        let saved = BTreeMap::from([("github.com/acme-corp".to_string(), "octo-work".to_string())]);
        assert_eq!(load_pr_account_cache(&store), saved);
        let contents = std::fs::read_to_string(&store).unwrap();
        assert!(
            !contents.contains("tok-"),
            "no token in the file: {contents}"
        );

        let relaunch = PrAccounts::new(EnvTokens::default(), Some(store.clone()));
        assert_eq!(relaunch.cache(), saved, "loaded at launch");
        gh.calls();
        assert_eq!(run_with(&relaunch, &gh, REMOTE), Ok("[]".into()));
        assert_eq!(
            gh.calls(),
            ["auth token as octo-main", "pr list as octo-work"]
        );

        let mut logged_out = FakeGh::new(&["octo-main"]);
        logged_out.logged_in = vec!["octo-main"];
        let relaunch = PrAccounts::new(EnvTokens::default(), Some(store.clone()));
        assert_eq!(run_with(&relaunch, &logged_out, REMOTE), Ok("[]".into()));
        assert_eq!(
            logged_out.calls(),
            ["auth token as octo-main", "pr list as octo-main"]
        );
        assert!(relaunch.cache().is_empty(), "dropped");
        assert!(
            load_pr_account_cache(&store).is_empty(),
            "removed from the file"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn gitlab_and_other_programs_pass_through() {
        let gh = FakeGh::new(&[]);
        let accounts = PrAccounts::default();
        let glab = vec!["glab".to_string(), "api".into(), "x".into()];
        let seen = RefCell::new(Vec::new());
        let answer = accounts.run(
            Some("git@gitlab.com:group/demo.git"),
            &glab,
            &|argv, token| {
                seen.borrow_mut().push((argv.to_vec(), token.is_some()));
                Err(Some("404 Not Found".into()))
            },
        );
        assert_eq!(answer, Err(Some("404 Not Found".into())));
        assert_eq!(seen.into_inner(), [(glab, false)]);
        assert_eq!(
            accounts.run(None, &list_argv(), &|argv, token| gh.run(argv, token)),
            Err(Some(NOT_VISIBLE.into())),
            "no remote: as is"
        );
        assert_eq!(gh.calls(), ["pr list as octo-main"]);
    }

    #[test]
    fn a_token_debug_form_hides_the_secret() {
        let token = GhToken {
            var: "GH_TOKEN",
            secret: "tok-octo-work".into(),
        };
        let text = format!("{token:?}");
        assert_eq!(text, r#"GhToken { var: "GH_TOKEN", secret: "<redacted>" }"#);
        assert_eq!(token.env(), ("GH_TOKEN", "tok-octo-work"));
    }
}
