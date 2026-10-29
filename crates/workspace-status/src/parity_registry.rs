//! Parser and CI checks for the cross-surface parity registry,
//! `docs/parity.md`.
//!
//! The registry lists each shared TUI component, every surface it appears
//! on, and per feature one status: `supported`, `n/a: <reason>`, or
//! `gap: G<n>`. The tests here fail when the document breaks its format,
//! when a gap or an intentional difference is not indexed, or when a named
//! code anchor or test no longer exists as a `fn` under `crates/`. Behaviour
//! tests read the registry through [`supported_surface_looks`], so a
//! surface added to the registry without a matching test case (or the
//! reverse) fails too.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// The registry text, compiled in so a doc edit rebuilds the tests.
pub(crate) const REGISTRY: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/parity.md"));

/// Leading columns of every surface table, before the feature columns.
const SURFACE_COLUMNS: [&str; 6] = ["Surface", "Tab", "Depth", "Pane", "Looks", "Code anchors"];
/// Header of every feature table.
const FEATURE_COLUMNS: [&str; 3] = ["Feature", "What", "Tests"];
/// Header of the open-gap index.
const GAP_COLUMNS: [&str; 4] = ["Gap", "Component", "Has it", "Missing on"];
/// Header of the intentional-difference index.
const INTENTIONAL_COLUMNS: [&str; 3] = ["Id", "Difference", "Stated in"];
/// Pane looks a surface row may name.
const LOOKS: [&str; 2] = ["boxed", "flat"];

/// One feature status cell of a surface row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
    /// The surface has the feature.
    Supported,
    /// The feature does not apply to the surface, with the reason.
    NotApplicable(String),
    /// An open gap: the surface lacks the feature.
    Gap {
        /// The number of `G<n>`.
        id: u32,
        /// Text after `;`, required when the gap belongs to another
        /// component.
        note: Option<String>,
    },
}

/// One feature of a component: a column of its surface table.
#[derive(Clone, Debug)]
pub(crate) struct Feature {
    /// Feature id, without backticks.
    pub(crate) id: String,
    /// Tests that cover the feature (`fn` names).
    pub(crate) tests: Vec<String>,
}

/// One surface row of a component.
#[derive(Clone, Debug)]
pub(crate) struct Surface {
    /// Surface id, without backticks.
    pub(crate) id: String,
    /// Pane looks the row covers (`boxed`, `flat`).
    pub(crate) looks: Vec<String>,
    /// Code anchors (`fn` names) of the surface's code path.
    pub(crate) anchors: Vec<String>,
    /// One cell per feature, in feature order.
    pub(crate) cells: Vec<Cell>,
}

/// One component section.
#[derive(Clone, Debug)]
pub(crate) struct Component {
    /// Component id, without backticks.
    pub(crate) id: String,
    /// Feature columns, in order.
    pub(crate) features: Vec<Feature>,
    /// Surface rows, in order.
    pub(crate) surfaces: Vec<Surface>,
}

/// The parsed registry.
#[derive(Clone, Debug, Default)]
pub(crate) struct Registry {
    /// Components, in document order.
    pub(crate) components: Vec<Component>,
    /// Open-gap index: gap number → home component id.
    pub(crate) gaps: BTreeMap<u32, String>,
    /// Intentional-difference index: `I<n>` numbers.
    pub(crate) intentional: BTreeSet<u32>,
}

/// Parse the registry `text`. `Err` lists every format problem, each with
/// its 1-based line number.
pub(crate) fn parse(text: &str) -> Result<Registry, Vec<String>> {
    let mut errors = Vec::new();
    let mut registry = Registry::default();
    let mut section = String::new();
    let mut component: Option<(usize, String, Vec<Table>)> = None;
    let mut table: Option<Table> = None;
    let lines: Vec<&str> = text.lines().collect();
    for (index, raw) in lines.iter().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.starts_with('|') {
            let cells = split_row(line);
            match table.as_mut() {
                None => {
                    table = Some(Table {
                        line: line_no,
                        header: cells,
                        rows: Vec::new(),
                        separator_seen: false,
                    })
                }
                Some(t) if !t.separator_seen => {
                    if !cells
                        .iter()
                        .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
                    {
                        errors.push(format!(
                            "line {line_no}: table header needs a `| --- |` separator row"
                        ));
                    }
                    t.separator_seen = true;
                }
                Some(t) => t.rows.push((line_no, cells)),
            }
            continue;
        }
        if let Some(done) = table.take() {
            place_table(done, &section, &mut component, &mut registry, &mut errors);
        }
        if let Some(heading) = line.strip_prefix("## ") {
            finish_component(component.take(), &mut registry, &mut errors);
            section = heading.trim().to_string();
        } else if let Some(heading) = line.strip_prefix("### ") {
            finish_component(component.take(), &mut registry, &mut errors);
            if section == "Components" {
                match backticked(heading.trim()).filter(|id| is_kebab(id)) {
                    Some(id) => component = Some((line_no, id, Vec::new())),
                    None => errors.push(format!(
                        "line {line_no}: a component heading starts with a backticked kebab-case id"
                    )),
                }
            }
        }
    }
    if let Some(done) = table.take() {
        place_table(done, &section, &mut component, &mut registry, &mut errors);
    }
    finish_component(component.take(), &mut registry, &mut errors);
    if registry.components.is_empty() {
        errors.push("no component under `## Components`".into());
    }
    if errors.is_empty() {
        Ok(registry)
    } else {
        Err(errors)
    }
}

