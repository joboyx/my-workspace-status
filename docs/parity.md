# Cross-surface parity

A shared TUI component (a commit-message footer, a file list, a scrollbar, a search highlight, …) often paints on more than one surface: another tab kind, another drill depth, or the other pane. The default is that a component has the same features and the same behaviour on every surface where it appears. This file is the registry of those surfaces. It records, for each component, every surface and, per feature, whether that surface has it, does not need it (with the reason), or lacks it (an open gap).

**Rule.** A change to the behaviour of a component listed here checks every surface in that component's table. The change applies on every surface by default. Each surface that does not take the change gets an `n/a: <reason>` cell here, and the pull request text says why. A new surface or a new feature gets a row or a column here in the same change. A gap found and not fixed gets a `gap:` cell and a row in **Open gaps**.

CI runs the checks in `crates/workspace-status/src/parity_registry.rs` (part of `cargo test --workspace`), so this file cannot drift silently from the code:

| Check | Fails when |
| --- | --- |
| `parity_registry_cells_are_supported_na_or_gap` | The format below is broken: a cell is empty or not one of the three forms, an `n/a:` has no reason, a gap id is malformed, a look is unknown, a surface table's feature columns differ from the feature table, or a surface or feature is listed twice. |
| `parity_registry_gaps_and_intentional_differences_are_indexed` | A `gap:` id is not in **Open gaps**, an indexed gap is used nowhere, a gap is used in another component with no note, or an `I<n>` cited in an `n/a:` reason and the **Intentional differences** table do not match. |
| `parity_registry_code_anchors_and_tests_exist` | A code anchor or a named test is no longer a `fn` anywhere under `crates/` (a mention in a `//` comment does not count). A rename or a moved code path forces an update here. |
| `commit_footer_message_scroll_matches_on_every_registry_surface` (`tui/render.rs`) | The `commit-footer` surfaces marked `supported` for `msg-scroll`, in each look, differ from the surfaces that test drives; or the wheel, scrollbar, more-below hint, or per-commit reset fails on one of them. |
| `commit_footer_rows_match_on_every_registry_surface`, `commit_footer_rule_drag_matches_on_every_registry_surface` (`tui/render.rs`) | The `commit-footer` surfaces marked `supported` for `meta`, `msg-colour`, `row-order`, or `rule-drag`, in each look, differ from the surfaces those tests drive; or on one of them the message rows are not on top in `pal.repo`, the meta row is not the graph footer's meta on the bottom row with a ref chip hit that pins its popover, or the top rule does not drag the rows and save them once on release. |
| `commit_footer_shows_hint_expand_and_hash_link_on_every_registry_surface` (`tui/render.rs`) | The `commit-footer` surfaces marked `supported` for `shown`, `hint-popover`, `expand`, or `hash-link`, in each look, differ from the surfaces that test drives; or on one of them the subject does not paint on the first body row, a click on the more-below hint does not pin the more-lines section with the footer's count, `M` does not hide and restore the body, or the meta row's short hash does not link to the commit page. |
| `commit_file_search_highlight_and_steps_agree_on_every_registry_surface` (`tui/render.rs`) | The `search` surfaces whose code anchors name `commit_file_search_match_ids` differ from the surfaces that test drives, or one of them is not `supported` for `agree` in each look; or on one of them the highlighted rows differ from the rows `n` / `N` visit and the pill counts, or a walk from a folded list changes the count. |
| `diff_vertical_bar_drags_and_track_click_jumps_on_every_registry_surface` (`tui/render.rs`) | The `scrollbars` surfaces whose code anchors name `draw_diff_pane` differ from the surfaces that test drives, or one of them is not `supported` for `vbar-drag` in each look; or on one of them a press on the vertical thumb picks a row, arms a text selection, or jumps, a drag does not move the viewport (and the diff cursor with it) by the graph bar's math, the release does not end the drag, or a track click does not jump to the end or the top. |
| `list_vertical_bar_drags_and_track_click_jumps_on_every_registry_surface`, `list_horizontal_bar_drags_and_track_click_jumps_on_every_registry_surface` (`tui/render.rs`) | The `scrollbars` surfaces whose code anchors name `draw_tree`, `draw_commit_file_list`, `draw_folder_summary`, `draw_explorer_tree`, `draw_prs_list`, `draw_prs_detail`, or `draw_file_body` (the clean-file view) differ from the surfaces those tests drive, or one of them (for the horizontal bar, one that pans) is not `supported` for `vbar` and `vbar-drag` (`hbar` and `hbar-drag`) in each look; or on one of them a bar does not paint where its rows leave room for it, a press on a thumb picks a row, a drag does not move the viewport (and the list cursor with it, asking for what the wheel asks for; the folder summary and the PRs detail have no cursor) or the pan by the graph bar's math, the horizontal bar clamps to another pan max than Shift-→ or hides the end of the longest row, the release does not end the drag, or a track click does not jump to the end or the start. |
| `diff_horizontal_bar_drags_and_track_click_jumps_on_every_registry_surface` (`tui/render.rs`) | The `scrollbars` surfaces whose code anchors name `draw_diff_pane` differ from the surfaces that test drives, or one of them is not `supported` for `hbar` and `hbar-drag` in each look; or on one of them the bar paints at the left edge, clamps to another pan max than Shift-→, a thumb press picks a row or arms a text selection, a drag does not pan by the graph bar's math, the release does not end the drag, or a track click does not jump to the end or the start. |
| `list_shift_keys_and_hwheel_pan_on_every_registry_list_surface` (`tui/render.rs`) | A `scrollbars` list surface that pans (the list scrollbar tests' surfaces with a horizontal bar) is not a `horizontal-pan` surface, or one of them is not `supported` for `shift-keys`, `hwheel`, and `pan-limit` in each look; or on one of them Shift-→ does not pan the focused list or does not stop at its pan max, or the horizontal wheel over the list (the other pane focused) does not pan it, stops elsewhere than Shift-→, or moves its cursor. |
| `tree_hwheel_pans_a_shown_file_view_that_can_pan_on_every_registry_tree_surface` (`tui/render.rs`) | The `horizontal-pan` tree surfaces (`tree`, `explorer-tree`) are not `supported` for `hwheel` in each look; or on one of them the horizontal wheel over the tree does not pan a shown file view that can pan (leaving the tree still), does not pan the tree once the view cannot pan, or Shift-→ with the tree focused pans anything but the tree. |
| `line_click_picks_the_row_and_a_drag_selects_on_every_registry_surface` (`tui/render.rs`) | The `line-viewers` surfaces differ from the surfaces that test drives, or one of them is not `supported` for `click-row` in each look; or on one of them a click on a line does not move the line cursor there and focus the viewer, or a press-drag from it does not select text (copied on release) or moves the cursor again. |
| `resize_edges_drag_and_hover_on_every_registry_surface` (`tui/render.rs`) | The `resize-edges` surfaces (except one that is `n/a` on every feature, with no edge to drive) differ from the surfaces that test drives, or one of them is not `supported` for `divider`, `diff-rule`, and `hover` in each look; or on one of them the divider or the side-by-side RULE under the pointer does not hover and light, a drag does not widen the left pane or move the RULE (painted where it was dropped), or the release does not end the drag. |
| `tree_rows_paint_the_same_icons_marks_flash_and_popovers_on_every_registry_tree_surface` (`tui/render.rs`) | The `file-lists` (`row-icons`), `icon-popovers` (`icon-hits`, `focused-row`), `row-marks` (`reviewed`, `comment`), or `watch-flash` (`flash`) surfaces whose code anchors name `draw_tree` or `draw_explorer_tree` differ from the trees that test drives (`tree`, `explorer-tree`), or one of them is not `supported` in each look; or on one of them a file or folder row paints other icons, glyphs, or order than on the other tree, a hit is wider than its glyph (also panned), a reloaded row does not flash, a hover does not find the icon, a click on a glyph does not pin its section (or a folder glyph changes the fold), `gh` does not pin one section per icon, or a section differs from the other tree's. |
| `line_gutter_comment_marks_peek_pin_and_gh_on_every_registry_surface` (`tui/render.rs`) | The `icon-popovers` surfaces whose code anchors name `draw_diff_pane` or `draw_file_body` differ from the surfaces that test drives, or one of them is not `supported` for `icon-hits` and `focused-row` in each look; or on one of them (each diff tab, inline and side by side, panned or not) a comment mark records no hit or a hit wider than its glyph, a hover does not peek the comments of that line only, a click does not select the line and pin with no text selection or RULE drag, or `gh` on the marked cursor line does not pin it. |

The `commit-footer` check is the pattern for other components: one test body that runs the same assertions on every surface the registry marks `supported`, and that compares its own surface list to `parity_registry::supported_surface_looks(component, feature)`.

## Format

The checks parse this file. Keep to these rules:

1. Each component is a `### ` heading under `## Components` that starts with a backticked kebab-case id, for example `` ### `commit-footer` — Commit message footer ``. A short description may follow.
2. Each component has exactly two tables, in this order:
   - **Features**: header `| Feature | What | Tests |`. Feature is a backticked kebab-case id. Tests is `none` or backticked test `fn` names, comma-separated.
   - **Surfaces**: header `| Surface | Tab | Depth | Pane | Looks | Code anchors |`, then one column per feature, named by its backticked id in feature-table order. Surface is a backticked kebab-case id, unique in the component. Tab, Depth, and Pane are free text (`-` for none). Looks is `boxed`, `flat`, or `boxed, flat`: the pane looks in which the surface paints (`viewDefaults.background` `"terminal"` paints boxed panes, `"paint"` flat ones; their geometry and hit tests differ). A `supported` cell is verified in a look only where the feature's Tests column names a test that runs that look; elsewhere it records the code path, not a per-look test. Looks is per row, not per cell: a feature that works in one look and not the other splits the surface into two rows (for example `files-d2-boxed` and `files-d2-flat`), each with its own cells. Code anchors is one or more backticked `fn` names of the surface's code path, comma-separated.
3. Each feature cell is exactly one of:
   - `supported`: the surface has the feature.
   - `n/a: <reason>`: the feature does not apply. The reason has two words or more. An intentional difference cites its id (`I3`) in the reason.
   - `gap: G<n>`: an open gap (`n` from 1, no leading zero). A gap belongs to the component that **Open gaps** names; a cell in another component adds a note after `;` (for example `gap: G99; <why it is missing here>`).
4. **Open gaps** has one row per gap: `| Gap | Component | Has it | Missing on |`. **Intentional differences** has one row per `I<n>`: `| Id | Difference | Stated in |`.
5. No `|` inside a cell.

## Components

### `commit-footer` — Commit message footer

The selected commit's message, pinned under a commit list: the graph selection footer (graph crate `GraphWidget`) and the commit-files footer of a depth-2 drill or a Diff commit vs parent compare tab (`render::draw_commit_detail` → `draw_commit_footer`; `commit_footer_source` gives the drilled source or the tab's head commit). Both build their lines with `selection_footer_parts` (for the commit-files footer, the graph row of that source, `commit_footer_parts`) and paint them with the shared `paint_selection_footer` (message rows through `paint_footer_message`, the meta row with its ref chips pinned at the bottom), so the same commit paints the same rows on every surface. A range compare tab (`compare-range`) has no single commit message and paints no footer. They keep one scroll offset, `commit_msg_scroll`, keyed by the commit's graph row identity, and their rule drags change the one `commit_msg_lines` setting.

