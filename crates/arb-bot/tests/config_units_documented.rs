//! Item 649 enforcement: every numeric field of the runtime `Config` struct
//! tree in `crates/arb-bot/src/config.rs` must carry explicit unit
//! documentation in its doc comment.
//!
//! Rationale: the flattened runtime config erases the unit-typed wrappers
//! (`arb_config::Bps` / `Lamports` / `Milliseconds` / `Seconds` / `Slots`),
//! so the only remaining carrier of units is the doc text. A numeric field
//! without an explicit unit allows silent unit mismatches (lamports vs SOL,
//! ms vs slots, USD vs raw) to enter financial decisions. This test scans the
//! source file so documentation drift fails the build rather than review.

/// Substrings accepted as unit evidence inside the doc run above a numeric
/// field. Deliberately a closed list so the guard fails loudly when a new
/// numeric field is added without units.
const UNIT_TOKENS: &[&str] = &[
    "lamports",
    "bps",
    "milliseconds",
    "seconds",
    "slots",
    "usd",
    "fraction",
    "ratio",
    "multiplier",
    "coefficient",
    "count",
    "raw units",
    "hops",
];

/// Numeric primitive type fragments marking a field as carrying a quantity.
const NUMERIC_TYPE_FRAGMENTS: &[&str] = &["u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64"];

fn config_rs_path() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/src/config.rs").to_string()
}

#[test]
fn every_numeric_config_field_documents_its_units() {
    let path = config_rs_path();
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("config.rs must be readable: {e}"));

    let mut checked = 0usize;
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("pub ") else {
            continue;
        };
        let Some((field, ty)) = rest.split_once(':') else {
            continue;
        };
        let ty = ty.trim().trim_end_matches(',');
        let is_numeric = NUMERIC_TYPE_FRAGMENTS
            .iter()
            .any(|frag| has_type_fragment(ty, frag));
        if !is_numeric {
            continue;
        }
        checked += 1;

        // walk up over the contiguous doc run (`///` lines) and serde
        // attributes attached to the field; only `///` bodies count as units
        // evidence, `#[serde(...)]` lines are transparent
        let mut doc = String::new();
        let mut cursor = idx;
        while cursor > 0 {
            let prev = source.lines().nth(cursor - 1).unwrap().trim_start();
            if let Some(body) = prev.strip_prefix("///") {
                doc.push_str(body);
                doc.push(' ');
                cursor -= 1;
            } else if prev.starts_with("#[") {
                cursor -= 1;
            } else {
                break;
            }
        }
        let doc_lc = doc.to_lowercase();
        assert!(
            UNIT_TOKENS.iter().any(|u| doc_lc.contains(u)),
            "field `{}` (type `{}`) at {}:{} has no explicit units in its doc \
             comment (expected one of {UNIT_TOKENS:?}); every numeric config \
             field must document its units (item 649)",
            field.trim(),
            ty,
            path,
            idx + 1,
        );
    }
    assert!(
        checked >= 30,
        "expected at least 30 numeric config fields, found {checked} — \
         the scan is broken or the struct tree changed drastically",
    );
}

/// `has_type_fragment("Option<u64>", "u64") == true`;
/// `has_type_fragment("Vec<String>", "u64") == false` — boundary-aware exact
/// type-word comparison, so `u8` never matches inside `u64`.
fn has_type_fragment(ty: &str, frag: &str) -> bool {
    ty.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == frag)
}