/// A Markdown table as read: header, separator flag, body rows.
struct Table {
    line: usize,
    header: Vec<String>,
    rows: Vec<(usize, Vec<String>)>,
    separator_seen: bool,
}

/// Cells of one `| a | b |` row, trimmed.
fn split_row(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// The text between the first pair of backticks, when `text` starts with
/// one.
fn backticked(text: &str) -> Option<String> {
    let rest = text.strip_prefix('`')?;
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

/// `cell` is exactly one backticked id.
fn backticked_id(cell: &str) -> Option<String> {
    let id = backticked(cell)?;
    (cell == format!("`{id}`")).then_some(id)
}

fn is_kebab(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_rust_ident(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A comma-separated list of backticked Rust identifiers.
fn fn_names(cell: &str) -> Option<Vec<String>> {
    let names: Option<Vec<String>> = cell
        .split(',')
        .map(|part| backticked_id(part.trim()).filter(|name| is_rust_ident(name)))
        .collect();
    names.filter(|names| !names.is_empty())
}

/// Parse a status cell: `supported`, `n/a: <reason>`, or
/// `gap: G<n>` with an optional `; <note>`.
pub(crate) fn parse_cell(cell: &str) -> Result<Cell, String> {
    if cell == "supported" {
        return Ok(Cell::Supported);
    }
    if let Some(reason) = cell.strip_prefix("n/a:") {
        let reason = reason.trim();
        if reason.split_whitespace().count() < 2 {
            return Err(format!(
                "`{cell}`: an `n/a:` cell needs a reason of two words or more"
            ));
        }
        return Ok(Cell::NotApplicable(reason.to_string()));
    }
    if let Some(rest) = cell.strip_prefix("gap:") {
        let (id, note) = match rest.split_once(';') {
            Some((id, note)) => (id.trim(), Some(note.trim().to_string())),
            None => (rest.trim(), None),
        };
        let number = id
            .strip_prefix('G')
            .filter(|n| !n.starts_with('0'))
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or_else(|| format!("`{cell}`: a gap id is `G<n>`, n from 1"))?;
        if note.as_deref() == Some("") {
            return Err(format!("`{cell}`: a `;` after a gap id needs a note"));
        }
        return Ok(Cell::Gap { id: number, note });
    }
    Err(format!(
        "`{cell}`: a cell is `supported`, `n/a: <reason>`, or `gap: G<n>`"
    ))
}

fn place_table(
    table: Table,
    section: &str,
    component: &mut Option<(usize, String, Vec<Table>)>,
    registry: &mut Registry,
    errors: &mut Vec<String>,
) {
    if let Some((_, _, tables)) = component.as_mut() {
        tables.push(table);
        return;
    }
    let line = table.line;
    match section {
        "Open gaps" => {
            if table.header != GAP_COLUMNS {
                errors.push(format!(
                    "line {line}: the gap index header is {GAP_COLUMNS:?}"
                ));
                return;
            }
            for (line, row) in &table.rows {
                if row.len() != GAP_COLUMNS.len() || row.iter().any(String::is_empty) {
                    errors.push(format!("line {line}: a gap index row fills every column"));
                    continue;
                }
                let id = match parse_cell(&format!("gap: {}", row[0])) {
                    Ok(Cell::Gap { id, note: None }) => id,
                    _ => {
                        errors.push(format!("line {line}: gap id `{}` is not `G<n>`", row[0]));
                        continue;
                    }
                };
                let Some(home) = backticked_id(&row[1]) else {
                    errors.push(format!("line {line}: gap component is one backticked id"));
                    continue;
                };
                if registry.gaps.insert(id, home).is_some() {
                    errors.push(format!("line {line}: G{id} is listed twice"));
                }
            }
        }
        "Intentional differences" => {
            if table.header != INTENTIONAL_COLUMNS {
                errors.push(format!(
                    "line {line}: the intentional-difference header is {INTENTIONAL_COLUMNS:?}"
                ));
                return;
            }
            for (line, row) in &table.rows {
                let id = row
                    .first()
                    .and_then(|id| id.strip_prefix('I'))
                    .and_then(|n| n.parse::<u32>().ok());
                match id {
                    Some(id) if row.len() == 3 && row.iter().all(|c| !c.is_empty()) => {
                        if !registry.intentional.insert(id) {
                            errors.push(format!("line {line}: I{id} is listed twice"));
                        }
                    }
                    _ => errors.push(format!(
                        "line {line}: an intentional-difference row is `I<n>` plus two filled columns"
                    )),
                }
            }
        }
        "Components" => errors.push(format!(
            "line {line}: a table under `## Components` belongs under a component heading (`### `)"
        )),
        _ => {}
    }
}

fn finish_component(
    component: Option<(usize, String, Vec<Table>)>,
    registry: &mut Registry,
    errors: &mut Vec<String>,
) {
    let Some((line, id, tables)) = component else {
        return;
    };
    if registry.components.iter().any(|c| c.id == id) {
        errors.push(format!("line {line}: component `{id}` is listed twice"));
    }
    let [features, surfaces] = tables.as_slice() else {
        errors.push(format!(
            "line {line}: component `{id}` has exactly two tables, features then surfaces"
        ));
        return;
    };
    let features = parse_features(features, &id, errors);
    let surfaces = parse_surfaces(surfaces, &id, &features, errors);
    registry.components.push(Component {
        id,
        features,
        surfaces,
    });
}

fn parse_features(table: &Table, component: &str, errors: &mut Vec<String>) -> Vec<Feature> {
    if table.header != FEATURE_COLUMNS {
        errors.push(format!(
            "line {}: `{component}` feature table header is {FEATURE_COLUMNS:?}",
            table.line
        ));
        return Vec::new();
    }
    let mut features: Vec<Feature> = Vec::new();
    for (line, row) in &table.rows {
        if row.len() != FEATURE_COLUMNS.len() || row[1].is_empty() {
            errors.push(format!(
                "line {line}: a feature row fills Feature, What, and Tests"
            ));
            continue;
        }
        let Some(id) = backticked_id(&row[0]).filter(|id| is_kebab(id)) else {
            errors.push(format!(
                "line {line}: a feature id is one backticked kebab-case id"
            ));
            continue;
        };
        if features.iter().any(|f| f.id == id) {
            errors.push(format!(
                "line {line}: `{component}` lists feature `{id}` twice"
            ));
        }
        let tests = if row[2] == "none" {
            Vec::new()
        } else if let Some(names) = fn_names(&row[2]) {
            names
        } else {
            errors.push(format!(
                "line {line}: Tests is `none` or backticked test `fn` names, comma-separated"
            ));
            Vec::new()
        };
        features.push(Feature { id, tests });
    }
    if features.is_empty() {
        errors.push(format!("line {}: `{component}` has no feature", table.line));
    }
    features
}

fn parse_surfaces(
    table: &Table,
    component: &str,
    features: &[Feature],
    errors: &mut Vec<String>,
) -> Vec<Surface> {
    let expected: Vec<String> = SURFACE_COLUMNS
        .iter()
        .map(|c| c.to_string())
        .chain(features.iter().map(|f| format!("`{}`", f.id)))
        .collect();
    if table.header != expected {
        errors.push(format!(
            "line {}: `{component}` surface table header must be {expected:?} (the fixed columns, then each feature id in feature-table order), found {:?}",
            table.line, table.header
        ));
        return Vec::new();
    }
    let mut surfaces: Vec<Surface> = Vec::new();
    for (line, row) in &table.rows {
        if row.len() != expected.len() {
            errors.push(format!(
                "line {line}: a surface row has {} cells, the header {}",
                row.len(),
                expected.len()
            ));
            continue;
        }
        let Some(id) = backticked_id(&row[0]).filter(|id| is_kebab(id)) else {
            errors.push(format!(
                "line {line}: a surface id is one backticked kebab-case id"
            ));
            continue;
        };
        if surfaces.iter().any(|s| s.id == id) {
            errors.push(format!(
                "line {line}: `{component}` lists surface `{id}` twice"
            ));
        }
        if row[1..4].iter().any(String::is_empty) {
            errors.push(format!(
                "line {line}: Tab, Depth, and Pane are filled (`-` for none)"
            ));
        }
        let looks: Vec<String> = row[4].split(',').map(|l| l.trim().to_string()).collect();
        let unique: HashSet<&String> = looks.iter().collect();
        if looks.iter().any(|l| !LOOKS.contains(&l.as_str())) || unique.len() != looks.len() {
            errors.push(format!(
                "line {line}: Looks is `boxed`, `flat`, or `boxed, flat`, found `{}`",
                row[4]
            ));
        }
        let anchors = fn_names(&row[5]).unwrap_or_else(|| {
            errors.push(format!(
                "line {line}: Code anchors is one or more backticked `fn` names, comma-separated"
            ));
            Vec::new()
        });
        let mut cells = Vec::new();
        for (feature, cell) in features.iter().zip(&row[SURFACE_COLUMNS.len()..]) {
            match parse_cell(cell) {
                Ok(cell) => cells.push(cell),
                Err(e) => errors.push(format!(
                    "line {line}: `{component}` / `{id}` / `{}`: {e}",
                    feature.id
                )),
            }
        }
        surfaces.push(Surface {
            id,
            looks,
            anchors,
            cells,
        });
    }
    if surfaces.is_empty() {
        errors.push(format!("line {}: `{component}` has no surface", table.line));
    }
    surfaces
}

/// The parsed checked-in registry. Panics with every format problem.
pub(crate) fn registry() -> Registry {
    parse(REGISTRY).unwrap_or_else(|errors| panic!("docs/parity.md format:\n{}", errors.join("\n")))
}

/// `(surface, look)` pairs of `component` whose `feature` cell is
/// `supported`, sorted. A behaviour test that drives a feature on every
/// surface compares its own case list to this.
pub(crate) fn supported_surface_looks(component: &str, feature: &str) -> Vec<(String, String)> {
    let registry = registry();
    let component = registry
        .components
        .iter()
        .find(|c| c.id == component)
        .unwrap_or_else(|| panic!("docs/parity.md has no component `{component}`"));
    let column = component
        .features
        .iter()
        .position(|f| f.id == feature)
        .unwrap_or_else(|| panic!("`{}` has no feature `{feature}`", component.id));
    let mut pairs: Vec<(String, String)> = component
        .surfaces
        .iter()
        .filter(|s| s.cells.get(column) == Some(&Cell::Supported))
        .flat_map(|s| s.looks.iter().map(|look| (s.id.clone(), look.clone())))
        .collect();
    pairs.sort();
    pairs
}

/// Problems with the gap and intentional-difference indexes of `registry`.
pub(crate) fn index_problems(registry: &Registry) -> Vec<String> {
    let mut problems = Vec::new();
    let mut used: BTreeSet<u32> = BTreeSet::new();
    let mut cited: BTreeSet<u32> = BTreeSet::new();
    for component in &registry.components {
        for surface in &component.surfaces {
            for (feature, cell) in component.features.iter().zip(&surface.cells) {
                let at = format!("`{}` / `{}` / `{}`", component.id, surface.id, feature.id);
                match cell {
                    Cell::Gap { id, note } => {
                        used.insert(*id);
                        match registry.gaps.get(id) {
                            None => problems.push(format!("{at}: G{id} is not in `## Open gaps`")),
                            Some(home) if *home != component.id && note.is_none() => {
                                problems.push(format!(
                                    "{at}: G{id} belongs to `{home}`; a cell in another component says why after `;`"
                                ))
                            }
                            Some(_) => {}
                        }
                    }
                    Cell::NotApplicable(reason) => cited.extend(intentional_ids(reason)),
                    Cell::Supported => {}
                }
            }
        }
    }
    for (id, home) in &registry.gaps {
        if !used.contains(id) {
            problems.push(format!("G{id} is indexed but no cell names it"));
        }
        if !registry.components.iter().any(|c| c.id == *home) {
            problems.push(format!("G{id}: no component `{home}`"));
        }
    }
    for id in &registry.intentional {
        if !cited.contains(id) {
            problems.push(format!("I{id} is indexed but no `n/a:` reason cites it"));
        }
    }
    for id in cited.difference(&registry.intentional) {
        problems.push(format!(
            "I{id} is cited but not in `## Intentional differences`"
        ));
    }
    problems
}

/// `I<n>` tokens in an `n/a:` reason.
fn intentional_ids(reason: &str) -> Vec<u32> {
    reason
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|word| word.strip_prefix('I')?.parse().ok())
        .collect()
}

/// Every `fn` name declared in the `.rs` files under `root`.
pub(crate) fn declared_fns(root: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut dirs: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    dirs.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                names.extend(fn_names_in(&text));
            }
        }
    }
    names
}