| Feature | What | Tests |
| --- | --- | --- |
| `shown` | The footer paints under the list for a commit-backed view. | `depth_2_left_files_pane_pins_commit_footer_under_the_list`, `depth_1_files_pane_has_no_commit_footer_beside_the_graph_footer`, `commit_footer_shows_hint_expand_and_hash_link_on_every_registry_surface`, `compare_range_tab_paints_no_commit_footer` |
| `msg-scroll` | A long message scrolls with the wheel over the footer (the list cursor stays), shows a 1-column scrollbar and a ` <more_below><K>` hint, another commit starts at the top, and the last-scrolled commit keeps its offset (the same offset on every surface that shows it). | `commit_footer_message_scroll_matches_on_every_registry_surface`, `compare_commit_footer_shares_the_message_offset_with_graph_and_depth_2`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph` |
| `hint-popover` | The more-below hint hovers and pins the more-lines popover (Collapse, Taller). | `commit_detail_footer_hint_records_a_hit_and_pins_the_more_lines_section`, `commit_footer_shows_hint_expand_and_hash_link_on_every_registry_surface`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph` |
| `expand` | `M` toggles the full message and the subject-only footer. | `m_toggles_commit_msg_expand_on_graph_footer_and_commit_detail`, `commit_footer_shows_hint_expand_and_hash_link_on_every_registry_surface` |
| `hash-link` | The short hash links to the forge commit page (OSC 8). | `commit_detail_footer_hash_links_to_forge_commit_page`, `commit_footer_shows_hint_expand_and_hash_link_on_every_registry_surface` |
| `rule-drag` | A drag on the footer's top rule adds or removes message rows. | `commit_msg_rule_drag_resizes_the_footer_and_release_saves_once`, `commit_footer_rule_drag_matches_on_every_registry_surface`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph` |
| `meta` | The meta row has coloured ref chips with popovers and `parent <id>`. | `commit_footer_rows_match_on_every_registry_surface`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph`, `compare_commit_footer_paints_a_head_tab_commit_like_the_graph_row` |
| `msg-colour` | The message text paints in the subject colour (`pal.repo`). | `commit_footer_rows_match_on_every_registry_surface`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph` |
| `row-order` | The message rows come first and the meta row is pinned at the bottom. | `commit_footer_rows_match_on_every_registry_surface`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `shown` | `msg-scroll` | `hint-popover` | `expand` | `hash-link` | `rule-drag` | `meta` | `msg-colour` | `row-order` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `selection_footer_parts`, `commit_msg_scroll`, `paint_selection_footer`, `paint_footer_message`, `scroll_graph_footer_msg`, `on_footer_box`, `step_commit_msg_scroll`, `graph_more_lines_section`, `more_lines_section`, `record_commit_msg_rule` | supported | supported | supported | supported | supported | supported | supported | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `selection_footer_parts`, `commit_msg_scroll`, `paint_selection_footer`, `paint_footer_message`, `scroll_graph_footer_msg`, `on_footer_box`, `step_commit_msg_scroll`, `graph_more_lines_section`, `more_lines_section`, `record_commit_msg_rule` | supported | supported | supported | supported | supported | supported | supported | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_detail`, `draw_commit_footer`, `commit_detail_graph_row`, `commit_detail_fallback_name`, `commit_footer_parts`, `commit_detail_footer_lines`, `paint_selection_footer`, `paint_footer_message`, `scroll_commit_detail_msg`, `on_footer_box`, `step_commit_msg_scroll`, `commit_detail_msg_identity`, `commit_detail_more_lines_section`, `more_lines_section`, `record_commit_detail_msg_rule` | supported | supported | supported | supported | supported | supported | supported | supported | supported |
| `compare-commit` | Compare (Diff commit vs parent, Blame: open commit changes, Blame: open previous line change) | - | left | boxed, flat | `draw_panes`, `compare_commit_vs_parent`, `blame_commit_vs_parent`, `apply_blame_previous`, `commit_vs_parent`, `commit_vs_parent_head`, `compute_compare_range`, `load_commit`, `apply_compare_range`, `is_range_compare_tab`, `commit_footer_model`, `remote_target_for_chip`, `footer_chip_row`, `draw_commit_detail`, `draw_commit_footer`, `commit_footer_source`, `commit_detail_graph_row`, `commit_detail_fallback_name`, `commit_footer_parts`, `paint_selection_footer`, `paint_footer_message`, `scroll_commit_detail_msg`, `on_footer_box`, `step_commit_msg_scroll`, `commit_detail_msg_identity`, `commit_detail_more_lines_section`, `more_lines_section`, `record_commit_detail_msg_rule` | supported | supported | supported | supported | supported | supported | supported | supported | supported |
| `compare-range` | Compare (Diff vs default, branch, or commit; commit vs working tree) | - | left | boxed, flat | `draw_panes`, `draw_commit_file_list`, `is_range_compare_tab` | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message |

### `file-lists` — File lists

Lists of file rows: the workspace tree, the commit-file list (depth 1 right, depth 2 left), the compare-tab list, the folder summary (right pane, beside a focused commit-file folder), and the Explorer tree.

| Feature | What | Tests |
| --- | --- | --- |
| `row-icons` | Rows paint devicons and status marks, and record icon hits for hover and pin popovers. | `commit_file_rows_record_icon_hits`, `file_row_badge_and_devicon_hits_cover_the_glyph_only`, `folder_summary_rows_paint_the_file_list_marks_flash_and_icon_hits`, `tree_rows_paint_the_same_icons_marks_flash_and_popovers_on_every_registry_tree_surface` |
| `scroll` | Rows past the pane height scroll into view. | `folder_summary_scrolls_with_the_keys_and_wheel_and_resets_on_another_folder`, `list_vertical_bar_drags_and_track_click_jumps_on_every_registry_surface` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `row-icons` | `scroll` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `tree_row_paint_segments` | supported | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `draw_right`, `draw_commit_file_list`, `commit_file_row_paint_segments` | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_detail`, `draw_commit_file_list`, `commit_file_row_paint_segments` | supported | supported |
| `compare-files` | Compare | - | left | boxed, flat | `draw_panes`, `draw_commit_file_list`, `commit_file_row_paint_segments` | supported | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary`, `folder_summary`, `summary_rows`, `summary_row_paint_segments_in`, `scroll_summary_by` | supported | supported |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `explorer_row_segments`, `explorer_row_paint_segments` | supported | supported |

### `line-viewers` — Line viewers

The file diff (`draw_diff_pane`) and the clean-file view (`draw_file_body`, an Explorer inner file tab of a clean file).

| Feature | What | Tests |
| --- | --- | --- |
| `click-row` | A click on a line row moves the line cursor there; a press-drag from it selects text. | `line_click_picks_the_row_and_a_drag_selects_on_every_registry_surface` |
| `cursor-middle` | `j` / `k` keep the cursor near the vertical middle. | none |
| `section-label` | The section label names the diff's side (`STAGED`, `UNSTAGED`, `COMMITTED`, …). | `depth_2_commit_and_stash_diffs_paint_committed_in_boxed_and_flat_panes`, `commit_and_stash_diffs_label_committed_and_keep_partial_patch_kinds` |
| `full-context` | `Ctrl-o` toggles the whole-file context. | none |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `click-row` | `cursor-middle` | `section-label` | `full-context` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `diff-d0` | Workspace | 0 (file focused) | right | boxed, flat | `draw_right`, `draw_diff_pane`, `click_diff` | supported | supported | supported | supported |
| `diff-d2` | Workspace | 2 | right | boxed, flat | `draw_right`, `draw_diff_pane`, `click_diff`, `compute_commit_diff`, `from_commit_lines` | supported | supported | supported | supported |
| `compare-diff` | Compare | - | right | boxed, flat | `draw_right`, `draw_diff_pane`, `click_diff` | supported | supported | supported | supported |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `draw_explorer_tab`, `draw_diff_pane`, `explorer_click`, `explorer_click_row`, `click_diff` | supported | supported | supported | n/a: I4, Explorer diffs always show the whole file and `Ctrl-o` refuses |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `draw_explorer_tab`, `draw_file_body`, `click_file_tab` | supported | n/a: I3, the clean-file view scrolls at the edge | n/a: a clean file has no diff sections | n/a: I4, the Explorer refuses `Ctrl-o` and a clean file shows the whole file |

### `scrollbars` — Scrollbars

Vertical and horizontal scrollbars of lists and line viewers. The commit-message footer scrollbars are in `commit-footer`.

| Feature | What | Tests |
| --- | --- | --- |
| `vbar` | A vertical scrollbar paints in the last column whenever the rows overflow, at the top too. A list's rows stop left of it, and its pan clamp counts that column. A file diff and the clean-file view keep that column whether or not the bar shows. | `list_vertical_bar_drags_and_track_click_jumps_on_every_registry_surface`, `explorer_clean_file_bars_sit_where_the_changed_file_diff_bars_sit`, `the_detail_scrolls_with_the_wheel_clamps_and_starts_another_pr_at_the_top` |
| `vbar-drag` | A drag on the vertical thumb scrolls; a track click jumps. A press on the bar picks no row. A viewport that follows its cursor (the file diff, the Workspace tree, a commit-file list, the Explorer tree, the PRs list) moves the cursor with it and asks for what the wheel asks for (the right pane, a diff, a preview; nothing for the PRs list, whose detail follows its cursor). The clean-file view scrolls at the edge, so its line cursor moves to the nearest edge row of the new viewport, as the wheel leaves it. The folder summary and the PRs detail have no cursor, so only their viewport moves. | `graph_scrollbar_thumb_drag_updates_scroll_and_release_ends_drag`, `graph_scrollbar_track_click_jumps_and_arms_drag`, `diff_vertical_bar_drags_and_track_click_jumps_on_every_registry_surface`, `hit_test_diff_vertical_scrollbar_thumb_and_track_in_both_looks`, `cursor_for_viewport_top_on_wrapped_rows`, `list_vertical_bar_drags_and_track_click_jumps_on_every_registry_surface`, `hit_test_list_scrollbars_win_over_the_pane_band_and_share_the_corner`, `clean_file_vertical_bar_drags_wrapped_lines_in_both_looks` |
| `hbar` | A horizontal scrollbar paints once the pane pans off the left edge. A list keeps its last row for it, left of the vertical bar's corner cell. | `list_horizontal_bar_drags_and_track_click_jumps_on_every_registry_surface`, `explorer_clean_file_bars_sit_where_the_changed_file_diff_bars_sit` |
| `hbar-drag` | A drag on the horizontal thumb pans the offset Shift-←/→ pans, up to the same pan clamp; a track click jumps. A press on the bar picks no row. | `diff_hscrollbar_on_the_last_pane_row_drags_only_when_flat`, `diff_horizontal_bar_drags_and_track_click_jumps_on_every_registry_surface`, `list_horizontal_bar_drags_and_track_click_jumps_on_every_registry_surface`, `hit_test_list_scrollbars_win_over_the_pane_band_and_share_the_corner` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `vbar` | `vbar-drag` | `hbar` | `hbar-drag` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `record_graph_scrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `hit_hscrollbar`, `press_hscrollbar` | supported | supported | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `record_graph_scrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `hit_hscrollbar`, `press_hscrollbar` | supported | supported | supported | supported |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `paint_vscrollbar`, `paint_hscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_tree_to`, `cursor_for_viewport_top`, `hit_hscrollbar`, `press_hscrollbar`, `tree_max_col_in` | supported | supported | supported | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `draw_commit_file_list`, `paint_vscrollbar`, `paint_hscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_commit_files_to`, `cursor_for_viewport_top`, `hit_hscrollbar`, `press_hscrollbar`, `commit_file_max_col_in` | supported | supported | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_detail`, `draw_commit_file_list`, `paint_vscrollbar`, `paint_hscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_commit_files_to`, `cursor_for_viewport_top`, `hit_hscrollbar`, `press_hscrollbar`, `commit_file_max_col_in` | supported | supported | supported | supported |
| `compare-files` | Compare | - | left | boxed, flat | `draw_commit_file_list`, `paint_vscrollbar`, `paint_hscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_commit_files_to`, `cursor_for_viewport_top`, `hit_hscrollbar`, `press_hscrollbar`, `commit_file_max_col_in` | supported | supported | supported | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary`, `paint_vscrollbar`, `paint_hscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_summary_to`, `hit_hscrollbar`, `press_hscrollbar`, `summary_max_col_in` | supported | supported | supported | supported |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `paint_vscrollbar`, `paint_hscrollbar`, `explorer_click`, `press_split_hit`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_explorer_tree_to`, `cursor_for_viewport_top`, `hit_hscrollbar`, `press_hscrollbar`, `explorer_tree_max_col_in` | supported | supported | supported | supported |
| `diff-d0` | Workspace | 0 (file focused) | right | boxed, flat | `draw_diff_pane`, `paint_vscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_diff_to`, `cursor_for_viewport_top`, `paint_hscrollbar`, `hit_hscrollbar`, `press_hscrollbar` | supported | supported | supported | supported |
| `diff-d2` | Workspace | 2 | right | boxed, flat | `draw_diff_pane`, `paint_vscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_diff_to`, `cursor_for_viewport_top`, `paint_hscrollbar`, `hit_hscrollbar`, `press_hscrollbar` | supported | supported | supported | supported |
| `compare-diff` | Compare | - | right | boxed, flat | `draw_diff_pane`, `paint_vscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_diff_to`, `cursor_for_viewport_top`, `paint_hscrollbar`, `hit_hscrollbar`, `press_hscrollbar` | supported | supported | supported | supported |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `draw_diff_pane`, `explorer_click`, `press_split_hit`, `paint_vscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_diff_to`, `cursor_for_viewport_top`, `paint_hscrollbar`, `hit_hscrollbar`, `press_hscrollbar` | supported | supported | supported | supported |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `draw_file_body`, `explorer_click`, `paint_vscrollbar`, `paint_hscrollbar`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_file_view_to`, `cursor_for_viewport_top`, `file_view_scroll`, `hit_hscrollbar`, `press_hscrollbar`, `pan_max`, `pan_file_tab` | supported | supported | supported | supported |
| `prs-list` | PRs | - | left | boxed, flat | `draw_prs_list`, `paint_vscrollbar`, `prs_click`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_prs_list_to`, `cursor_for_viewport_top`, `prs_wheel` | supported | supported | n/a: PRs tab rows cut the title to fit and never pan | n/a: PRs tab rows cut the title to fit and never pan |
| `prs-detail` | PRs | - | right | boxed, flat | `draw_prs_detail`, `keep_prs_detail_scroll`, `paint_vscrollbar`, `prs_click`, `hit_vscrollbar`, `press_vscrollbar`, `scroll_prs_detail_to`, `prs_wheel`, `dispatch_prs_tab` | supported | supported | n/a: long values end in an ellipsis and never pan | n/a: long values end in an ellipsis and never pan |

