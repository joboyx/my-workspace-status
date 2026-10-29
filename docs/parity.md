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
   - `gap: G<n>`: an open gap (`n` from 1, no leading zero). A gap belongs to the component that **Open gaps** names; a cell in another component adds a note after `;` (`gap: G13; the tree never pans, so it has no bar`).
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
| `rule-drag` | A drag on the footer's top rule adds or removes message rows. | `commit_msg_rule_drag_resizes_the_footer_and_release_saves_once`, `commit_footer_rule_drag_matches_on_every_registry_surface` |
| `meta` | The meta row has coloured ref chips with popovers and `parent <id>`. | `commit_footer_rows_match_on_every_registry_surface`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph` |
| `msg-colour` | The message text paints in the subject colour (`pal.repo`). | `commit_footer_rows_match_on_every_registry_surface`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph` |
| `row-order` | The message rows come first and the meta row is pinned at the bottom. | `commit_footer_rows_match_on_every_registry_surface`, `blame_opened_compare_commit_footer_reads_the_tab_commit_off_the_graph` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `shown` | `msg-scroll` | `hint-popover` | `expand` | `hash-link` | `rule-drag` | `meta` | `msg-colour` | `row-order` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `selection_footer_parts`, `commit_msg_scroll`, `paint_selection_footer`, `paint_footer_message`, `scroll_graph_footer_msg`, `on_footer_box`, `step_commit_msg_scroll`, `graph_more_lines_section`, `more_lines_section`, `record_commit_msg_rule` | supported | supported | supported | supported | supported | supported | supported | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `selection_footer_parts`, `commit_msg_scroll`, `paint_selection_footer`, `paint_footer_message`, `scroll_graph_footer_msg`, `on_footer_box`, `step_commit_msg_scroll`, `graph_more_lines_section`, `more_lines_section`, `record_commit_msg_rule` | supported | supported | supported | supported | supported | supported | supported | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_detail`, `draw_commit_footer`, `commit_detail_graph_row`, `commit_detail_fallback_name`, `commit_footer_parts`, `commit_detail_footer_lines`, `paint_selection_footer`, `paint_footer_message`, `scroll_commit_detail_msg`, `on_footer_box`, `step_commit_msg_scroll`, `commit_detail_msg_identity`, `commit_detail_more_lines_section`, `more_lines_section`, `record_commit_detail_msg_rule` | supported | supported | supported | supported | supported | supported | supported | supported | supported |
| `compare-commit` | Compare (Diff commit vs parent, Blame: open commit changes, Blame: open previous line change) | - | left | boxed, flat | `draw_panes`, `compare_commit_vs_parent`, `blame_commit_vs_parent`, `apply_blame_previous`, `commit_vs_parent`, `commit_vs_parent_head`, `compute_compare_range`, `load_commit`, `apply_compare_range`, `is_range_compare_tab`, `commit_footer_model`, `draw_commit_detail`, `draw_commit_footer`, `commit_footer_source`, `commit_detail_graph_row`, `commit_detail_fallback_name`, `commit_footer_parts`, `paint_selection_footer`, `paint_footer_message`, `scroll_commit_detail_msg`, `on_footer_box`, `step_commit_msg_scroll`, `commit_detail_msg_identity`, `commit_detail_more_lines_section`, `more_lines_section`, `record_commit_detail_msg_rule` | supported | supported | supported | supported | supported | supported | supported | supported | supported |
| `compare-range` | Compare (Diff vs default, branch, or commit; commit vs working tree) | - | left | boxed, flat | `draw_panes`, `draw_commit_file_list`, `is_range_compare_tab` | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message | n/a: a range compare has no single commit message |

### `file-lists` — File lists

Lists of file rows: the workspace tree, the commit-file list (depth 1 right, depth 2 left), the compare-tab list, the folder summary (right pane, beside a focused commit-file folder), and the Explorer tree.

| Feature | What | Tests |
| --- | --- | --- |
| `row-icons` | Rows paint devicons and status marks, and record icon hits for hover and pin popovers. | `commit_file_rows_record_icon_hits`, `file_row_badge_and_devicon_hits_cover_the_glyph_only` |
| `scroll` | Rows past the pane height scroll into view. | none |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `row-icons` | `scroll` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `tree_row_paint_segments` | supported | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `draw_right`, `draw_commit_file_list`, `commit_file_row_paint_segments` | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_detail`, `draw_commit_file_list`, `commit_file_row_paint_segments` | supported | supported |
| `compare-files` | Compare | - | left | boxed, flat | `draw_panes`, `draw_commit_file_list`, `commit_file_row_paint_segments` | supported | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary`, `folder_summary` | gap: G9 | gap: G9 |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `explorer_row_segments` | gap: G15 | supported |

### `line-viewers` — Line viewers

The file diff (`draw_diff_pane`) and the clean-file view (`draw_file_body`, an Explorer inner file tab of a clean file).

| Feature | What | Tests |
| --- | --- | --- |
| `click-row` | A click on a line row moves the line cursor there. | none |
| `cursor-middle` | `j` / `k` keep the cursor near the vertical middle. | none |
| `section-label` | The section label names the diff's side (`STAGED`, `UNSTAGED`, `COMMITTED`, …). | `depth_2_commit_and_stash_diffs_paint_committed_in_boxed_and_flat_panes`, `commit_and_stash_diffs_label_committed_and_keep_partial_patch_kinds` |
| `full-context` | `Ctrl-o` toggles the whole-file context. | none |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `click-row` | `cursor-middle` | `section-label` | `full-context` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `diff-d0` | Workspace | 0 (file focused) | right | boxed, flat | `draw_right`, `draw_diff_pane`, `click_diff` | supported | supported | supported | supported |
| `diff-d2` | Workspace | 2 | right | boxed, flat | `draw_right`, `draw_diff_pane`, `click_diff`, `compute_commit_diff`, `from_commit_lines` | supported | supported | supported | supported |
| `compare-diff` | Compare | - | right | boxed, flat | `draw_right`, `draw_diff_pane`, `click_diff` | supported | supported | supported | supported |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `draw_explorer_tab`, `draw_diff_pane`, `explorer_click` | gap: G12 | supported | supported | n/a: I4, Explorer diffs always show the whole file and `Ctrl-o` refuses |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `draw_explorer_tab`, `draw_file_body`, `click_file_tab` | supported | n/a: I3, the clean-file view scrolls at the edge | n/a: a clean file has no diff sections | n/a: I4, the Explorer refuses `Ctrl-o` and a clean file shows the whole file |

### `scrollbars` — Scrollbars

Vertical and horizontal scrollbars of lists and line viewers. The commit-message footer scrollbars are in `commit-footer`.

| Feature | What | Tests |
| --- | --- | --- |
| `vbar` | A vertical scrollbar paints when the rows overflow. | none |
| `vbar-drag` | A drag on the vertical thumb scrolls; a track click jumps. | `graph_scrollbar_thumb_drag_updates_scroll_and_release_ends_drag`, `graph_scrollbar_track_click_jumps_and_arms_drag` |
| `hbar` | A horizontal scrollbar paints when the columns overflow. | none |
| `hbar-drag` | A drag on the horizontal thumb pans; a track click jumps. | `diff_hscrollbar_on_the_last_pane_row_drags_only_when_flat` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `vbar` | `vbar-drag` | `hbar` | `hbar-drag` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `record_graph_scrollbar`, `hit_graph_scrollbar` | supported | supported | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `record_graph_scrollbar`, `hit_graph_scrollbar` | supported | supported | supported | supported |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree` | gap: G5 | gap: G5 | gap: G23 | gap: G23 |
| `files-d1` | Workspace | 1 | right | boxed, flat | `draw_commit_file_list` | gap: G5 | gap: G5 | gap: G23 | gap: G23 |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_detail`, `draw_commit_file_list` | gap: G5 | gap: G5 | gap: G23 | gap: G23 |
| `compare-files` | Compare | - | left | boxed, flat | `draw_commit_file_list` | gap: G5 | gap: G5 | gap: G23 | gap: G23 |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree` | gap: G5 | gap: G5 | gap: G13; the tree never pans, so it has no bar | gap: G13; the tree never pans, so it has no bar |
| `diff-d0` | Workspace | 0 (file focused) | right | boxed, flat | `draw_diff_pane`, `hit_diff_hscrollbar` | supported | gap: G4 | supported | supported |
| `diff-d2` | Workspace | 2 | right | boxed, flat | `draw_diff_pane`, `hit_diff_hscrollbar` | supported | gap: G4 | supported | supported |
| `compare-diff` | Compare | - | right | boxed, flat | `draw_diff_pane`, `hit_diff_hscrollbar` | supported | gap: G4 | supported | supported |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `draw_diff_pane`, `explorer_click` | supported | gap: G4 | supported | gap: G11; `explorer_click` never calls `hit_split` |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `draw_file_body` | gap: G6 | gap: G6 | gap: G6 | gap: G6 |

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

