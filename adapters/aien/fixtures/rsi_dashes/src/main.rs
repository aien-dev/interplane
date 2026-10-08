//! VAC M5 fixture. `rsi-dashes holdout <input>` prints one answer line for the judge's holdout
//! cases; with no arguments it prints the README title. The README is compiled in, so the binary
//! the judge builds answers for exactly the README bytes in its tree.

const README: &str = include_str!("../README.md");

fn has_banned_dashes(text: &str) -> bool {
    text.contains('\u{2014}') || text.contains('\u{2013}')
}

fn holdout(input: &str) -> &'static str {
    match input {
        "check_readme_dashes" if has_banned_dashes(README) => "DASHES_PRESENT",
        "check_readme_dashes" => "DASHES_ABSENT",
        "readme_has_title" if README.starts_with("# ") => "TITLE_OK",
        "readme_has_title" => "TITLE_MISSING",
        "" => "EMPTY_OK",
        "0" => "DIV0_GUARDED",
        _ => "UNKNOWN_CASE",
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("holdout") => println!("{}", holdout(args.get(2).map_or("", String::as_str))),
        _ => println!("{}", README.lines().next().unwrap_or("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_has_no_banned_dashes() {
        assert!(!has_banned_dashes(README), "README still has em or en dashes");
    }

    #[test]
    fn readme_keeps_its_title() {
        assert_eq!(holdout("readme_has_title"), "TITLE_OK");
    }
}