### `search` — Pane search

`/` binds the focused pane's search: matching rows paint highlighted, and `n` / `N` step through the matches with the `/q 3/7` count.

| Feature | What | Tests |
| --- | --- | --- |
| `highlight` | Matching rows paint the search highlight. | none |
| `step` | `n` / `N` step to the next / previous match and the pill counts them. | `commit_file_search_follows_painted_tree_order` |
| `agree` | The highlighted rows are exactly the rows `n` / `N` step to and count. | `commit_file_search_highlight_and_steps_agree_on_every_registry_surface`, `commit_file_search_steps_to_folder_rows`, `commit_file_search_lands_on_the_folder_row_when_a_file_shares_its_path` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `highlight` | `step` | `agree` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `collect_match_ids`, `apply_tree_search` | supported | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `graph_search_matches`, `collect_graph_match_indices`, `focus_graph_search` | supported | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `graph_search_matches`, `collect_graph_match_indices`, `focus_graph_search` | supported | supported | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `commit_file_search_match_ids`, `commit_file_search_hits`, `commit_file_row_match_indices` | supported | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `commit_file_search_match_ids`, `commit_file_search_hits`, `commit_file_row_match_indices` | supported | supported | supported |
| `compare-files` | Compare | - | left | boxed, flat | `commit_file_search_match_ids`, `commit_file_search_hits`, `commit_file_row_match_indices` | supported | supported | supported |
| `diff-d0` | Workspace | 0 (file focused) | right | boxed, flat | `draw_diff_pane`, `diff_search_hits` | supported | supported | supported |
| `diff-d2` | Workspace | 2 | right | boxed, flat | `draw_diff_pane`, `diff_search_hits` | supported | supported | supported |
| `compare-diff` | Compare | - | right | boxed, flat | `draw_diff_pane`, `diff_search_hits` | supported | supported | supported |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `explorer_row_match_indices` | supported | supported | supported |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `draw_diff_pane`, `diff_search_hits` | supported | supported | supported |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `draw_file_body`, `file_search_hits` | supported | supported | supported |
| `prs-list` | PRs | - | left | boxed, flat | `prs_tab_refusal`, `draw_prs_list` | gap: G24 | gap: G24 | gap: G24 |