### `icon-popovers` — Icon popovers

Icons and marks on rows record an icon hit (`LayoutHit::icon_hits`): hover peeks and a click pins the icon's popover. `gh` pins the focused row's popover.

| Feature | What | Tests |
| --- | --- | --- |
| `icon-hits` | The painted icons hover and pin their popover with the mouse. | `commit_file_rows_record_icon_hits`, `click_on_an_icon_selects_its_row_and_pins_without_a_double_click` |
| `focused-row` | `gh` pins the focused row's popover. | `gh_pins_the_icon_popover_and_bare_h_still_folds` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `icon-hits` | `focused-row` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `tree_row_paint_segments`, `focused_row_icons` | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `graph_row_icons`, `focused_row_icons` | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `graph_row_icons`, `focused_row_icons` | supported | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `draw_commit_file_list`, `commit_file_row_paint_segments`, `focused_row_icons` | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_file_list`, `commit_file_row_paint_segments`, `focused_row_icons` | supported | supported |
| `compare-files` | Compare | - | left | boxed, flat | `draw_commit_file_list`, `commit_file_row_paint_segments`, `focused_row_icons` | supported | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary` | gap: G9; `draw_folder_summary` records no icon hit | n/a: the summary is never the focused list, so it has no focused row |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `explorer_row_segments`, `focused_row_icons` | gap: G15; Explorer rows record no icon hit | gap: G15; `focused_row_icons` is empty on an Explorer tab |
| `diff-gutter` | Workspace, Compare, Explorer | 0, 2, - | right | boxed, flat | `draw_diff_pane`, `comment_mark_cols` | gap: G7 | gap: G7 |
| `file-gutter` | Explorer (clean file) | - | right | boxed, flat | `draw_file_body`, `format_line_gutter` | gap: G7 | gap: G7 |

### `resize-edges` — Mouse on resize edges

A press on a resize edge drags it: the tree / right pane divider and the in-diff rule in split mode. The hovered edge lights. The commit-footer rule drag is in `commit-footer`.

| Feature | What | Tests |
| --- | --- | --- |
| `divider` | A drag on the pane divider resizes the panes. | `flat_divider_pad_press_and_drag_resizes` |
| `diff-rule` | A drag on the split-mode diff rule moves it. | `diff_split_rule_round_trips_through_the_fraction_boxed_and_flat` |
| `hover` | The edge under the pointer lights. | `hovered_pane_divider_lights_the_left_pane_edge_boxed_and_flat` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `divider` | `diff-rule` | `hover` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `workspace` | Workspace | any | both | boxed, flat | `click_at`, `hit_split`, `resize_edges_take_mouse`, `hovered_resize_edge` | supported | supported | supported |
| `compare` | Compare | - | both | boxed, flat | `click_at`, `hit_split`, `resize_edges_take_mouse`, `hovered_resize_edge` | supported | supported | supported |
| `explorer` | Explorer | - | both | boxed, flat | `explorer_click`, `resize_edges_take_mouse`, `draw_explorer_tab` | gap: G11 | gap: G11 | gap: G11 |

### `horizontal-pan` — Horizontal pan

Long labels and lines pan sideways: Shift-←/→ and `h` / `l` on the focused pane, and the horizontal wheel (or Shift-wheel) on the pane under the pointer.

| Feature | What | Tests |
| --- | --- | --- |
| `shift-keys` | Shift-←/→ pan the focused pane. | `list_pan_shifts_tree_and_graph_without_stealing_fold` |
| `h-l` | `h` / `l` pan the focused row or line. | `h_l_fold_commit_file_dirs_and_pan_on_files`, `focused_diff_l_increases_pan_h_decreases_tree_still_folds` |
| `hwheel` | The horizontal wheel pans the pane under the pointer. | `mouse_hscroll_and_shift_wheel_map_to_horizontal_pan` |
| `pan-limit` | The pan stops where the pane's own longest row ends. | `commit_file_and_diff_search_and_pan_clamp`, `compare_file_pan_clamps_to_the_compare_list_not_the_parked_drill` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `shift-keys` | `h-l` | `hwheel` | `pan-limit` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `pan_left_pane`, `tree_max_col`, `mouse_pan` | supported | n/a: I1, `h` / `l` fold on a tree row | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `pan_right_pane`, `graph_max_col`, `mouse_pan` | supported | supported | supported | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `pan_left_pane`, `graph_max_col`, `mouse_pan` | supported | supported | supported | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `pan_right_pane`, `commit_file_max_col`, `mouse_pan` | supported | supported | supported | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `pan_left_pane`, `commit_file_max_col`, `mouse_pan` | supported | supported | supported | supported |
| `compare-files` | Compare | - | left | boxed, flat | `pan_left_pane`, `commit_file_max_col`, `commit_file_comment_scope`, `mouse_pan` | supported | supported | supported | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary`, `pan_right_pane` | gap: G22 | gap: G22 | gap: G22 | gap: G22 |
| `explorer-tree` | Explorer | - | left | boxed, flat | `dispatch_explorer_tab`, `explorer_wheel`, `draw_explorer_tree` | gap: G13 | n/a: as on the Workspace tree (I1), `h` / `l` move through folders | gap: G13 | gap: G13 |
| `diff-d0` | Workspace | 0 (file focused) | right | boxed, flat | `pan_diff_content`, `diff_pan_max` | supported | supported | supported | supported |
| `diff-d2` | Workspace | 2 | right | boxed, flat | `pan_diff_content`, `diff_pan_max` | supported | supported | supported | supported |
| `compare-diff` | Compare | - | right | boxed, flat | `pan_diff_content`, `diff_pan_max` | supported | supported | supported | supported |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `explorer_preview_pan`, `pan_diff_content` | supported | n/a: I2, `h` gives focus back to the tree and `l` does nothing | supported | supported |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `explorer_preview_pan`, `pan_file_tab` | supported | n/a: I2, `h` gives focus back to the tree and `l` does nothing | supported | supported |

