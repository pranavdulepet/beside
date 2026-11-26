// Only a plainly stated, unqualified pantry item can cancel a suggested
// purchase. Inventory with a count or a qualifier still needs human judgment.
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

static STOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:already in (?:the|my|our) (?:kitchen|pantry|fridge)|(?:already )?on hand|in (?:the|my|our) pantry|(?:i |we )?already have)\s*(?::|[-–—])?\s+(.+)$",
).unwrap()
});
static SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*,\s*(?:and\s+)?|\s+and\s+").unwrap());
static PURCHASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*[-*•]?\s*(?:[0-9]+(?:[./–-][0-9]+)?\s+|(?:a|an|one|two)\s+)?(?:(?:bags?|blocks?|bottles?|box(?:es)?|bunch(?:es)?|cans?|cartons?|containers?|dozen|heads?|jars?|loaves|packs?|packages?|packets?|pieces?|pounds?|lbs?|ounces?|oz|grams?|kilograms?|kg)\s+(?:of\s+)?)?",
).unwrap()
});

pub(super) fn stock(note: &str) -> HashSet<String> {
    note.lines()
        .filter_map(|line| STOCK.captures(line))
        .flat_map(|c| {
            SEPARATOR
                .split(&c[1])
                .map(|v| {
                    v.trim_matches(|c: char| c.is_whitespace() || c == '.')
                        .to_lowercase()
                })
                .collect::<Vec<_>>()
        })
        .filter(|item| {
            !item.is_empty()
                && item.split_whitespace().count() <= 5
                && item
                    .chars()
                    .all(|c| c.is_alphabetic() || c == ' ' || c == '-')
        })
        .collect()
}

pub(super) fn in_stock(line: &str, stocked: &HashSet<String>) -> bool {
    let text = line.trim().trim_matches('*').to_lowercase();
    let item = PURCHASE.replace(&text, "");
    let item = item.trim_start_matches([' ', '–', '—', ':']);
    let item = item
        .split_once(" (")
        .filter(|(_, note)| note.contains("optional") || note.contains("if "))
        .map_or(item, |(name, _)| name);
    item.split(" or ")
        .any(|name| stocked.contains(name.trim().trim_end_matches('.')))
}

/// An explicit ingredient list can constrain the model before it produces any
/// tokens. Keep the order from the note and combine ingredients used twice.
pub(super) fn listed_ingredients(note: &str) -> Option<Vec<String>> {
    let mut rows = 0;
    let mut ingredients = Vec::new();
    for line in note.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if STOCK.is_match(line) {
            continue;
        }
        let Some((_, list)) = line.split_once(':') else {
            // A day or bullet without an ingredient row makes the plan partial.
            let lower = line.to_lowercase();
            if line.starts_with(['-', '•'])
                || ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
                    .iter()
                    .any(|d| lower.starts_with(d))
            {
                return None;
            }
            continue;
        };
        if !list.contains(',') {
            return None;
        }
        for term in SEPARATOR.split(list.trim_end_matches('.')) {
            let item = term.trim().to_lowercase();
            if item.is_empty()
                || item.len() > 40
                || item.split_whitespace().count() > 4
                || !item
                    .chars()
                    .all(|c| c.is_alphabetic() || matches!(c, ' ' | '-'))
            {
                return None;
            }
            if !ingredients.contains(&item) {
                ingredients.push(item);
            }
        }
        rows += 1;
    }
    (rows > 0 && ingredients.len() <= 18).then_some(ingredients)
}

pub(super) fn quantity_grammar(items: &[String]) -> String {
    // The ingredient strings come from the user's note; JSON escaping also
    // quotes literal GBNF terminals. Only counts and units are generated.
    let rows: Vec<_> = items
        .iter()
        .map(|item| {
            format!(
                "{} amount",
                serde_json::to_string(&format!("{item} — ")).unwrap()
            )
        })
        .collect();
    format!(
        "root ::= {}\n{}",
        rows.join(r#" "\n" "#),
        r#"amount ::= "1" unit? | ([2-9] | [1-4] [0-9]) units?
unit ::= " bag" | " block" | " bottle" | " box" | " bunch" | " can" | " carton" | " clove" | " cup" | " dozen" | " head" | " jar" | " loaf" | " pack" | " pint" | " pound" | " tub" | " oz" | " kg"
units ::= " bags" | " blocks" | " bottles" | " boxes" | " bunches" | " cans" | " cartons" | " cloves" | " cups" | " dozen" | " heads" | " jars" | " loaves" | " packs" | " pints" | " pounds" | " tubs" | " oz" | " kg""#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantities_follow_the_actual_named_ingredients() {
        let plan = "Dinners\nMonday soup: leeks, potatoes, broth.\nThursday gratin: potatoes, cheese.\nOn hand: broth.";
        let listed = listed_ingredients(plan).unwrap();
        assert_eq!(listed, ["leeks", "potatoes", "broth", "cheese"]);
        let needed: Vec<_> = listed
            .into_iter()
            .filter(|i| !stock(plan).contains(i))
            .collect();
        let grammar = quantity_grammar(&needed);
        assert!(grammar.contains(r#""leeks — " amount "\n" "potatoes — " amount"#));
        assert!(!grammar.contains("broth"));
        assert!(listed_ingredients(&format!("{plan}\nSaturday pizza")).is_none());
        assert!(listed_ingredients("Dinner: make a salad and serve warm.").is_none());
    }

    #[test]
    fn omits_stocked_alternatives_without_dropping_different_or_partially_stocked_items() {
        let s = stock("Chicken with rice\nAlready in the kitchen: rice, olive oil and eggs.\nOn hand: 2 limes.");
        assert!(in_stock(
            "1 package rice bowls or rice (if not already stocked)",
            &s
        ));
        assert!(in_stock("- 1 bottle of olive oil", &s));
        assert!(in_stock("1 dozen eggs", &s));
        for wanted in [
            "1 jar rice vinegar",
            "1 bag brown rice",
            "3 limes",
            "1 bottle chili oil",
        ] {
            assert!(!in_stock(wanted, &s));
        }
        assert!(stock("We still need rice and eggs.").is_empty());
    }
}