### `icon-popovers` — Icon popovers

Icons and marks on rows record an icon hit (`LayoutHit::icon_hits`): hover peeks and a click pins the icon's popover. `gh` pins the focused row's popover. The comment mark on a line gutter (a file diff, the Explorer clean-file view) is an icon too (`IconTarget::LineComment`): its popover lists the comments of that line, and on a focused line viewer the cursor line is the focused row.

| Feature | What | Tests |
| --- | --- | --- |
| `icon-hits` | The painted icons hover and pin their popover with the mouse. | `commit_file_rows_record_icon_hits`, `click_on_an_icon_selects_its_row_and_pins_without_a_double_click`, `line_gutter_comment_marks_peek_pin_and_gh_on_every_registry_surface`, `folder_summary_rows_paint_the_file_list_marks_flash_and_icon_hits`, `tree_rows_paint_the_same_icons_marks_flash_and_popovers_on_every_registry_tree_surface`, `the_mouse_selects_rows_folds_headers_and_opens_the_pr` |
| `focused-row` | `gh` pins the focused row's popover. | `gh_pins_the_icon_popover_and_bare_h_still_folds`, `line_gutter_comment_marks_peek_pin_and_gh_on_every_registry_surface`, `line_comment_popover_closes_when_the_cursor_leaves_its_line`, `line_comment_popover_frames_build_the_diff_rows_once`, `tree_rows_paint_the_same_icons_marks_flash_and_popovers_on_every_registry_tree_surface`, `gh_pins_the_popover_of_the_focused_pr_row` |
| `folder-count` | A folder glyph's section counts the changed files under the folder in its list. | `tree_rows_paint_the_same_icons_marks_flash_and_popovers_on_every_registry_tree_surface`, `explorer_folder_popover_counts_distinct_changed_paths` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `icon-hits` | `focused-row` | `folder-count` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `tree_row_paint_segments`, `focused_row_icons`, `tree_file_facts` | supported | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `graph_row_icons`, `focused_row_icons` | supported | supported | n/a: a graph row is a commit, with no folder glyph |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `graph_row_icons`, `focused_row_icons` | supported | supported | n/a: a graph row is a commit, with no folder glyph |
| `files-d1` | Workspace | 1 | right | boxed, flat | `draw_commit_file_list`, `commit_file_row_paint_segments`, `focused_row_icons`, `commit_file_facts` | supported | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_file_list`, `commit_file_row_paint_segments`, `focused_row_icons`, `commit_file_facts` | supported | supported | supported |
| `compare-files` | Compare | - | left | boxed, flat | `draw_commit_file_list`, `commit_file_row_paint_segments`, `focused_row_icons`, `commit_file_facts` | supported | supported | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary`, `summary_row_paint_segments_in`, `summary_rows` | supported | n/a: the summary is never the focused list, so it has no focused row | n/a: the summary lists file rows only, with no folder glyph |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `explorer_row_paint_segments`, `explorer_row_marks`, `segmented_icon_spans`, `explorer_row_section`, `explorer_click_row`, `click_with`, `focused_row_icons`, `explorer_cursor_row_icons`, `popover_owner`, `changed_files_under` | supported | supported | n/a: I7, the Explorer has no Staged or Changes sections, so a folder counts its distinct changed paths |
| `diff-gutter` | Workspace, Compare, Explorer | 0, 2, - | right | boxed, flat | `draw_diff_pane`, `comment_mark_cols`, `diff_row_mark_hits`, `diff_row_gutter_cells`, `line_comment_mark`, `cursor_line_comment_icons`, `diff_row_comment_marks`, `prime_cursor_comment_lines`, `line_comments`, `line_comment_section`, `focused_row_icons`, `click_with`, `split_hit_at` | supported | supported | n/a: a line gutter has no folder glyph |
| `file-gutter` | Explorer (clean file) | - | right | boxed, flat | `draw_file_body`, `format_line_gutter`, `line_mark_hit`, `line_comment_mark`, `cursor_line_comment_icons`, `file_cursor_comment_marks`, `line_comments`, `line_comment_section`, `focused_row_icons`, `click_with` | supported | supported | n/a: a line gutter has no folder glyph |
| `prs-tab` | PRs | - | left | boxed, flat | `draw_prs_list`, `prs_pin_focused` | supported | supported | n/a: a PRs tab row is one PR, with no folder glyph |