### `double-click` — Double-click

A second press on the same cell within 400 ms is Enter on that row.

| Feature | What | Tests |
| --- | --- | --- |
| `enter` | A double-click does what Enter does on the row. | `double_click_dispatches_enter` |

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
| `explorer-tree` | Explorer | - | left | boxed, flat | `explorer_click`, `explorer_enter` | gap: G14 |
| `explorer-diff` | Explorer (changed file) | - | right | boxed, flat | `explorer_click`, `dispatch_explorer_tab` | n/a: Enter does nothing in the Explorer file pane |
| `explorer-file` | Explorer (clean file) | - | right | boxed, flat | `explorer_click`, `dispatch_explorer_tab` | n/a: Enter does nothing in the Explorer file pane |

### `row-marks` — Row marks

The reviewed eye and the comment mark (open or resolved) on a row or a line gutter.

| Feature | What | Tests |
| --- | --- | --- |
| `reviewed` | A reviewed file paints the eye. | none |
| `comment` | A row or line with comments paints the comment mark. | `commit_file_row_paints_comment_mark_when_file_has_comments`, `graph_commit_row_marks_object_and_file_comments`, `explorer_file_view_paints_a_comment_mark_on_its_line` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `reviewed` | `comment` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `tree_row_marks` | supported | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `graph_row_comments` | n/a: a graph row is a commit, and reviewed marks are per file | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `graph_row_comments` | n/a: a graph row is a commit, and reviewed marks are per file | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `commit_file_row_paint_segments`, `commit_file_row_has_comment` | n/a: reviewed marks are for workspace-tree files and compare ranges; Space in a commit drill says so | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `commit_file_row_paint_segments`, `commit_file_row_has_comment` | n/a: reviewed marks are for workspace-tree files and compare ranges; Space in a commit drill says so | supported |
| `compare-files` | Compare | - | left | boxed, flat | `commit_file_row_paint_segments`, `compare_file_reviewed` | supported | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary` | gap: G21 | gap: G21 |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree`, `explorer_toggle_reviewed` | gap: G16 | gap: G16 |
| `diff-gutter` | Workspace, Compare, Explorer | 0, 2, - | right | boxed, flat | `draw_diff_pane`, `comment_mark_cols` | n/a: reviewed is per file, and the file's list row carries the eye | supported |
| `file-gutter` | Explorer (clean file) | - | right | boxed, flat | `draw_file_body`, `file_line_comment` | n/a: reviewed is per file, and the file's list row carries the eye | supported |

