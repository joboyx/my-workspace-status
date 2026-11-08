use super::*;
use crate::testutil::unique_dir;
use std::fs;

#[test]
fn missing_config_uses_defaults() {
    let dir = unique_dir("ws-config-missing");
    fs::create_dir_all(&dir).unwrap();
    let cfg = load_config_files(None, &dir).unwrap();
    assert!(cfg.hidden_paths.is_empty());
    assert_eq!(cfg.max_depth, 3);
    assert_eq!(cfg.diff_tool, None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn loads_hidden_paths_sorted() {
    let dir = unique_dir("ws-config-ok");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(CONFIG_FILENAME),
        r#"{"hiddenPaths":["notes","./vendor/"]}"#,
    )
    .unwrap();
    let cfg = load_config_files(None, &dir).unwrap();
    assert_eq!(cfg.hidden_paths, vec!["notes", "vendor"]);
    assert_eq!(cfg.diff_tool, None);
    let _ = fs::remove_dir_all(&dir);
}

fn write_config(dir_prefix: &str, json: &str) -> std::path::PathBuf {
    let dir = unique_dir(dir_prefix);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(CONFIG_FILENAME), json).unwrap();
    dir
}

#[test]
fn omit_diff_tool_is_none() {
    let dir = write_config("ws-config-omit-diff", r#"{"hiddenPaths":[]}"#);
    let cfg = load_config_files(None, &dir).unwrap();
    assert_eq!(cfg.diff_tool, None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn diff_tool_vimdiff_is_kept() {
    let dir = write_config(
        "ws-config-diff-vim",
        r#"{"hiddenPaths":[],"diffTool":"vimdiff"}"#,
    );
    let cfg = load_config_files(None, &dir).unwrap();
    assert_eq!(cfg.diff_tool.as_deref(), Some("vimdiff"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn blank_diff_tool_is_none() {
    let dir = write_config(
        "ws-config-diff-blank",
        r#"{"hiddenPaths":[],"diffTool":"  "}"#,
    );
    let cfg = load_config_files(None, &dir).unwrap();
    assert_eq!(cfg.diff_tool, None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn non_string_diff_tool_is_error() {
    let dir = write_config("ws-config-diff-bad", r#"{"hiddenPaths":[],"diffTool":1}"#);
    let err = load_config_files(None, &dir).unwrap_err();
    assert!(
        err.contains("diffTool must be a string"),
        "unexpected error: {err}"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn load_view_defaults(json: &str) -> Result<ViewDefaults, String> {
    let dir = write_config("ws-config-view", json);
    let out = load_config_files(None, &dir).map(|cfg| cfg.view_defaults);
    let _ = fs::remove_dir_all(&dir);
    out
}

#[test]
fn omitted_view_defaults_are_all_none() {
    assert_eq!(
        load_view_defaults(r#"{"hiddenPaths":[]}"#).unwrap(),
        ViewDefaults::default()
    );
    assert_eq!(
        WorkspaceStatusConfig::with_defaults().view_defaults,
        ViewDefaults::default()
    );
}

#[test]
fn empty_view_defaults_object_is_all_none() {
    assert_eq!(
        load_view_defaults(r#"{"hiddenPaths":[],"viewDefaults":{}}"#).unwrap(),
        ViewDefaults::default()
    );
}

#[test]
fn view_defaults_first_values_are_true() {
    let got = load_view_defaults(
            r#"{"hiddenPaths":[],"viewDefaults":{"tree":"tree","commitTree":"tree","diff":"split","wrap":"wrap","commitMessage":"expand","lineBlame":"show","background":"paint"}}"#,
        )
        .unwrap();
    assert_eq!(
        got,
        ViewDefaults {
            tree: Some(true),
            commit_tree: Some(true),
            diff_split: Some(true),
            wrap: Some(true),
            commit_message_expand: Some(true),
            line_blame: Some(true),
            commit_message_lines: None,
            background: Some(BackgroundMode::Paint),
        }
    );
}

#[test]
fn view_defaults_second_values_are_false() {
    let got = load_view_defaults(
            r#"{"hiddenPaths":[],"viewDefaults":{"tree":"flat","commitTree":"flat","diff":"inline","wrap":"unwrap","commitMessage":"collapse","lineBlame":"hide","background":"terminal"}}"#,
        )
        .unwrap();
    assert_eq!(
        got,
        ViewDefaults {
            tree: Some(false),
            commit_tree: Some(false),
            diff_split: Some(false),
            wrap: Some(false),
            commit_message_expand: Some(false),
            line_blame: Some(false),
            commit_message_lines: None,
            background: Some(BackgroundMode::Terminal),
        }
    );
}

#[test]
fn view_defaults_value_is_trimmed() {
    let got =
        load_view_defaults(r#"{"hiddenPaths":[],"viewDefaults":{"wrap":"  unwrap "}}"#).unwrap();
    assert_eq!(got.wrap, Some(false));
    assert_eq!(got.tree, None);

    let got =
        load_view_defaults(r#"{"hiddenPaths":[],"viewDefaults":{"background":" terminal  "}}"#)
            .unwrap();
    assert_eq!(got.background, Some(BackgroundMode::Terminal));
    assert_eq!(got.wrap, None);
}

#[test]
fn view_defaults_not_an_object_is_error() {
    for raw in [r#"[]"#, r#""wrap""#, "1"] {
        let err = load_view_defaults(&format!(r#"{{"hiddenPaths":[],"viewDefaults":{raw}}}"#))
            .unwrap_err();
        assert_eq!(
            err, ".workspace-status-config.json viewDefaults must be an object",
            "viewDefaults: {raw}"
        );
    }
}

#[test]
fn view_defaults_bad_value_names_the_key_and_choices() {
    let cases = [
        ("tree", "1", r#""tree" or "flat""#),
        ("tree", r#""Tree""#, r#""tree" or "flat""#),
        ("commitTree", r#""list""#, r#""tree" or "flat""#),
        ("diff", r#""sbs""#, r#""split" or "inline""#),
        ("diff", "true", r#""split" or "inline""#),
        ("wrap", r#""""#, r#""wrap" or "unwrap""#),
        ("wrap", r#""   ""#, r#""wrap" or "unwrap""#),
        ("wrap", "false", r#""wrap" or "unwrap""#),
        ("commitMessage", r#""open""#, r#""expand" or "collapse""#),
        ("commitMessage", "null", r#""expand" or "collapse""#),
        ("lineBlame", r#""on""#, r#""show" or "hide""#),
        ("lineBlame", "true", r#""show" or "hide""#),
        ("background", r#""Paint""#, r#""paint" or "terminal""#),
        ("background", r#""none""#, r#""paint" or "terminal""#),
        ("background", "false", r#""paint" or "terminal""#),
    ];
    for (key, raw, choices) in cases {
        let err = load_view_defaults(&format!(
            r#"{{"hiddenPaths":[],"viewDefaults":{{"{key}":{raw}}}}}"#
        ))
        .unwrap_err();
        assert_eq!(
            err,
            format!(".workspace-status-config.json viewDefaults.{key} must be {choices}"),
            "{key}: {raw}"
        );
    }
}

#[test]
fn view_defaults_commit_message_lines_accepts_min_max_and_default() {
    for n in [1usize, 4, 20] {
        let got = load_view_defaults(&format!(
            r#"{{"hiddenPaths":[],"viewDefaults":{{"commitMessageLines":{n}}}}}"#
        ))
        .unwrap();
        assert_eq!(got.commit_message_lines, Some(n));
        assert_eq!(got.commit_message_expand, None);
    }
    assert_eq!(
        load_view_defaults(r#"{"hiddenPaths":[],"viewDefaults":{"wrap":"wrap"}}"#)
            .unwrap()
            .commit_message_lines,
        None,
        "omitted key keeps the in-app default"
    );
}

#[test]
fn view_defaults_commit_message_lines_rejects_bad_values() {
    for raw in [
        "0", "21", "-1", r#""8""#, "8.5", "8.0", "true", "null", "[8]",
    ] {
        let err = load_view_defaults(&format!(
            r#"{{"hiddenPaths":[],"viewDefaults":{{"commitMessageLines":{raw}}}}}"#
        ))
        .unwrap_err();
        assert_eq!(
                err,
                ".workspace-status-config.json viewDefaults.commitMessageLines must be an integer from 1 to 20",
                "commitMessageLines: {raw}"
            );
    }
}

#[test]
fn view_defaults_unknown_key_is_error() {
    let err =
        load_view_defaults(r#"{"hiddenPaths":[],"viewDefaults":{"wrap":"wrap","theme":"dark"}}"#)
            .unwrap_err();
    assert_eq!(
        err,
        r#".workspace-status-config.json viewDefaults has unknown key "theme""#
    );

    let err = load_view_defaults(
        r#"{"hiddenPaths":[],"viewDefaults":{"background":"paint","backgroundColor":"paint"}}"#,
    )
    .unwrap_err();
    assert_eq!(
        err,
        r#".workspace-status-config.json viewDefaults has unknown key "backgroundColor""#
    );
}

/// Temp root with an optional user file and an optional workspace file.
/// Returns `(root, user_file, workspace_root)`; `user_file` is the path the
/// loader gets, whether or not the file exists.
fn layered(user_json: Option<&str>, workspace_json: Option<&str>) -> (PathBuf, PathBuf, PathBuf) {
    let root = unique_dir("ws-config-layered");
    let user_file = root
        .join("xdg-config")
        .join(USER_CONFIG_DIR)
        .join(USER_CONFIG_FILENAME);
    let workspace = root.join("workspace");
    fs::create_dir_all(user_file.parent().unwrap()).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    if let Some(json) = user_json {
        fs::write(&user_file, json).unwrap();
    }
    if let Some(json) = workspace_json {
        fs::write(workspace.join(CONFIG_FILENAME), json).unwrap();
    }
    (root, user_file, workspace)
}

fn load_layered(
    user_json: Option<&str>,
    workspace_json: Option<&str>,
) -> Result<WorkspaceStatusConfig, String> {
    let (root, user_file, workspace) = layered(user_json, workspace_json);
    let out = load_config_files(Some(&user_file), &workspace);
    let _ = fs::remove_dir_all(root);
    out
}

#[test]
fn layered_no_files_uses_defaults() {
    let cfg = load_layered(None, None).unwrap();
    assert!(cfg.hidden_paths.is_empty());
    assert_eq!(cfg.max_depth, DEFAULT_MAX_DEPTH);
    assert!(cfg.default_branches.is_empty());
    assert_eq!(cfg.editor, None);
    assert_eq!(cfg.diff_tool, None);
    assert_eq!(cfg.view_defaults, ViewDefaults::default());

    let (root, _, workspace) = layered(None, None);
    let cfg = load_config_files(None, &workspace).unwrap();
    assert_eq!(cfg.max_depth, DEFAULT_MAX_DEPTH);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn layered_user_file_only() {
    let cfg = load_layered(
            Some(
                r#"{"hiddenPaths":["notes"],"maxDepth":2,"editor":"nvim","diffTool":"code --diff --wait","defaultBranches":{"app":"develop"},"viewDefaults":{"wrap":"unwrap"}}"#,
            ),
            None,
        )
        .unwrap();
    assert_eq!(cfg.hidden_paths, vec!["notes"]);
    assert_eq!(cfg.max_depth, 2);
    assert_eq!(cfg.editor.as_deref(), Some("nvim"));
    assert_eq!(cfg.diff_tool.as_deref(), Some("code --diff --wait"));
    assert_eq!(
        cfg.default_branches.get("app").map(String::as_str),
        Some("develop")
    );
    assert_eq!(cfg.view_defaults.wrap, Some(false));
}

#[test]
fn layered_workspace_file_only() {
    let cfg = load_layered(None, Some(r#"{"hiddenPaths":["vendor"],"maxDepth":4}"#)).unwrap();
    assert_eq!(cfg.hidden_paths, vec!["vendor"]);
    assert_eq!(cfg.max_depth, 4);
    assert_eq!(cfg.editor, None);
}

#[test]
fn layered_workspace_wins_per_top_level_key() {
    let cfg = load_layered(
        Some(r#"{"hiddenPaths":[],"maxDepth":2,"editor":"nvim","diffTool":"vimdiff"}"#),
        Some(r#"{"hiddenPaths":[],"maxDepth":5,"diffTool":"  "}"#),
    )
    .unwrap();
    assert_eq!(cfg.max_depth, 5, "workspace maxDepth wins");
    assert_eq!(
        cfg.editor.as_deref(),
        Some("nvim"),
        "key the workspace omits keeps the user value"
    );
    assert_eq!(
        cfg.diff_tool.as_deref(),
        Some("vimdiff"),
        "blank workspace diffTool is unset, so the user value applies"
    );
}

#[test]
fn layered_view_defaults_merge_per_sub_key() {
    let cfg = load_layered(
            Some(
                r#"{"hiddenPaths":[],"viewDefaults":{"tree":"flat","wrap":"unwrap","commitMessageLines":12}}"#,
            ),
            Some(
                r#"{"hiddenPaths":[],"viewDefaults":{"wrap":"wrap","lineBlame":"hide","background":"terminal"}}"#,
            ),
        )
        .unwrap();
    assert_eq!(
        cfg.view_defaults,
        ViewDefaults {
            tree: Some(false),
            wrap: Some(true),
            line_blame: Some(false),
            commit_message_lines: Some(12),
            background: Some(BackgroundMode::Terminal),
            ..ViewDefaults::default()
        }
    );

    let cfg = load_layered(
        Some(r#"{"hiddenPaths":[],"viewDefaults":{"background":"terminal"}}"#),
        Some(r#"{"hiddenPaths":[],"viewDefaults":{"wrap":"wrap"}}"#),
    )
    .unwrap();
    assert_eq!(
        cfg.view_defaults.background,
        Some(BackgroundMode::Terminal),
        "user background survives a workspace file that omits it"
    );
}

#[test]
fn layered_default_branches_merge_per_sub_key() {
    let cfg = load_layered(
        Some(r#"{"hiddenPaths":[],"defaultBranches":{"app":"develop","lib":"main"}}"#),
        Some(r#"{"hiddenPaths":[],"defaultBranches":{"./app/":"trunk","api":"dev"}}"#),
    )
    .unwrap();
    let got: Vec<(&str, &str)> = cfg
        .default_branches
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(got, vec![("api", "dev"), ("app", "trunk"), ("lib", "main")]);
}

#[test]
fn layered_hidden_paths_are_replaced_not_joined() {
    let cfg = load_layered(
        Some(r#"{"hiddenPaths":["notes","vendor"]}"#),
        Some(r#"{"hiddenPaths":["scratch"]}"#),
    )
    .unwrap();
    assert_eq!(cfg.hidden_paths, vec!["scratch"]);

    let cfg = load_layered(
        Some(r#"{"hiddenPaths":["notes"]}"#),
        Some(r#"{"hiddenPaths":[]}"#),
    )
    .unwrap();
    assert!(
        cfg.hidden_paths.is_empty(),
        "workspace [] clears the user list"
    );
}

#[test]
fn layered_invalid_user_file_names_the_user_path() {
    let cases = [
        (
            r#"{"hiddenPaths":"notes"}"#,
            "hiddenPaths must be a string array",
        ),
        (
            r#"{"hiddenPaths":{"notes":true}}"#,
            "hiddenPaths must be a string array",
        ),
        (
            r#"{"hiddenPaths":["notes",1]}"#,
            "hiddenPaths must contain only strings",
        ),
        (
            r#"{"hiddenPaths":[],"maxDepth":0}"#,
            "maxDepth must be a positive integer",
        ),
        (
            r#"{"hiddenPaths":[],"viewDefaults":{"commitMessageLines":0}}"#,
            "viewDefaults.commitMessageLines must be an integer from 1 to 20",
        ),
        (
            r#"{"hiddenPaths":[],"viewDefaults":{"wrap":"on"}}"#,
            r#"viewDefaults.wrap must be "wrap" or "unwrap""#,
        ),
    ];
    for (json, tail) in cases {
        let (root, user_file, workspace) = layered(Some(json), Some(r#"{"hiddenPaths":[]}"#));
        let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
        assert_eq!(err, format!("{} {tail}", user_file.display()), "{json}");
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn layered_invalid_workspace_file_names_the_workspace_file() {
    let err = load_layered(
        Some(r#"{"hiddenPaths":[]}"#),
        Some(r#"{"hiddenPaths":[],"editor":1}"#),
    )
    .unwrap_err();
    assert_eq!(err, ".workspace-status-config.json editor must be a string");
}

#[test]
fn layered_unreadable_user_file_names_the_user_path() {
    let (root, user_file, workspace) = layered(None, None);
    fs::create_dir_all(&user_file).unwrap();
    let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
    assert!(
        err.starts_with(&format!("{}: ", user_file.display())),
        "{err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn user_config_path_prefers_xdg_config_home() {
    let env = |key: &str| match key {
        "XDG_CONFIG_HOME" => Some("/xdg/config".to_string()),
        "HOME" => Some("/home/demo".to_string()),
        _ => None,
    };
    assert_eq!(
        user_config_path_from_env(env),
        Some(PathBuf::from("/xdg/config/my-workspace-status/config.json"))
    );
}

#[test]
fn user_config_path_blank_xdg_falls_back_to_home() {
    for xdg in [None, Some(""), Some("   ")] {
        let env = |key: &str| match key {
            "XDG_CONFIG_HOME" => xdg.map(str::to_string),
            "HOME" => Some("/home/demo".to_string()),
            _ => None,
        };
        assert_eq!(
            user_config_path_from_env(env),
            Some(PathBuf::from(
                "/home/demo/.config/my-workspace-status/config.json"
            )),
            "XDG_CONFIG_HOME: {xdg:?}"
        );
    }
}

#[test]
fn user_config_path_without_xdg_or_home_is_none() {
    assert_eq!(user_config_path_from_env(|_| None), None);
    let blank = |key: &str| match key {
        "XDG_CONFIG_HOME" | "HOME" => Some(" ".to_string()),
        _ => None,
    };
    assert_eq!(user_config_path_from_env(blank), None);
}

#[test]
fn blank_xdg_config_home_reads_the_home_dot_config_file() {
    let (root, _, workspace) = layered(None, None);
    let home = root.join("home");
    let user_file = home
        .join(".config")
        .join(USER_CONFIG_DIR)
        .join(USER_CONFIG_FILENAME);
    fs::create_dir_all(user_file.parent().unwrap()).unwrap();
    fs::write(&user_file, r#"{"hiddenPaths":[],"maxDepth":7}"#).unwrap();
    let home_str = home.to_string_lossy().to_string();
    let path = user_config_path_from_env(|key| match key {
        "XDG_CONFIG_HOME" => Some(String::new()),
        "HOME" => Some(home_str.clone()),
        _ => None,
    });
    assert_eq!(path.as_deref(), Some(user_file.as_path()));
    let cfg = load_config_files(path.as_deref(), &workspace).unwrap();
    assert_eq!(cfg.max_depth, 7);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn layered_user_file_without_hidden_paths_loads() {
    for json in [
        r#"{"viewDefaults":{"wrap":"unwrap"}}"#,
        r#"{"hiddenPaths":null,"viewDefaults":{"wrap":"unwrap"}}"#,
    ] {
        let cfg = load_layered(Some(json), None).unwrap();
        assert!(cfg.hidden_paths.is_empty(), "{json}");
        assert_eq!(cfg.view_defaults.wrap, Some(false), "{json}");

        let cfg = load_layered(Some(json), Some(r#"{"hiddenPaths":["vendor"]}"#)).unwrap();
        assert_eq!(cfg.hidden_paths, vec!["vendor"], "{json}");
        assert_eq!(cfg.view_defaults.wrap, Some(false), "{json}");
    }
    assert_eq!(
        load_layered(Some("{}"), None).unwrap().max_depth,
        DEFAULT_MAX_DEPTH
    );
}

#[test]
fn layered_user_hidden_paths_apply_without_workspace_file() {
    let cfg = load_layered(Some(r#"{"hiddenPaths":["./notes/","vendor"]}"#), None).unwrap();
    assert_eq!(cfg.hidden_paths, vec!["notes", "vendor"]);
}

#[test]
fn workspace_file_hidden_paths_is_optional() {
    for json in [
        r#"{}"#,
        r#"{"hiddenPaths":null}"#,
        r#"{"maxDepth":2}"#,
        r#"{"ignoredPaths":["dotfiles","notes"],"editor":"cursor","diffTool":"cursor --diff --wait"}"#,
        r#"{"editor":"nvim"}"#,
    ] {
        let cfg = load_layered(Some(r#"{"hiddenPaths":["notes"]}"#), Some(json)).unwrap();
        assert_eq!(cfg.hidden_paths, vec!["notes"], "user list kept: {json}");

        let cfg = load_layered(None, Some(json)).unwrap();
        assert!(cfg.hidden_paths.is_empty(), "no default entries: {json}");
        if json.contains("ignoredPaths") {
            assert_eq!(cfg.ignored_paths, vec!["dotfiles", "notes"], "{json}");
        }
    }
}

#[test]
fn workspace_file_rejects_wrong_hidden_paths_type() {
    for json in [
        r#"{"hiddenPaths":"notes"}"#,
        r#"{"hiddenPaths":{}}"#,
        r#"{"hiddenPaths":[1]}"#,
    ] {
        let err = load_layered(None, Some(json)).unwrap_err();
        assert!(
            err.starts_with(".workspace-status-config.json hiddenPaths must "),
            "{json}: {err}"
        );
    }
}

#[test]
fn ignored_paths_parse_and_normalize_in_both_files() {
    let cfg = load_layered(
        None,
        Some(r#"{"hiddenPaths":["./notes/"],"ignoredPaths":[" tmp/ ","./vendor\\lib","tmp"]}"#),
    )
    .unwrap();
    assert_eq!(cfg.hidden_paths, vec!["notes"]);
    assert_eq!(cfg.ignored_paths, vec!["tmp", "vendor/lib"]);

    let cfg = load_layered(Some(r#"{"ignoredPaths":["tmp"]}"#), None).unwrap();
    assert!(cfg.hidden_paths.is_empty());
    assert_eq!(cfg.ignored_paths, vec!["tmp"]);
}

#[test]
fn ignored_paths_is_optional_in_both_files() {
    for json in [
        r#"{"hiddenPaths":[]}"#,
        r#"{"hiddenPaths":[],"ignoredPaths":null}"#,
    ] {
        let cfg = load_layered(Some(r#"{"ignoredPaths":null}"#), Some(json)).unwrap();
        assert!(cfg.ignored_paths.is_empty(), "{json}");
    }
}

#[test]
fn layered_ignored_paths_are_replaced_not_joined() {
    let cfg = load_layered(
        Some(r#"{"hiddenPaths":["notes"],"ignoredPaths":["tmp","vendor"]}"#),
        Some(r#"{"hiddenPaths":["scratch"],"ignoredPaths":["build"]}"#),
    )
    .unwrap();
    assert_eq!(cfg.hidden_paths, vec!["scratch"]);
    assert_eq!(cfg.ignored_paths, vec!["build"]);

    let cfg = load_layered(
        Some(r#"{"ignoredPaths":["tmp"]}"#),
        Some(r#"{"hiddenPaths":[],"ignoredPaths":[]}"#),
    )
    .unwrap();
    assert!(
        cfg.ignored_paths.is_empty(),
        "workspace [] clears the user list"
    );

    let cfg = load_layered(
        Some(r#"{"ignoredPaths":["tmp"]}"#),
        Some(r#"{"hiddenPaths":["notes"]}"#),
    )
    .unwrap();
    assert_eq!(
        cfg.ignored_paths,
        vec!["tmp"],
        "workspace file without ignoredPaths keeps the user list"
    );
}

#[test]
fn path_list_entries_must_be_workspace_relative() {
    for key in ["hiddenPaths", "ignoredPaths"] {
        for raw in [
            r#""/abs""#,
            r#""\\share\\x""#,
            r#""~/notes""#,
            r#""C:\\work""#,
            r#""c:/work""#,
            r#""..""#,
            r#""../sibling""#,
            r#""tmp/../x""#,
            r#""tmp\\..\\x""#,
            r#"" /abs""#,
        ] {
            let json = if key == "hiddenPaths" {
                format!(r#"{{"hiddenPaths":[{raw}]}}"#)
            } else {
                format!(r#"{{"hiddenPaths":[],"{key}":[{raw}]}}"#)
            };
            let err = load_layered(None, Some(&json)).unwrap_err();
            assert!(
                    err.starts_with(&format!(
                        ".workspace-status-config.json {key} entries must be paths relative to the workspace root"
                    )),
                    "{json}: {err}"
                );
            let user_json = format!(r#"{{"{key}":[{raw}]}}"#);
            let err = load_layered(Some(&user_json), None).unwrap_err();
            assert!(
                err.contains(&format!(" {key} entries must be paths relative")),
                "{err}"
            );
        }
    }
    let cfg = load_layered(
        None,
        Some(r#"{"hiddenPaths":["a..b/x"],"ignoredPaths":["..hidden"]}"#),
    )
    .unwrap();
    assert_eq!(
        cfg.hidden_paths,
        vec!["a..b/x"],
        "`..` inside a name is fine"
    );
    assert_eq!(cfg.ignored_paths, vec!["..hidden"]);
}

#[test]
fn path_list_rejects_non_string_entries() {
    let err =
        load_layered(None, Some(r#"{"hiddenPaths":[],"ignoredPaths":["tmp",1]}"#)).unwrap_err();
    assert_eq!(
        err,
        ".workspace-status-config.json ignoredPaths must contain only strings"
    );
    let err = load_layered(None, Some(r#"{"hiddenPaths":[],"ignoredPaths":"tmp"}"#)).unwrap_err();
    assert_eq!(
        err,
        ".workspace-status-config.json ignoredPaths must be a string array"
    );
}

#[test]
fn removed_ignored_repos_key_fails_in_both_files() {
    let want_tail = "ignoredRepos is no longer supported: use hiddenPaths (hidden by default) or ignoredPaths (never shown)";
    for json in [
        r#"{"ignoredRepos":["notes"]}"#,
        r#"{"ignoredRepos":[]}"#,
        r#"{"ignoredRepos":null}"#,
        r#"{"hiddenPaths":[],"ignoredRepos":["notes"]}"#,
    ] {
        let err = load_layered(None, Some(json)).unwrap_err();
        assert_eq!(
            err,
            format!(".workspace-status-config.json {want_tail}"),
            "{json}"
        );
        let (root, user_file, workspace) = layered(Some(json), None);
        let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
        assert_eq!(
            err,
            format!("{} {want_tail}", user_file.display()),
            "{json}"
        );
        let _ = fs::remove_dir_all(root);
    }
}

fn load_layered_home(
    user_json: Option<&str>,
    workspace_json: Option<&str>,
    home: Option<&str>,
) -> Result<WorkspaceStatusConfig, String> {
    let (root, user_file, workspace) = layered(user_json, workspace_json);
    let out = load_config_files_with_home(Some(&user_file), &workspace, home);
    let _ = fs::remove_dir_all(root);
    out
}

/// Load `{<key>: <raw>}` as the user file (every runtime key is allowed
/// there). Errors start with `<user>` in place of the temp user path.
fn load_runtime_key(key: &str, raw: &str) -> Result<RuntimeKeys, String> {
    let (root, user_file, workspace) = layered(Some(&format!(r#"{{"{key}":{raw}}}"#)), None);
    let out = load_config_files_with_home(Some(&user_file), &workspace, Some("/home/demo"))
        .map(|cfg| cfg.runtime)
        .map_err(|e| e.replace(&user_file.display().to_string(), "<user>"));
    let _ = fs::remove_dir_all(root);
    out
}

#[test]
fn runtime_keys_omitted_or_null_are_none() {
    assert_eq!(
        load_runtime_key("theme", "null").unwrap(),
        RuntimeKeys::default()
    );
    let cfg = load_layered(None, Some(r#"{"hiddenPaths":[]}"#)).unwrap();
    assert_eq!(cfg.runtime, RuntimeKeys::default());
    assert_eq!(
        WorkspaceStatusConfig::with_defaults().runtime,
        RuntimeKeys::default()
    );
}

#[test]
fn theme_key_accepts_every_built_in_slug() {
    for id in THEME_IDS {
        assert_eq!(
            load_runtime_key("theme", &format!(r#""{}""#, id.as_str()))
                .unwrap()
                .theme,
            Some(id),
            "{id:?}"
        );
    }
}

#[test]
fn config_theme_accepts_light_slugs() {
    for (slug, id) in [
        ("tokyo-night-day", ThemeId::TokyoNightDay),
        ("solarized-light", ThemeId::SolarizedLight),
        ("catppuccin-latte", ThemeId::CatppuccinLatte),
        ("github-light", ThemeId::GithubLight),
    ] {
        assert_eq!(
            load_runtime_key("theme", &format!(r#""{slug}""#))
                .unwrap()
                .theme,
            Some(id),
            "{slug}"
        );
    }
    let cfg = load_layered_home(
        Some(r#"{"theme":"one-light"}"#),
        Some(r#"{"hiddenPaths":[],"theme":"rose-pine-dawn"}"#),
        Some("/home/demo"),
    )
    .unwrap();
    assert_eq!(cfg.runtime.theme, Some(ThemeId::RosePineDawn));
}

#[test]
fn theme_mode_key_parses_and_rejects_bad_values() {
    for (raw, want) in [
        (r#""auto""#, ThemeMode::Auto),
        (r#""dark""#, ThemeMode::Dark),
        (r#"" light ""#, ThemeMode::Light),
    ] {
        assert_eq!(
            load_runtime_key("themeMode", raw).unwrap().theme_mode,
            Some(want),
            "{raw}"
        );
    }
    assert_eq!(
        load_runtime_key("themeMode", "null").unwrap().theme_mode,
        None
    );
    for raw in [r#""Dark""#, r#""system""#, r#""""#, "true"] {
        assert_eq!(
            load_runtime_key("themeMode", raw).unwrap_err(),
            r#"<user> themeMode must be one of "auto", "dark", "light""#,
            "{raw}"
        );
    }
    let cfg = load_layered_home(
        Some(r#"{"themeMode":"light"}"#),
        Some(r#"{"hiddenPaths":[],"themeMode":"dark"}"#),
        Some("/home/demo"),
    )
    .unwrap();
    assert_eq!(
        cfg.runtime.theme_mode,
        Some(ThemeMode::Dark),
        "workspace wins"
    );
}

#[test]
fn runtime_keys_parse_valid_values() {
    let cfg = load_layered_home(
            Some(
                r#"{"theme":" dracula ","themeMode":"light","glyphs":"ascii","watchMs":0,"fetchMs":60000,"fetchConcurrency":4,"updateCheck":false,"updateCheckStore":"~/state/update-check.json","commentStore":"/var/ws/comments.json","viewedStore":"~/viewed.json","uiStore":"~/state/ui-state.json","git":"git-wrapper","termProgress":false,"smoothScroll":false,"prPolling":false,"prTitleCommand":"~/bin/pr-title"}"#,
            ),
            None,
            Some("/home/demo"),
        )
        .unwrap();
    assert_eq!(
        cfg.runtime,
        RuntimeKeys {
            theme: Some(ThemeId::Dracula),
            theme_mode: Some(ThemeMode::Light),
            glyphs: Some(GlyphSet::Ascii),
            watch_ms: Some(0),
            fetch_ms: Some(60000),
            fetch_concurrency: Some(4),
            update_check: Some(false),
            update_check_store: Some(PathBuf::from("/home/demo/state/update-check.json")),
            comment_store: Some(PathBuf::from("/var/ws/comments.json")),
            viewed_store: Some(PathBuf::from("/home/demo/viewed.json")),
            ui_store: Some(PathBuf::from("/home/demo/state/ui-state.json")),
            git: Some(PathBuf::from("git-wrapper")),
            term_progress: Some(false),
            smooth_scroll: Some(false),
            pr_polling: Some(false),
            pr_title_command: Some(PathBuf::from("/home/demo/bin/pr-title")),
        }
    );
    assert_eq!(
        load_runtime_key("prPolling", "true").unwrap().pr_polling,
        Some(true)
    );
    for (raw, want) in [
        (r#""/opt/bin/pr-title""#, "/opt/bin/pr-title"),
        (r#""pr-title""#, "pr-title"),
    ] {
        assert_eq!(
            load_runtime_key("prTitleCommand", raw)
                .unwrap()
                .pr_title_command,
            Some(PathBuf::from(want)),
            "prTitleCommand: {raw}"
        );
    }
    assert_eq!(
        load_runtime_key("glyphs", r#""nerd""#).unwrap().glyphs,
        Some(GlyphSet::Nerd)
    );
    assert_eq!(
        load_runtime_key("updateCheck", "true")
            .unwrap()
            .update_check,
        Some(true)
    );
    for (raw, want) in [
        (r#""/usr/local/bin/git""#, "/usr/local/bin/git"),
        (r#""~/bin/git""#, "/home/demo/bin/git"),
        (r#""git""#, "git"),
    ] {
        assert_eq!(
            load_runtime_key("git", raw).unwrap().git,
            Some(PathBuf::from(want)),
            "git: {raw}"
        );
    }
}

#[test]
fn smooth_scroll_config_false_turns_it_off() {
    let smooth = |raw: &str| {
        let keys = load_runtime_key("smoothScroll", raw).unwrap();
        // A state dir that does not exist: no last-used theme is read.
        let env = |key: &str| (key == "XDG_STATE_HOME").then(|| "/nonexistent/state".into());
        crate::settings::Settings::resolve(&keys, env).smooth_scroll
    };
    assert!(!smooth("false"), "config false turns the glide off");
    assert!(smooth("true"));
    assert!(smooth("null"), "an unset key keeps the default on");
}

#[test]
fn runtime_keys_bad_values_name_the_file_key_and_choices() {
    let themes = r#"must be one of "tokyo-night", "monokai", "dracula", "gruvbox-dark", "catppuccin-mocha", "slate", "solarized-dark", "nord", "rose-pine", "kanagawa", "everforest", "one-dark", "github-dark-dimmed", "tokyo-night-day", "monokai-light", "dracula-light", "gruvbox-light", "catppuccin-latte", "slate-light", "solarized-light", "nord-light", "rose-pine-dawn", "kanagawa-lotus", "everforest-light", "one-light", "github-light""#;
    let path_rule = "must be an absolute path or a path that starts with ~/";
    let git_rule =
        "must be an absolute path, a path that starts with ~/, or a command name without /";
    let cases: Vec<(&str, &str, String)> = vec![
        ("theme", r#""solarized""#, themes.to_string()),
        ("theme", r#""Dracula""#, themes.to_string()),
        ("theme", r#""""#, themes.to_string()),
        ("theme", "1", themes.to_string()),
        (
            "glyphs",
            r#""unicode""#,
            r#"must be "nerd" or "ascii""#.to_string(),
        ),
        (
            "glyphs",
            r#""  ""#,
            r#"must be "nerd" or "ascii""#.to_string(),
        ),
        ("glyphs", "true", r#"must be "nerd" or "ascii""#.to_string()),
        ("watchMs", "-1", "must be an integer >= 0".to_string()),
        (
            "watchMs",
            r#""3000""#,
            "must be an integer >= 0".to_string(),
        ),
        ("watchMs", "1.5", "must be an integer >= 0".to_string()),
        ("fetchMs", "-5", "must be an integer >= 0".to_string()),
        ("fetchMs", "false", "must be an integer >= 0".to_string()),
        (
            "fetchConcurrency",
            "0",
            "must be an integer >= 1".to_string(),
        ),
        (
            "fetchConcurrency",
            r#""8""#,
            "must be an integer >= 1".to_string(),
        ),
        ("updateCheck", "0", "must be true or false".to_string()),
        (
            "termProgress",
            r#""off""#,
            "must be true or false".to_string(),
        ),
        ("smoothScroll", "0", "must be true or false".to_string()),
        (
            "updateCheck",
            r#""false""#,
            "must be true or false".to_string(),
        ),
        (
            "updateCheckStore",
            r#""state/check.json""#,
            path_rule.to_string(),
        ),
        ("updateCheckStore", r#""""#, path_rule.to_string()),
        (
            "commentStore",
            r#""./comments.json""#,
            path_rule.to_string(),
        ),
        ("commentStore", "1", path_rule.to_string()),
        ("viewedStore", r#""~viewed.json""#, path_rule.to_string()),
        ("uiStore", r#""ui-state.json""#, path_rule.to_string()),
        ("uiStore", r#""  ""#, path_rule.to_string()),
        ("uiStore", "true", path_rule.to_string()),
        ("git", r#""bin/git""#, git_rule.to_string()),
        ("git", r#""./git""#, git_rule.to_string()),
        ("git", r#""~""#, git_rule.to_string()),
        ("git", r#""   ""#, git_rule.to_string()),
        ("git", "[]", git_rule.to_string()),
        ("prPolling", "0", "must be true or false".to_string()),
        ("prPolling", r#""off""#, "must be true or false".to_string()),
        ("prTitleCommand", r#""bin/pr-title""#, git_rule.to_string()),
        ("prTitleCommand", r#""  ""#, git_rule.to_string()),
        ("prTitleCommand", "true", git_rule.to_string()),
    ];
    for (key, raw, tail) in cases {
        let err = load_runtime_key(key, raw).unwrap_err();
        assert_eq!(err, format!("<user> {key} {tail}"), "{key}: {raw}");
    }
    let err = load_layered(None, Some(r#"{"hiddenPaths":[],"glyphs":"unicode"}"#)).unwrap_err();
    assert_eq!(
        err,
        r#".workspace-status-config.json glyphs must be "nerd" or "ascii""#
    );
    let (root, user_file, workspace) = layered(Some(r#"{"fetchConcurrency":0}"#), None);
    let err = load_config_files_with_home(Some(&user_file), &workspace, None).unwrap_err();
    assert_eq!(
        err,
        format!(
            "{} fetchConcurrency must be an integer >= 1",
            user_file.display()
        )
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runtime_path_keys_need_home_for_tilde() {
    for home in [None, Some(""), Some("  ")] {
        let (root, user_file, workspace) =
            layered(Some(r#"{"commentStore":"~/comments.json"}"#), None);
        let err = load_config_files_with_home(Some(&user_file), &workspace, home).unwrap_err();
        assert_eq!(
            err,
            format!(
                "{} commentStore starts with ~/ but HOME is not set",
                user_file.display()
            ),
            "HOME: {home:?}"
        );
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn runtime_keys_merge_per_key_workspace_wins() {
    let cfg = load_layered_home(
        Some(r#"{"theme":"dracula","watchMs":4000,"git":"/opt/git"}"#),
        Some(r#"{"hiddenPaths":[],"theme":"monokai","fetchMs":0}"#),
        Some("/home/demo"),
    )
    .unwrap();
    assert_eq!(cfg.runtime.theme, Some(ThemeId::Monokai), "workspace wins");
    assert_eq!(cfg.runtime.watch_ms, Some(4000), "user key kept");
    assert_eq!(cfg.runtime.fetch_ms, Some(0));
    assert_eq!(cfg.runtime.git, Some(PathBuf::from("/opt/git")));
}

#[test]
fn invalid_json_is_a_json_error_naming_the_file() {
    for json in ["{", "[]", r#""notes""#, "{\"theme\": }"] {
        let (root, user_file, workspace) = layered(Some(json), None);
        let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
        let prefix = format!("{} is not a valid JSON object: ", user_file.display());
        assert!(err.starts_with(&prefix), "{json}: {err}");
        assert!(err.len() > prefix.len(), "{json}: serde message missing");
        let _ = fs::remove_dir_all(root);
    }
    for json in ["{", "[]"] {
        let err = load_layered(None, Some(json)).unwrap_err();
        let prefix = ".workspace-status-config.json is not a valid JSON object: ";
        assert!(err.starts_with(prefix), "{json}: {err}");
        assert!(err.len() > prefix.len(), "{json}: serde message missing");
    }
}

#[cfg(unix)]
#[test]
fn dangling_user_file_symlink_is_an_error_not_a_missing_file() {
    let (root, user_file, workspace) = layered(None, None);
    std::os::unix::fs::symlink(root.join("no-such-target.json"), &user_file).unwrap();
    let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
    assert!(
        err.starts_with(&format!(
            "{}: symlink target is missing: ",
            user_file.display()
        )),
        "{err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_file_rejects_user_only_keys() {
    for (key, raw, env_var) in [
        ("git", r#""sh""#, "WORKSPACE_STATUS_GIT"),
        ("git", r#""/usr/bin/git""#, "WORKSPACE_STATUS_GIT"),
        ("git", "1", "WORKSPACE_STATUS_GIT"),
        (
            "updateCheckStore",
            r#""~/.bashrc""#,
            "WS_STATUS_UPDATE_CHECK_STORE",
        ),
        (
            "commentStore",
            r#""/tmp/c.json""#,
            "WS_STATUS_COMMENT_STORE",
        ),
        ("viewedStore", r#""/tmp/v.json""#, "WS_STATUS_VIEWED_STORE"),
        ("uiStore", r#""~/ui-state.json""#, "WS_STATUS_UI_STORE"),
        (
            "prTitleCommand",
            r#""/usr/local/bin/pr-title""#,
            "WS_STATUS_PR_TITLE_CMD",
        ),
        ("prTitleCommand", r#""sh""#, "WS_STATUS_PR_TITLE_CMD"),
    ] {
        let workspace_json = format!(r#"{{"hiddenPaths":[],"{key}":{raw}}}"#);
        let (root, user_file, workspace) = layered(None, Some(&workspace_json));
        let err = load_config_files_with_home(Some(&user_file), &workspace, Some("/home/demo"))
            .unwrap_err();
        assert_eq!(
                err,
                format!(
                    ".workspace-status-config.json {key} is only allowed in the user config file ({}) or {env_var}",
                    user_file.display()
                ),
                "{key}: {raw}"
            );
        let err = load_config_files_with_home(None, &workspace, Some("/home/demo")).unwrap_err();
        assert_eq!(
                err,
                format!(
                    ".workspace-status-config.json {key} is only allowed in the user config file (~/.config/my-workspace-status/config.json) or {env_var}"
                ),
                "no user path: {key}"
            );
        let _ = fs::remove_dir_all(root);
    }
    let cfg = load_layered(
            None,
            Some(
                r#"{"hiddenPaths":[],"git":null,"commentStore":null,"uiStore":null,"prTitleCommand":null}"#,
            ),
        )
        .unwrap();
    assert_eq!(
        cfg.runtime,
        RuntimeKeys::default(),
        "null counts as omitted"
    );
}

#[test]
fn pr_polling_is_allowed_in_either_file_and_the_workspace_wins() {
    let cfg = load_layered(None, Some(r#"{"hiddenPaths":[],"prPolling":false}"#)).unwrap();
    assert_eq!(cfg.runtime.pr_polling, Some(false), "workspace file");
    let cfg = load_layered(Some(r#"{"prPolling":false}"#), None).unwrap();
    assert_eq!(cfg.runtime.pr_polling, Some(false), "user file");
    let cfg = load_layered(
        Some(r#"{"prPolling":false}"#),
        Some(r#"{"hiddenPaths":[],"prPolling":true}"#),
    )
    .unwrap();
    assert_eq!(cfg.runtime.pr_polling, Some(true), "workspace wins");
}

#[test]
fn user_file_sets_user_only_keys_under_a_workspace_file() {
    let cfg = load_layered_home(
            Some(
                r#"{"git":"git-wrapper","updateCheckStore":"~/u.json","commentStore":"/c.json","viewedStore":"/v.json","uiStore":"~/ui.json","prTitleCommand":"~/bin/pr-title"}"#,
            ),
            Some(r#"{"hiddenPaths":[],"theme":"monokai"}"#),
            Some("/home/demo"),
        )
        .unwrap();
    assert_eq!(cfg.runtime.git, Some(PathBuf::from("git-wrapper")));
    assert_eq!(
        cfg.runtime.pr_title_command,
        Some(PathBuf::from("/home/demo/bin/pr-title"))
    );
    assert_eq!(
        cfg.runtime.update_check_store,
        Some(PathBuf::from("/home/demo/u.json"))
    );
    assert_eq!(cfg.runtime.comment_store, Some(PathBuf::from("/c.json")));
    assert_eq!(cfg.runtime.viewed_store, Some(PathBuf::from("/v.json")));
    assert_eq!(
        cfg.runtime.ui_store,
        Some(PathBuf::from("/home/demo/ui.json"))
    );
    assert_eq!(cfg.runtime.theme, Some(ThemeId::Monokai));
}

#[test]
fn user_file_with_a_leftover_theme_store_key_ignores_it() {
    let cfg = load_layered_home(
        Some(r#"{"theme":"nord","themeStore":"~/state/theme.json"}"#),
        Some(r#"{"hiddenPaths":[]}"#),
        Some("/home/demo"),
    )
    .unwrap();
    assert_eq!(cfg.runtime.ui_store, None, "themeStore is not uiStore");
    assert_eq!(
        cfg.runtime,
        RuntimeKeys {
            theme: Some(ThemeId::Nord),
            ..RuntimeKeys::default()
        }
    );
}