### `resize-edges` — Mouse on resize edges

A press on a resize edge drags it: the tree / right pane divider and the in-diff rule in split mode. The hovered edge lights. The commit-footer rule drag is in `commit-footer`.

| Feature | What | Tests |
| --- | --- | --- |
| `divider` | A drag on the pane divider resizes the panes. | `flat_divider_pad_press_and_drag_resizes`, `resize_edges_drag_and_hover_on_every_registry_surface`, `explorer_flat_divider_pad_press_released_in_place_clicks_the_tree_row` |
| `diff-rule` | A drag on the split-mode diff rule moves it. | `diff_split_rule_round_trips_through_the_fraction_boxed_and_flat`, `resize_edges_drag_and_hover_on_every_registry_surface` |
| `hover` | The edge under the pointer lights. | `hovered_pane_divider_lights_the_left_pane_edge_boxed_and_flat`, `resize_edges_drag_and_hover_on_every_registry_surface` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `divider` | `diff-rule` | `hover` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `workspace` | Workspace | any | both | boxed, flat | `click_at`, `press_split_hit`, `hit_split`, `resize_edges_take_mouse`, `hovered_resize_edge` | supported | supported | supported |
| `compare` | Compare | - | both | boxed, flat | `click_at`, `press_split_hit`, `hit_split`, `resize_edges_take_mouse`, `hovered_resize_edge` | supported | supported | supported |
| `explorer` | Explorer | - | both | boxed, flat | `explorer_click`, `press_split_hit`, `hit_split`, `resize_edges_take_mouse`, `hovered_resize_edge`, `draw_explorer_tab` | supported | supported | supported |
| `prs-tab` | PRs | - | both | boxed, flat | `click_at`, `resize_edges_take_mouse`, `draw_prs_tab` | n/a: the detail pane takes a fixed third of the width | n/a: the PRs tab paints no diff | n/a: the PRs tab has no edge to drag |

### `horizontal-pan` — Horizontal pan

Long labels and lines pan sideways: Shift-←/→ and `h` / `l` on the focused pane, and the horizontal wheel (or Shift-wheel) on the pane under the pointer.