### `watch-flash` — Watch-reload flash

A row that a watch reload adds or changes flashes and fades.

| Feature | What | Tests |
| --- | --- | --- |
| `flash` | A reloaded row flashes. | `flash_paints_node_and_spacer_cursor_wins` |

| Surface | Tab | Depth | Pane | Looks | Code anchors | `flash` |
| --- | --- | --- | --- | --- | --- | --- |
| `tree` | Workspace | 0 | left | boxed, flat | `draw_tree`, `flash_color` | supported |
| `graph-d0` | Workspace | 0 (repo focused) | right | boxed, flat | `draw_graph`, `graph_flash_rows` | supported |
| `graph-d1` | Workspace | 1 | left | boxed, flat | `draw_graph`, `graph_flash_rows` | supported |
| `files-d1` | Workspace | 1 | right | boxed, flat | `draw_commit_file_list`, `commit_file_flash_color` | supported |
| `files-d2` | Workspace | 2 | left | boxed, flat | `draw_commit_file_list`, `commit_file_flash_color` | supported |
| `compare-files` | Compare | - | left | boxed, flat | `draw_commit_file_list`, `commit_file_flash_color` | supported |
| `summary` | Workspace 2, Compare | - | right | boxed, flat | `draw_folder_summary` | gap: G21; `draw_folder_summary` passes no flash |
| `explorer-tree` | Explorer | - | left | boxed, flat | `draw_explorer_tree` | gap: G17 |

