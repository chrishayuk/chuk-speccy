//! Starter game templates for `speccy new` (spec 08, L0 ergonomics). Each template
//! is a real, checked-in sample that **crosses the fidelity dial** — it compiles both
//! under `rustc` (a host [`Game`](crate::Game)) and under `rustz80` (a bootable tape)
//! — so every scaffolded game starts life as one-source-two-artifacts. Scaffolding is
//! just renaming the template's state struct to the new game's name; the dual-compile
//! guarantee is held by `tests/dial.rs` + the tests here.

/// A named starter template: its state struct name (the only thing scaffolding
/// rewrites) and the dialect source.
pub struct Template {
    pub name: &'static str,
    pub about: &'static str,
    pub struct_name: &'static str,
    pub src: &'static str,
}

/// Every starter template, in `speccy new --template <name>` order. Add a row (and a
/// dial.rs check) to ship a new one.
pub const TEMPLATES: &[Template] = &[
    Template {
        name: "blank",
        about: "move a blob — the minimal starter",
        struct_name: "Starter",
        src: include_str!("../samples/blank.rs"),
    },
    Template {
        name: "snake",
        about: "a grid snake (RNG food, growth)",
        struct_name: "Snake",
        src: include_str!("../samples/snake_game.rs"),
    },
];

/// The template named `name`, if any.
pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Scaffold `template` as a game named `name` — the template source with its state
/// struct renamed to `PascalCase(name)`. `None` if the template is unknown.
pub fn scaffold(template: &str, name: &str) -> Option<String> {
    let t = find(template)?;
    Some(t.src.replace(t.struct_name, &pascal(name)))
}

/// Turn a free-form game name into a valid PascalCase Rust type identifier
/// (`"my-cool game"` → `MyCoolGame`); prefix `Game` if it would start with a digit or
/// be empty.
fn pascal(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            if upper {
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
            upper = false;
        } else {
            upper = true; // word boundary
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_numeric()) {
        out.insert_str(0, "Game");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_makes_valid_identifiers() {
        assert_eq!(pascal("snake"), "Snake");
        assert_eq!(pascal("my-cool game"), "MyCoolGame");
        assert_eq!(pascal("2048"), "Game2048");
        assert_eq!(pascal(""), "Game");
    }

    #[test]
    fn scaffold_renames_the_state_struct() {
        let s = scaffold("blank", "hero").expect("blank exists");
        assert!(s.contains("struct Hero"), "renamed to the game name");
        assert!(!s.contains("Starter"), "no trace of the placeholder name");
        assert!(scaffold("nope", "x").is_none(), "unknown template");
    }

    // The dial guarantee: every template (and a scaffolded instance) compiles pure.
    #[cfg(feature = "compile")]
    #[test]
    fn every_template_compiles_pure() {
        for t in TEMPLATES {
            crate::compile::compile_game(t.src, "T")
                .unwrap_or_else(|e| panic!("`{}` template should compile pure: {e}", t.name));
        }
    }

    #[cfg(feature = "compile")]
    #[test]
    fn a_scaffolded_game_compiles_pure() {
        let s = scaffold("blank", "hero").unwrap();
        crate::compile::compile_game(&s, "HERO").expect("scaffolded blank compiles pure");
    }
}