| Feature | What | Tests |
| --- | --- | --- |
| `shift-keys` | Shift-←/→ pan the focused pane. | `list_pan_shifts_tree_and_graph_without_stealing_fold`, `folder_summary_pans_its_own_rows_not_the_hidden_diff`, `list_shift_keys_and_hwheel_pan_on_every_registry_list_surface` |
| `h-l` | `h` / `l` pan the focused row or line. | `h_l_fold_commit_file_dirs_and_pan_on_files`, `focused_diff_l_increases_pan_h_decreases_tree_still_folds`, `folder_summary_pans_its_own_rows_not_the_hidden_diff` |
| `hwheel` | The horizontal wheel pans the pane under the pointer; over a tree (the Workspace tree, the Explorer tree), a shown file view that can pan takes the pan, and the tree pans only when it cannot. | `mouse_hscroll_and_shift_wheel_map_to_horizontal_pan`, `folder_summary_pans_its_own_rows_not_the_hidden_diff`, `list_shift_keys_and_hwheel_pan_on_every_registry_list_surface`, `tree_hwheel_pans_a_shown_file_view_that_can_pan_on_every_registry_tree_surface` |
| `pan-limit` | The pan stops where the pane's own longest row ends. | `commit_file_and_diff_search_and_pan_clamp`, `compare_file_pan_clamps_to_the_compare_list_not_the_parked_drill`, `folder_summary_pans_its_own_rows_not_the_hidden_diff`, `list_horizontal_bar_drags_and_track_click_jumps_on_every_registry_surface`, `list_shift_keys_and_hwheel_pan_on_every_registry_list_surface` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `shift-keys` | `h-l` | `hwheel` | `pan-limit` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `pan_left_pane`, `tree_max_col`, `mouse_pan` | supported | n/a: I1, `h` / `l` fold on a tree row | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `pan_right_pane`, `graph_max_col`, `mouse_pan` | supported | supported | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `pan_left_pane`, `graph_max_col`, `mouse_pan` | supported | supported | supported | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `pan_right_pane`, `commit_file_max_col`, `mouse_pan` | supported | supported | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `pan_left_pane`, `commit_file_max_col`, `mouse_pan` | supported | supported | supported | supported |
| `compare-files` | Compare | - | left | boxed, flat | `pan_left_pane`, `commit_file_max_col`, `commit_file_comment_scope`, `mouse_pan` | supported | supported | supported | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary`, `pan_right_pane`, `pan_summary`, `summary_max_col_in`, `mouse_pan` | supported | supported | supported | supported |
| `explorer-tree` | Explorer | - | left | boxed, flat | `dispatch_explorer_tab`, `pan_explorer_tree`, `explorer_tree_max_col_in`, `explorer_wheel`, `explorer_preview_pan_max`, `draw_explorer_tree` | supported | n/a: as on the Workspace tree (I1), `h` / `l` move through folders | supported | supported |
| `diff-d0` | Workspace | 0 (file focused) | right | boxed, flat | `pan_diff_content`, `diff_pan_max` | supported | supported | supported | supported |
| `diff-d2` | Workspace | 2 | right | boxed, flat | `pan_diff_content`, `diff_pan_max` | supported | supported | supported | supported |
| `compare-diff` | Compare | - | right | boxed, flat | `pan_diff_content`, `diff_pan_max` | supported | supported | supported | supported |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `explorer_preview_pan`, `pan_diff_content` | supported | n/a: I2, `h` gives focus back to the tree and `l` does nothing | supported | supported |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `explorer_preview_pan`, `pan_file_tab` | supported | n/a: I2, `h` gives focus back to the tree and `l` does nothing | supported | supported |
| `prs-tab` | PRs | - | left, right | boxed, flat | `dispatch_prs_tab`, `row_segments` | n/a: PRs tab rows cut the title to fit and never pan | n/a: `h` and `l` do nothing on the PRs tab | n/a: PRs tab rows cut the title to fit and never pan | n/a: PRs tab rows cut the title to fit and never pan |

### `double-click` — Double-click

A second press on the same cell within 400 ms is Enter on that row.

| Feature | What | Tests |
| --- | --- | --- |
| `enter` | A double-click does what Enter does on the row. | `double_click_dispatches_enter`, `explorer_tree_double_click_is_enter_boxed_and_flat` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `enter` |
| --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `click_row`, `nav_enter` | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `click_row`, `click_graph` | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `click_row`, `click_graph` | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `click_row`, `click_commit_files` | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `click_row`, `click_commit_files` | supported |
| `compare-files` | Compare | - | left | boxed, flat | `click_row`, `click_commit_files` | supported |
| `diff-d0` | Workspace | 0 (file focused) | right | boxed, flat | `click_row`, `click_diff`, `nav_enter` | n/a: Enter has no action on a focused file diff (`nav_enter` warns `focus a repo commit to drill` at depth 0 and returns `Effect::None` at depth 2 and on a compare tab), so a double-click there has nothing to repeat |
| `diff-d2` | Workspace | 2 | right | boxed, flat | `click_row`, `click_diff`, `nav_enter` | n/a: Enter has no action on a focused file diff (`nav_enter` warns `focus a repo commit to drill` at depth 0 and returns `Effect::None` at depth 2 and on a compare tab), so a double-click there has nothing to repeat |
| `compare-diff` | Compare | - | right | boxed, flat | `click_row`, `click_diff`, `nav_enter` | n/a: Enter has no action on a focused file diff (`nav_enter` warns `focus a repo commit to drill` at depth 0 and returns `Effect::None` at depth 2 and on a compare tab), so a double-click there has nothing to repeat |
| `explorer-tree` | Explorer | - | left | boxed, flat | `explorer_click`, `explorer_click_row`, `note_row_press`, `explorer_enter` | supported |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `explorer_click`, `dispatch_explorer_tab` | n/a: Enter does nothing in the Explorer file pane |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `explorer_click`, `dispatch_explorer_tab` | n/a: Enter does nothing in the Explorer file pane |
| `prs-tab` | PRs | - | left | boxed, flat | `prs_click`, `dispatch_prs_tab` | n/a: Enter opens the PR in the browser, so a double-click only selects the row; on a group header the second press does not undo the fold |

### `row-marks` — Row marks

The reviewed eye and the comment mark (open or resolved) on a row or a line gutter.

| Feature | What | Tests |
| --- | --- | --- |
| `reviewed` | A reviewed file paints the eye. | `folder_summary_rows_paint_the_file_list_marks_flash_and_icon_hits`, `tree_rows_paint_the_same_icons_marks_flash_and_popovers_on_every_registry_tree_surface`, `explorer_space_paints_the_reviewed_eye_on_both_trees` |
| `comment` | A row or line with comments paints the comment mark. | `commit_file_row_paints_comment_mark_when_file_has_comments`, `graph_commit_row_marks_object_and_file_comments`, `explorer_file_view_paints_a_comment_mark_on_its_line`, `folder_summary_rows_paint_the_file_list_marks_flash_and_icon_hits`, `tree_rows_paint_the_same_icons_marks_flash_and_popovers_on_every_registry_tree_surface` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `reviewed` | `comment` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `tree_row_marks` | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `graph_row_comments` | n/a: a graph row is a commit, and reviewed marks are per file | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `graph_row_comments` | n/a: a graph row is a commit, and reviewed marks are per file | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `commit_file_row_paint_segments`, `commit_file_row_has_comment` | n/a: reviewed marks are for workspace-tree files and compare ranges; Space in a commit drill says so | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `commit_file_row_paint_segments`, `commit_file_row_has_comment` | n/a: reviewed marks are for workspace-tree files and compare ranges; Space in a commit drill says so | supported |
| `compare-files` | Compare | - | left | boxed, flat | `commit_file_row_paint_segments`, `compare_file_reviewed` | supported | supported |
| `summary-d2` | Workspace | 2 | right | boxed, flat | `draw_folder_summary`, `summary_row_paint_segments_in`, `commit_file_row_has_comment` | n/a: reviewed marks are for workspace-tree files and compare ranges, so the commit drill's file list has no eye to match | supported |
| `summary-compare` | Compare | - | right | boxed, flat | `draw_folder_summary`, `summary_row_paint_segments_in`, `compare_file_reviewed` | supported | supported |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `explorer_row_paint_segments`, `explorer_row_marks`, `checkout_file_comment_states`, `explorer_toggle_reviewed`, `toggle_viewed_for` | supported | supported |
| `diff-gutter` | Workspace, Compare, Explorer | 0, 2, - | right | boxed, flat | `draw_diff_pane`, `comment_mark_cols` | n/a: reviewed is per file, and the file's list row carries the eye | supported |
| `file-gutter` | Explorer (clean file) | - | right | boxed, flat | `draw_file_body`, `file_line_comment` | n/a: reviewed is per file, and the file's list row carries the eye | supported |

### `watch-flash` — Watch-reload flash

A row that a watch reload adds or changes flashes and fades. The rows that show a checkout's PR also flash when its PR changes (`pr-change`), closed included.

| Feature | What | Tests |
| --- | --- | --- |
| `flash` | A reloaded row flashes. | `flash_paints_node_and_spacer_cursor_wins`, `folder_summary_rows_paint_the_file_list_marks_flash_and_icon_hits`, `tree_rows_paint_the_same_icons_marks_flash_and_popovers_on_every_registry_tree_surface`, `explorer_rows_flash_on_a_watch_reload`, `explorer_folders_take_the_flash_of_the_workspace_row_that_shows_them` |
| `pr-change` | When a checkout's PR turns CI failed, ready to merge, merged, or closed, each row that shows its PR flashes (the checkout's tree row, never a family row; its graph worktree row; its PRs tab row while that tab is open), closed included though its badge is gone: the remove ramp for CI failed and closed, the add ramp for ready to merge and merged. The PR badge glyph keeps its state colour. | `a_pr_change_flashes_the_checkout_tree_row_on_the_remove_or_add_ramp`, `a_pr_change_flash_keeps_the_badge_glyph_in_its_state_colour`, `a_pr_change_flashes_the_graph_worktree_row_at_depth_0_and_1`, `a_pr_change_flashes_its_row_on_the_prs_tab`, `a_pr_change_flashes_its_prs_tab_row` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `flash` | `pr-change` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `flash_color`, `alert_pr_transitions` | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `graph_flash_rows`, `alert_pr_transitions` | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `graph_flash_rows`, `alert_pr_transitions` | supported | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `draw_commit_file_list`, `commit_file_flash_color` | supported | n/a: commit-file rows carry no PR |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_file_list`, `commit_file_flash_color` | supported | n/a: commit-file rows carry no PR |
| `compare-files` | Compare | - | left | boxed, flat | `draw_commit_file_list`, `commit_file_flash_color` | supported | n/a: compare file rows carry no PR |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary`, `commit_file_flash_color` | supported | n/a: folder summary rows carry no PR |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `explorer_row_marks`, `explorer_row_flash_color`, `explorer_folder_flashes`, `flash_color`, `apply_watch_snapshot` | supported | n/a: Explorer rows carry no PR |
| `prs-tab` | PRs | - | left | boxed, flat | `draw_prs_list`, `alert_pr_transitions`, `pr_row_ids` | n/a: PRs tab rows come from the PR cache, which no watch reload changes | supported |

### `pr-status` — PR status marks

The PR of a checkout's branch, in one of six states: the badge at the end of a workspace tree row (`<ref> <glyph>`), the badge flush right on a graph worktree row (glyph only), the chip flush right on the graph sync header (`<glyph> <ref> <state label>  ▸ <next action>`), and the PRs tab rows (`<glyph> <ref>` first, the state label in its own column; the next action on the group header), whose detail pane shows the PR popover lines (`popover::pr_detail_lines`). `tree::pr_badge_kind` and `tree::pr_badge_mark` give every surface the same glyph and colour role. Every mark opens the same PR popover (`popover::pr_section`), whose PR lines act on its own PR (`run_pr_action_at`). A checkout on its repository's known default branch has no PR status, so no surface paints a mark for it: every surface reads the same PR rows (`AppState::pr_rows`), which leave it out (`a_default_branch_checkout_shows_no_badge_total_section_or_prs_tab_row`).

| Feature | What | Tests |
| --- | --- | --- |
| `badge` | The state glyph paints bold in its state colour, one column, in both glyph modes. | `tree_pr_badge_ends_at_the_pane_edge_in_every_state_look_and_forge`, `graph_worktree_badges_paint_each_state_and_stale_muted`, `graph_header_chip_reads_state_ref_label_and_next_action`, `the_prs_tab_paints_groups_rows_and_the_detail_in_both_looks`, `ascii_mode_paints_letters` |
| `ref` | The PR ref (`#318` GitHub, `!12` GitLab) paints beside the glyph. | `tree_pr_badge_ends_at_the_pane_edge_in_every_state_look_and_forge`, `tree_pr_glyphs_share_one_column_and_refs_drop_on_a_narrow_tree`, `graph_header_chip_reads_state_ref_label_and_next_action`, `the_prs_tab_paints_groups_rows_and_the_detail_in_both_looks` |
| `stale` | A PR whose last successful lookup is old paints muted, shape unchanged. | `stale_tree_pr_badge_paints_muted`, `graph_worktree_badges_paint_each_state_and_stale_muted`, `graph_header_chip_reads_state_ref_label_and_next_action`, `empty_loading_error_and_stale_states_use_their_copy` |
| `link` | The painted PR mark is one icon hit (hover peek, click pin, Ctrl+click opens the PR) and one OSC 8 link to the PR. | `tree_pr_badge_ends_at_the_pane_edge_in_every_state_look_and_forge`, `ctrl_click_on_the_painted_tree_badge_opens_its_pr_only_there`, `graph_worktree_badge_paints_and_ctrl_click_opens_its_pr`, `graph_header_chip_paints_flush_right_and_links_its_ref_in_every_look`, `the_mouse_selects_rows_folds_headers_and_opens_the_pr`, `the_prs_tab_paints_groups_rows_and_the_detail_in_both_looks` |
| `chip` | The state label and the next action (merged: the age) follow the ref; a narrower pane drops the next action, then the label. | `graph_header_chip_reads_state_ref_label_and_next_action`, `graph_header_chip_paints_flush_right_and_links_its_ref_in_every_look`, `header_chip_paints_the_widest_candidate_that_fits_flush_right` |
| `popover` | The PR popover (peek or pin on the mark) shows `<glyph> PR · <state label>`, the PR lines, a rule, and Open PR, the CI page, Copy URL, and Refresh this PR (not on a merged PR); its PR lines act on that PR, whatever row has the focus. | `each_state_names_its_next_step_and_actions_on_both_forges`, `a_failed_github_pr_lists_every_line_in_order`, `a_ready_gitlab_mr_shows_its_pipeline_and_approvals`, `pr_popover_paints_its_heading_bold_ref_rule_and_clips_long_values`, `pr_popover_lines_act_on_their_own_pr_not_the_focused_graph_row`, `the_mouse_selects_rows_folds_headers_and_opens_the_pr`, `gh_pins_the_popover_of_the_focused_pr_row` |
| `open-ci` | `gc` on the row or in the pinned PR popover, and the popover CI line, open the PR's CI page (GitHub the failed, running, or first check, else the checks tab; GitLab the head pipeline, else the pipelines tab). | `gc_on_a_tree_row_opens_the_known_ci_link_on_both_forges`, `gc_in_the_graph_opens_a_worktree_rows_ci_on_both_forges`, `the_popover_open_ci_line_runs_gc_on_both_forges`, `gc_looks_the_pr_up_then_opens_its_ci_page_on_both_forges`, `pr_popover_lines_act_on_their_own_pr_not_the_focused_graph_row`, `gx_and_gc_in_a_pinned_pr_popover_act_on_its_pr_on_both_forges`, `keys_act_on_the_focused_pr_and_its_first_checkout` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `badge` | `ref` | `stale` | `link` | `chip` | `popover` | `open-ci` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `tree_pr_badge`, `with_pr_badge`, `segmented_icon_spans`, `pr_badge_links`, `pr_section`, `run_pr_action` | supported | supported | supported | supported | n/a: I6, the graph header chip carries the label and the next action | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `graph_pr_badges`, `row_badges`, `pr_badge_links`, `pr_section`, `run_pr_action` | supported | n/a: I6, the worktree row is narrow and the header chip carries the ref | supported | supported | n/a: I6, the header chip carries the label and the next action | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `graph_pr_badges`, `row_badges`, `pr_badge_links`, `pr_section`, `run_pr_action` | supported | n/a: I6, the worktree row is narrow and the header chip carries the ref | supported | supported | n/a: I6, the header chip carries the label and the next action | supported | supported |
| `graph-header` | Workspace | 0, 1 | right, left | boxed, flat | `draw_graph`, `graph_header_pr`, `graph_pr_chip`, `header_chip`, `pr_badge_links`, `pr_section`, `run_pr_action` | supported | supported | supported | supported | supported | supported | supported |
| `prs-tab` | PRs | - | left, right | boxed, flat | `draw_prs_tab`, `draw_prs_list`, `row_segments`, `header_segments`, `draw_prs_detail`, `pr_detail_lines`, `pr_badge_links`, `dispatch_prs_tab`, `run_pr_action_on_checkout` | supported | supported | supported | supported | n/a: I6, the rows paint the state label in a fixed column and the group headers the next action | supported | supported |

