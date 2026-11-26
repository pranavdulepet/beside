// Only a plainly stated, unqualified pantry item can cancel a suggested
// purchase. Inventory with a count or a qualifier still needs human judgment.
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

pub(super) fn asks_for_plan(q: &str) -> bool {
    let q = q.to_lowercase();
    static PURCHASE_REQUEST: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:what (?:should|do|can) (?:i|we) (?:get|buy|pick up|need)(?: to (?:get|buy|pick up))?|(?:make|write|build|give|put together|update|need|want)\b.{0,32}\b(?:grocer(?:y|ies)|shopping list))\b").unwrap()
    });
    let names_plan = [
        "my meal plan",
        "our meal plan",
        "my dinner plan",
        "our dinner plan",
        "my weekly menu",
        "our weekly menu",
    ]
    .iter()
    .any(|part| q.contains(part));
    if names_plan {
        return PURCHASE_REQUEST.is_match(&q)
            || q.contains("shopping list")
            || q.contains("grocer");
    }
    let planning = [
        "groceries",
        "grocery list",
        "shopping list",
        "for dinner",
        "week's dinners",
        "week’s dinners",
        "for the week",
        "this week's meals",
        "this week’s meals",
        "meal prep",
    ]
    .iter()
    .any(|part| q.contains(part));
    planning
        && PURCHASE_REQUEST.is_match(&q)
        && ![
            "how much",
            "cheapest",
            "price of",
            "open now",
            "online",
            "on sale",
            "best deal",
        ]
        .iter()
        .any(|part| q.contains(part))
}

pub(super) fn meal_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    ["meal", "dinner", "menu", "lunch", "breakfast", "grocer"]
        .iter()
        .any(|word| lower.contains(word))
}

pub(super) fn choose_plan<'a>(
    choices: &'a serde_json::Value,
    reply: &str,
) -> Option<&'a serde_json::Value> {
    let words = crate::conversation::words(reply);
    if words.is_empty() {
        return None;
    }
    let mut fits = choices.as_array()?.iter().filter(|row| {
        row["title"]
            .as_str()
            .is_some_and(|title| words.is_subset(&crate::conversation::words(title)))
    });
    let selected = fits.next()?;
    fits.next().is_none().then_some(selected)
}

/// Return independently saved, itemized plans. Different matching menus
/// need disambiguation before any quantities can be suggested.
pub(super) fn saved_plans(
    s: &crate::store::Store,
    q: &str,
) -> crate::store::Res<Vec<serde_json::Value>> {
    use super::*;
    let topics: Vec<_> = [
        "meal",
        "meals",
        "dinner",
        "dinners",
        "menu",
        "lunch",
        "breakfast",
    ]
    .map(str::to_owned)
    .into();
    let settings = config(s)?;
    let mut rows = imported::candidates_of_kind(s, &topics, &[], Some("note"))?;
    rows.extend(
        search_excluding(
            s,
            &format!("{q} {}", topics.join(" ")),
            &[],
            "",
            "",
            q,
            (true, false),
        )?
        .into_iter()
        .filter(|r| allowed(&settings, r["source"].as_str().unwrap_or(""))),
    );
    let terms = conversation::words(q);
    let mut names = HashSet::new();
    let mut matches = Vec::new();
    for row in rows {
        let title = row["title"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches("Notes · ");
        let body = row["body"].as_str().unwrap_or("");
        if !meal_title(title)
            || !names.insert(title.to_lowercase())
            || (listed_ingredients(body).is_none() && super::meal_plan::parse(body).is_none())
        {
            continue;
        }
        let score = conversation::words(title).intersection(&terms).count();
        matches.push((score, row));
    }
    matches.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let best = matches.first().map(|(score, _)| *score);
    Ok(matches
        .into_iter()
        .take_while(|(score, _)| Some(*score) == best)
        .map(|(_, r)| r)
        .collect())
}

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

/// Apply a complete, explicit inventory correction to the latest itemized
/// shopping list. Keep the quantities of every unaffected row verbatim.
pub(super) fn revision(request: &str) -> Option<Vec<String>> {
    static EDIT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i)^(?:actually[, ]+)?(?:(?:i|we) (?:already have|have enough) |(?:remove|leave out|skip) )(.+?)(?:[,.!; —]+(?:please )?(?:take (?:them|those|it) off(?: the list)?|update (?:my|the) list))?[.!]?$"
    ).unwrap()
    });
    let input = request.trim().to_lowercase();
    let capture = EDIT.captures(&input)?;
    let names: Vec<_> = SEPARATOR
        .split(&capture[1])
        .map(|n| n.trim().trim_start_matches("the ").to_owned())
        .collect();
    names
        .iter()
        .all(|n| {
            !n.is_empty()
                && n.len() <= 50
                && n.chars()
                    .all(|c| c.is_alphabetic() || matches!(c, ' ' | '-'))
        })
        .then_some(names)
}