/// Names after each `fn ` keyword in `text`.
///
/// Comments do not count: a line's text from `//` on (`//`, `///`, `//!`,
/// and a trailing comment) is skipped, so a stale doc mention of a renamed
/// fn cannot keep its anchor alive.
fn fn_names_in(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let code = line.split_once("//").map_or(line, |(code, _)| code);
        let bytes = code.as_bytes();
        let mut from = 0;
        while let Some(at) = code[from..].find("fn ") {
            let start = from + at;
            from = start + 3;
            let keyword = start == 0 || !is_ident_byte(bytes[start - 1]);
            if !keyword {
                continue;
            }
            let name: String = code[from..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                names.push(name);
            }
        }
    }
    names
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Code anchors and test names of `registry` that `declared` lacks, as
/// `component / surface-or-feature: name`.
pub(crate) fn missing_fns(registry: &Registry, declared: &HashSet<String>) -> Vec<String> {
    let mut missing = Vec::new();
    for component in &registry.components {
        for surface in &component.surfaces {
            for name in surface.anchors.iter().filter(|n| !declared.contains(*n)) {
                missing.push(format!(
                    "`{}` / `{}`: anchor `fn {name}`",
                    component.id, surface.id
                ));
            }
        }
        for feature in &component.features {
            for name in feature.tests.iter().filter(|n| !declared.contains(*n)) {
                missing.push(format!(
                    "`{}` / `{}`: test `fn {name}`",
                    component.id, feature.id
                ));
            }
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `crates/` directory the anchors are looked up in.
    fn crates_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crate dir has a parent")
            .to_path_buf()
    }

    /// A minimal valid registry with one component and both indexes, with
    /// `cell` as the `b` feature cell of surface `s1` and `extra` appended
    /// to the surface table.
    fn sample(cell: &str, extra: &str) -> String {
        format!(
            "# Registry\n\n## Components\n\n### `comp` — A component\n\n\
             | Feature | What | Tests |\n| --- | --- | --- |\n\
             | `a` | Feature a. | none |\n| `b` | Feature b. | `sample` |\n\n\
             | Surface | Tab | Depth | Pane | Looks | Code anchors | `a` | `b` |\n\
             | --- | --- | --- | --- | --- | --- | --- | --- |\n\
             | `s1` | Workspace | 0 | left | boxed, flat | `sample` | n/a: I1 keeps it apart | {cell} |\n\
             {extra}\n\
             ## Open gaps\n\n| Gap | Component | Has it | Missing on |\n| --- | --- | --- | --- |\n\
             | G1 | `comp` | s0 | s1 |\n\n\
             ## Intentional differences\n\n| Id | Difference | Stated in |\n| --- | --- | --- |\n\
             | I1 | A reason. | here |\n"
        )
    }

    fn errors_of(text: &str) -> String {
        match parse(text) {
            Ok(registry) => index_problems(&registry).join("\n"),
            Err(errors) => errors.join("\n"),
        }
    }

    /// Every cell of `docs/parity.md` is `supported`, `n/a: <reason>`, or
    /// `gap: G<n>`, every surface table's header names the component's
    /// features in order, and every row fills its columns.
    #[test]
    fn parity_registry_cells_are_supported_na_or_gap() {
        let registry = registry();
        assert!(
            registry.components.len() >= 11,
            "every shared component is listed"
        );
        for component in &registry.components {
            for surface in &component.surfaces {
                assert_eq!(
                    surface.cells.len(),
                    component.features.len(),
                    "`{}` / `{}`",
                    component.id,
                    surface.id
                );
            }
        }
    }

    /// Every `gap:` cell names an indexed gap, a gap used outside its home
    /// component carries a note, every indexed gap is used, and the
    /// intentional differences and the `n/a:` reasons that cite them match.
    #[test]
    fn parity_registry_gaps_and_intentional_differences_are_indexed() {
        let problems = index_problems(&registry());
        assert!(
            problems.is_empty(),
            "docs/parity.md:\n{}",
            problems.join("\n")
        );
    }

    /// Every code anchor and every named test is a `fn` under `crates/`,
    /// so a rename or a moved code path forces a registry update.
    #[test]
    fn parity_registry_code_anchors_and_tests_exist() {
        let declared = declared_fns(&crates_dir());
        let missing = missing_fns(&registry(), &declared);
        assert!(
            missing.is_empty(),
            "docs/parity.md names fns that no longer exist under crates/ (rename the anchor in the registry):\n{}",
            missing.join("\n")
        );
    }

    /// The checks reject each broken form: a bad cell, an `n/a:` with no
    /// reason, a malformed or unindexed gap id, an unknown look, a feature
    /// column out of order, a missing anchor, and an uncited I-id.
    #[test]
    fn parity_registry_rejects_broken_entries() {
        assert_eq!(errors_of(&sample("gap: G1", "")), "", "the sample is valid");
        for (cell, expected) in [
            ("", "a cell is `supported`"),
            ("yes", "a cell is `supported`"),
            ("n/a:", "needs a reason"),
            ("n/a: x", "needs a reason"),
            ("gap: 7", "a gap id is `G<n>`"),
            ("gap: G07", "a gap id is `G<n>`"),
            ("gap: G1;", "needs a note"),
            ("gap: G2", "G2 is not in `## Open gaps`"),
        ] {
            let errors = errors_of(&sample(cell, ""));
            assert!(errors.contains(expected), "`{cell}`: {errors}");
        }
        let unused = errors_of(&sample("supported", ""));
        assert!(
            unused.contains("G1 is indexed but no cell names it"),
            "{unused}"
        );
        let uncited = sample("gap: G1", "").replace("n/a: I1 keeps it apart", "supported");
        assert!(
            errors_of(&uncited).contains("I1 is indexed but no `n/a:` reason cites it"),
            "uncited I-id"
        );
        let unindexed = sample("gap: G1", "").replace("I1 keeps", "I2 keeps");
        assert!(
            errors_of(&unindexed).contains("I2 is cited but not in"),
            "unindexed I-id"
        );
        let looks = sample("gap: G1", "").replace("boxed, flat", "boxed, wide");
        assert!(errors_of(&looks).contains("Looks is"), "unknown look");
        let order = sample("gap: G1", "").replace("| `a` | `b` |\n", "| `b` | `a` |\n");
        assert!(
            errors_of(&order).contains("surface table header"),
            "column order"
        );
        let duplicate = sample(
            "gap: G1",
            "| `s1` | Compare | - | left | flat | `sample` | supported | supported |\n",
        );
        assert!(errors_of(&duplicate).contains("lists surface `s1` twice"));
        let declared: HashSet<String> = ["other".to_string()].into_iter().collect();
        let registry = parse(&sample("gap: G1", "")).expect("valid sample");
        let missing = missing_fns(&registry, &declared).join("\n");
        assert!(missing.contains("anchor `fn sample`"), "{missing}");
        assert!(missing.contains("test `fn sample`"), "{missing}");
        let home = sample("gap: G1", "").replace("| G1 | `comp` |", "| G1 | `other` |");
        assert!(
            errors_of(&home).contains("belongs to `other`"),
            "foreign gap needs a note"
        );
    }

    #[test]
    fn fn_names_in_reads_declarations_only() {
        let names = fn_names_in(
            "pub(crate) fn alpha(x: u8) {}\nfn beta<T>() {}\nlet info = 1; // defn gamma\n",
        );
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    }

    /// A renamed fn that a comment still mentions is not declared: `//`,
    /// `///`, `//!`, and trailing comments do not count.
    #[test]
    fn fn_names_in_skips_comments() {
        let text = "/// Was fn old_doc before the rename.\n\
                    //! Crate note: fn old_module.\n\
                    // fn old_line()\n\
                    fn kept() {} // was fn old_trailing\n";
        assert_eq!(fn_names_in(text), vec!["kept".to_string()]);
        let declared: HashSet<String> = fn_names_in(text).into_iter().collect();
        let registry =
            parse(&sample("gap: G1", "").replace("`sample`", "`old_doc`")).expect("valid sample");
        let missing = missing_fns(&registry, &declared).join("\n");
        assert!(missing.contains("anchor `fn old_doc`"), "{missing}");
    }
}