### `query-esc` — Esc clears the dialog query, then closes

The query dialogs that paint over the panes: the search-in-files dialog (Ctrl-f, or `#` in Quick Open) and Quick Open in files and commands mode. Esc with typed text clears it through the same edit path as Backspace; Esc on an empty query (a bare `>` in commands mode) closes the dialog; a closed search dialog parks for the next Ctrl-f. The footer Esc chip reads `Esc clear` or `Esc close` to match. The filter pickers keep one-press Esc (I5).

| Feature | What | Tests |
| --- | --- | --- |
| `two-step-esc` | Esc with a typed query clears it and keeps the dialog open; Esc on an empty query closes it. | `pty_search_in_files_esc_clears_query_then_closes`, `pty_quick_open_files_esc_clears_query_then_closes`, `pty_quick_open_commands_esc_keeps_prefix_then_closes`, `pty_quick_open_hash_search_esc_clears_query_then_closes`, `ctrl_f_toggles_and_esc_clears_then_parks`, `esc_from_the_results_zone_clears_the_query_and_its_hits`, `esc_clears_the_files_query_then_closes`, `esc_clears_commands_text_back_to_the_prefix_then_closes` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `two-step-esc` |
| --- | --- | --- | --- | --- | --- | --- |
| `search-files` | any | - | overlay | boxed, flat | `search_files_key_action`, `escape_search_files`, `search_files_edit`, `close_search_files`, `footer_hints` | supported |
| `quick-open-files` | any | - | overlay | boxed, flat | `quick_open_key_action`, `escape_quick_open`, `quick_open_edit`, `close_quick_open`, `cleared_query`, `draw_quick_open` | supported |
| `quick-open-commands` | any | - | overlay | boxed, flat | `quick_open_key_action`, `escape_quick_open`, `quick_open_edit`, `close_quick_open`, `cleared_query`, `sync_theme_preview`, `draw_quick_open` | supported |
| `quick-open-search` | any | - | overlay | boxed, flat | `quick_open_edit`, `open_search_files_with`, `search_files_key_action`, `escape_search_files`, `search_files_edit` | supported |
| `branch-picker` | any | - | overlay | boxed, flat | `key_to_action`, `dispatch_write`, `draw_branch_picker` | n/a: I5, one Esc closes a filter picker |
| `compare-picker` | any | - | overlay | boxed, flat | `key_to_action`, `dispatch_keymap`, `draw_compare_picker` | n/a: I5, one Esc closes a filter picker |
| `graph-focus-picker` | Workspace | - | overlay | boxed, flat | `key_to_action`, `dispatch_keymap`, `draw_graph_focus_picker` | n/a: I5, one Esc closes a filter picker |