pub(super) fn revised_list(answer: &str, remove: &[String]) -> Option<String> {
    let rows: Vec<_> = answer
        .lines()
        .map(|line| {
            line.split_once(" — ")
                .filter(|(name, qty)| !name.is_empty() && !qty.is_empty())
                .map(|(name, _)| (name.trim().to_lowercase(), line))
        })
        .collect::<Option<_>>()?;
    if !remove
        .iter()
        .all(|name| rows.iter().any(|(item, _)| item == name))
    {
        return None;
    }
    let kept: Vec<_> = rows
        .iter()
        .filter(|(name, _)| !remove.contains(name))
        .map(|(_, line)| *line)
        .collect();
    Some(if kept.is_empty() {
        "You have everything on this list.".to_owned()
    } else {
        kept.join("\n")
    })
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
    fn the_next_ask_edits_only_its_own_saved_list() {
        use crate::store::Store;
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().join("book"), &[8; 32]).unwrap();
        super::super::init(&s).unwrap();
        let epoch = s.get("activityEpoch").unwrap();
        let plan = json!({"id":"original","text":"Risotto: rice, peas. Soup: peas, broth."});
        s.put("note", &plan).unwrap();
        let first = json!({"id":"one","workKind":"shopping","epoch":epoch,"answer":"rice — 1 bag\npeas — 2 cups\nbroth — 1 carton"});
        s.put(
            "floatingChat",
            &json!({"id":"dinner","epoch":epoch,"turns":[first]}),
        )
        .unwrap();
        let args = json!({"chatId":"dinner","previousTurn":"one","epoch":epoch,"text":"Actually, I already have rice."});
        let next = super::super::ask(&s, &args).unwrap();
        assert_eq!(next["answer"], "peas — 2 cups\nbroth — 1 carton");
        assert_eq!(s.doc("note", "original").unwrap(), plan);
        let original = s.doc("floatingChat", "dinner").unwrap();
        assert_eq!(original["turns"][0], first);
    }
    #[test]
    fn revisions_preserve_portions_and_refuse_partial_matches() {
        let list = "limes — 4\nbrown rice — 1 bag\nrice vinegar — 1 bottle";
        let names = revision("Actually, I already have the limes and brown rice.").unwrap();
        assert_eq!(
            revised_list(list, &names).as_deref(),
            Some("rice vinegar — 1 bottle")
        );
        assert_eq!(
            revised_list(list, &revision("Skip rice and limes").unwrap()),
            None
        );
        assert!(revision("Do I already have limes?").is_none());
        assert!(revised_list("Plan for dinner", &names).is_none());
        assert_eq!(
            revision("I already have limes, take them off.").unwrap(),
            ["limes"]
        );
    }

    #[test]
    fn a_saved_plan_is_selected_by_its_subject_and_distinct_plans_stay_ambiguous() {
        use crate::store::Store;
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().join("notes"), &[8; 32]).unwrap();
        super::super::init(&s).unwrap();
        let note = |key: &str, title: &str, body: &str| {
            s.db.lock()
                .unwrap()
                .execute(
                    "INSERT INTO imported_passages(id,title,body,kind) VALUES(?,?,?,'note')",
                    rusqlite::params![key, title, body],
                )
                .unwrap();
        };
        note(
            "one",
            "Cabin dinners",
            "Chili: beans, tomatoes.\nOn hand: beans.",
        );
        note("two", "Birthday dinners", "Pasta: rigatoni, squash.");
        note("three", "Dinner thoughts", "I'd like to eat out.");
        let question = "Make a shopping list from my meal plan.";
        assert!(asks_for_plan(question));
        assert_eq!(saved_plans(&s, question).unwrap().len(), 2);
        assert_eq!(
            saved_plans(&s, "Use my meal plan for cabin dinners.").unwrap()[0]["id"],
            json!("one")
        );
        assert!(!asks_for_plan("Is this grocery store open?"));
        for ask in [
            "What should I pick up for this week’s meals?",
            "Make me a grocery list for the week.",
            "What do we need at the store for dinner?",
        ] {
            assert!(asks_for_plan(ask), "{ask}");
        }
        for ask in [
            "Which store is open now?",
            "How much will my groceries cost?",
            "What should I get my sister for her birthday?",
            "What should I buy online for the week?",
        ] {
            assert!(!asks_for_plan(ask), "{ask}");
        }
    }
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