## Open gaps

Each open gap is tracked by its id. Fixing one turns its cells into `supported` (or `n/a: <reason>`) and removes its row here.

| Gap | Component | Has it | Missing on |
| --- | --- | --- | --- |
| G4 | `scrollbars` | Graph list scrollbar drag and track click (`SplitDrag::GraphScrollbar`) | File diff vertical bar: `SplitHit` / `SplitDrag` have no diff vertical variant; `diff_scrollbar_x` is read only in tests |
| G5 | `scrollbars` | Graph list and file diff paint a vertical scrollbar | Workspace tree, commit-file lists, Explorer tree |
| G6 | `scrollbars` | Explorer pane on a changed file (`draw_diff_pane`) | Explorer pane on a clean file: `draw_file_body` paints no bar |
| G7 | `icon-popovers` | Comment marks on tree, graph, and commit-file rows open a popover | Diff gutter and clean-file gutter: `IconTarget` has no line variant |
| G9 | `file-lists` | Commit-file and compare lists record icon hits and scroll | Folder summary: `draw_folder_summary` takes `&AppState`, records no icon hit, and folds overflow into `… N more` |
| G11 | `resize-edges` | Workspace and compare tabs: divider drag, in-diff rule drag, diff horizontal thumb drag, edge hover | Explorer tab: `resize_edges_take_mouse` excludes it, `explorer_click` never calls `hit_split`, and `diff_split_rule_x` is forced to `None` |
| G12 | `line-viewers` | Workspace and compare diffs: a click picks the row (`click_diff`) | Explorer pane on a changed file: `explorer_click` only arms a text selection |
| G13 | `horizontal-pan` | Workspace tree pans | Explorer tree: `PanDiff` with the tree focused does nothing, the horizontal wheel does nothing, and the paint uses offset 0 |
| G14 | `double-click` | Workspace lists (`click_row`, `last_click`) | Explorer tree: no double-click handling in `explorer_click` |
| G15 | `file-lists` | Workspace tree and commit-file lists: devicons, icon hits, popovers, `gh` | Explorer tree rows: `explorer_row_segments` sets `icon: None` and records no hit; `focused_row_icons` is empty on an Explorer tab |
| G16 | `row-marks` | Workspace tree and compare list paint the reviewed eye and comment marks | Explorer tree paints no mark (code comment on `explorer_toggle_reviewed`); the docs do not say so |
| G17 | `watch-flash` | Tree, graph, and commit-file rows flash on a watch reload | Explorer tree: the flash is passed as `None` in `draw_explorer_tree` |
| G21 | `row-marks` | Commit-file and compare list rows paint the reviewed eye, comment marks, and the watch-reload flash | Folder summary rows of the same files: `draw_folder_summary` paints `file_change_segments` with no mark and no flash |
| G22 | `horizontal-pan` | Commit-file list rows pan with the keys and the horizontal wheel | Folder summary: its rows always paint at offset 0; a pan key or a horizontal wheel over it pans the hidden file diff (`pan_right_pane` → `pan_diff_content`) |
| G23 | `scrollbars` | Graph list and file diff paint a horizontal scrollbar with drag and track click | Workspace tree and commit-file lists pan sideways with no horizontal bar |

## Intentional differences

| Id | Difference | Stated in |
| --- | --- | --- |
| I1 | Workspace tree file row: `h` / `l` fold; commit-file file row: `h` / `l` pan | `docs/tui-rust.md` (`h` / `l` key row) |
| I2 | Explorer file pane: `h` gives focus back to the tree, `l` does nothing | `docs/tui-model.md` (Explorer keys) |
| I3 | Clean-file view: `j` / `k` scroll at the edge, not the middle | `docs/tui-rust.md` (viewport rule), `docs/tui-model.md` (sticky file view top) |
| I4 | Explorer: `??` untracked, `●` dirty folder, no `t` toggle, `Ctrl-o` refused | `docs/tui-model.md` (Explorer tabs) |
