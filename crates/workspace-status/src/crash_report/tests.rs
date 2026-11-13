use super::crash_footer;

#[test]
fn crash_footer_names_version_platform_size_and_tracker() {
    assert_eq!(
        crash_footer("v1.2.3", "macos", "aarch64", Some((80, 24)), None),
        "\nws v1.2.3 crashed (macos aarch64, terminal 80x24).\n\
         Please report it at https://github.com/joboyx/my-workspace-status/issues with this whole output.\n"
    );
}

#[test]
fn crash_footer_without_terminal_says_unknown() {
    assert_eq!(
        crash_footer("v1.2.3", "linux", "x86_64", None, None),
        "\nws v1.2.3 crashed (linux x86_64, terminal unknown).\n\
         Please report it at https://github.com/joboyx/my-workspace-status/issues with this whole output.\n"
    );
}

#[test]
fn crash_footer_appends_backtrace_when_given() {
    assert_eq!(
        crash_footer("v1.2.3", "macos", "aarch64", Some((10, 3)), Some("   0: main\n")),
        "\nws v1.2.3 crashed (macos aarch64, terminal 10x3).\n\
         Please report it at https://github.com/joboyx/my-workspace-status/issues with this whole output.\n\
         \nstack backtrace:\n   0: main\n"
    );
}