## Open gaps

Each open gap is tracked by its id. Fixing one turns its cells into `supported` (or `n/a: <reason>`) and removes its row here.

| Gap | Component | Has it | Missing on |
| --- | --- | --- | --- |
| G24 | `search` | Workspace tree, graph, file lists, diffs, and the Explorer: `/` binds the focused pane, and `n` / `N` step | PRs tab list: `/`, `n`, and `N` say `PRs tab has no search` (`prs_tab_refusal`) |

## Intentional differences

| Id | Difference | Stated in |
| --- | --- | --- |
| I1 | Workspace tree file row: `h` / `l` fold; commit-file file row: `h` / `l` pan | `docs/tui-rust.md` (`h` / `l` key row) |
| I2 | Explorer file pane: `h` gives focus back to the tree, `l` does nothing | `docs/tui-model.md` (Explorer keys) |
| I3 | Clean-file view: `j` / `k` scroll at the edge, not the middle | `docs/tui-rust.md` (viewport rule), `docs/tui-model.md` (sticky file view top) |
| I4 | Explorer: `??` untracked, `●` dirty folder, no `t` toggle, `Ctrl-o` refused | `docs/tui-model.md` (Explorer tabs) |
| I5 | Filter pickers (branch, compare, graph focus): one Esc closes, typed filter or not. Quick Open and the search-in-files dialog take two presses with a typed query. | `docs/tui-rust.md` (picker key rule), `docs/tui-model.md` (Quick Open keys) |
| I6 | PR status: the tree badge is `<ref> <glyph>`, a graph worktree row paints the glyph alone, and of the badges only the graph header chip paints the state label and the next action; the PRs tab paints the state label in a row column and the next action in the group header | `docs/tui-rust.md` (Pull request badge, PRs tab) |
| I7 | Explorer folder popover counts distinct changed paths; the Workspace counts per Staged / Changes section, since the Explorer has no sections | `docs/tui-model.md` (Explorer tabs) |
