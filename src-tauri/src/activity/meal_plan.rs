// Exact quantities belong to a meal, so changing Thursday's head count must
// not scale Tuesday's dinner. Shopping state is scoped to one encrypted chat.
use super::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    sync::LazyLock,
};

#[derive(Clone, Serialize, Deserialize)]
struct Item {
    name: String,
    unit: String,
    amount: u32,
}
#[derive(Clone, Serialize, Deserialize)]
struct Meal {
    name: String,
    serves: u32,
    items: Vec<Item>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Plan {
    meals: Vec<Meal>,
    pantry: Vec<Item>,
}

static SERVES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.+?)\s*\(?(?:serves|for)\s+([a-z]+|\d+)\s*(?:people)?\)?$").unwrap()
});
static ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(\d+|a|an|zero|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)\s+(?:(cans?|bags?|blocks?|cartons?|packs?|heads?|bunch(?:es)?|jars?|bottles?)\s+(?:of\s+)?)?([a-z][a-z -]{0,47})$").unwrap()
});
static SEPARATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r",\s*(?:and\s+)?|\s+and\s+").unwrap());
fn count(s: &str) -> Option<u32> {
    let small = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve",
    ];
    match s {
        "a" | "an" => Some(1),
        _ => s
            .parse()
            .ok()
            .or_else(|| small.iter().position(|v| *v == s).map(|v| v as u32)),
    }
}
fn singular(name: &str) -> &str {
    if name.ends_with("ss") || name.ends_with("us") {
        return name;
    }
    name.strip_suffix('s').unwrap_or(name)
}
fn items(line: &str) -> Option<Vec<Item>> {
    SEPARATE
        .split(line.trim_end_matches(['.', '!', '?']))
        .map(|part| {
            let c = ITEM.captures(part.trim())?;
            let amount = count(&c[1])?;
            if amount > 150 {
                return None;
            }
            let unit = c.get(2).map_or("", |m| m.as_str());
            let unit = if unit == "bunches" {
                "bunch"
            } else {
                unit.trim_end_matches('s')
            }
            .to_owned();
            Some(Item {
                name: c[3].trim().to_owned(),
                unit,
                amount,
            })
        })
        .collect()
}
fn key(item: &Item) -> String {
    format!("{}:{}", item.unit, singular(&item.name))
}
pub(super) fn parse(doc: &str) -> Option<Plan> {
    let mut plan = Plan {
        meals: vec![],
        pantry: vec![],
    };
    enum Section {
        Meal(usize),
        Pantry,
    }
    let mut section = None;
    for raw in doc.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let line = raw
            .trim_start_matches(['•', '-', '*'])
            .trim()
            .to_lowercase();
        let (heading, contents) = line
            .split_once(':')
            .map_or((line.as_str(), ""), |(head, body)| {
                (head.trim(), body.trim())
            });
        let ingredient_line =
            if ["on hand", "pantry", "in the pantry", "already have"].contains(&heading) {
                section = Some(Section::Pantry);
                contents
            } else if let Some(c) = SERVES.captures(heading) {
                let serves = count(&c[2])?;
                let name = c[1].trim();
                if !(1..=24).contains(&serves)
                    || plan.meals.len() >= 14
                    || plan.meals.iter().any(|m| m.name == name)
                {
                    return None;
                }
                section = Some(Section::Meal(plan.meals.len()));
                plan.meals.push(Meal {
                    name: name.into(),
                    serves,
                    items: Vec::new(),
                });
                contents
            } else if section.is_none()
                && super::shopping::meal_title(&line)
                && line.split_whitespace().count() <= 8
            {
                continue;
            } else {
                &line
            };
        if ingredient_line.is_empty() {
            continue;
        }
        let dest = match section.as_ref()? {
            Section::Meal(i) => &mut plan.meals[*i].items,
            Section::Pantry => &mut plan.pantry,
        };
        dest.extend(items(ingredient_line)?);
        if dest.len() > 60 {
            return None;
        }
    }
    (!plan.meals.is_empty() && plan.meals.iter().all(|m| !m.items.is_empty())).then_some(plan)
}

fn list(plan: &Plan, sizes: &BTreeMap<String, u32>, stock: &BTreeMap<String, u32>) -> String {
    let mut total: BTreeMap<String, (&Item, f64)> = BTreeMap::new();
    for meal in &plan.meals {
        let people = sizes.get(&meal.name).unwrap_or(&meal.serves);
        for item in &meal.items {
            total.entry(key(item)).or_insert((item, 0.)).1 +=
                f64::from(item.amount) * f64::from(*people) / f64::from(meal.serves);
        }
    }
    let mut pantry = BTreeMap::<String, u32>::new();
    for item in &plan.pantry {
        *pantry.entry(key(item)).or_default() += item.amount;
    }
    pantry.extend(stock.clone());
    let mut rows = Vec::new();
    for (key, (item, need)) in total {
        let buy = (need - 1e-9).ceil() as u32;
        let missing = buy.saturating_sub(*pantry.get(&key).unwrap_or(&0));
        if missing == 0 {
            continue;
        }
        let plural = if missing == 1 {
            ""
        } else if item.unit == "bunch" {
            "es"
        } else {
            "s"
        };
        let quantity = if item.unit.is_empty() {
            missing.to_string()
        } else {
            format!("{missing} {}{plural}", item.unit)
        };
        rows.push(format!("- {} — {quantity}", item.name));
    }
    if rows.is_empty() {
        "You already have everything for these meals.".into()
    } else {
        rows.sort();
        rows.join("\n")
    }
}

pub(super) fn start(plan: Plan, question: &str, output: &mut Value, generation: &Value) -> bool {
    let q = question.to_lowercase();
    static PEOPLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"\bfor (?:\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)\b",
        )
        .unwrap()
    });
    if PEOPLE.is_match(&q)
        || [
            "without",
            "except",
            "allerg",
            "substitut",
            "replace",
            "already",
            "budget",
            "cheaper",
            "vegetarian",
            "vegan",
            "restock",
            "double",
            "instead",
        ]
        .iter()
        .any(|s| q.contains(s))
    {
        return false;
    }
    output["answer"] = json!(list(&plan, &BTreeMap::new(), &BTreeMap::new()));
    output["workKind"] = json!("meal-plan");
    output["mealTask"] = json!({"plan":plan,"sizes":{},"stock":{},"generation":generation});
    true
}

pub(super) fn revise(s: &Store, a: &Value, question: &str, epoch: &Value) -> Res<Option<Value>> {
    let Some(task) = crate::floating_chats::last_task(s, a, epoch, "meal-plan")? else {
        return Ok(None);
    };
    let state = &task["mealTask"];
    let cfg = config(s)?;
    if state["generation"] != cfg["generation"] {
        return Ok(None);
    }
    let Ok(plan) = serde_json::from_value::<Plan>(state["plan"].clone()) else {
        return Ok(None);
    };
    if plan.meals.is_empty()
        || plan.meals.len() > 14
        || plan.pantry.len() > 180
        || plan.meals.iter().any(|m| {
            !(1..=24).contains(&m.serves)
                || m.items.is_empty()
                || m.items.len() > 60
                || m.items.iter().any(|item| item.amount > 150)
        })
        || plan.pantry.iter().any(|item| item.amount > 150)
    {
        return Ok(None);
    }
    let Ok(mut sizes) = serde_json::from_value::<BTreeMap<String, u32>>(state["sizes"].clone())
    else {
        return Ok(None);
    };
    let Ok(mut stock) = serde_json::from_value::<BTreeMap<String, u32>>(state["stock"].clone())
    else {
        return Ok(None);
    };
    if sizes.values().any(|n| !(1..=24).contains(n)) || stock.values().any(|n| *n > 150) {
        return Ok(None);
    }
    let q = question.to_lowercase();
    let q = q
        .trim()
        .trim_start_matches("actually")
        .trim_start_matches([',', ' '])
        .trim_end_matches(['.', '!', '?']);
    let pantry = ["i already have ", "i have ", "we have ", "we already have "]
        .iter()
        .find_map(|p| q.strip_prefix(p));
    static SIZE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:make |scale )?([a-z][a-z -]{1,55}?)\s+(?:is now for|is for|now serves|now for|serves|for|to)\s+([a-z]+|\d+)(?: people)?$").unwrap()
    });
    static GUESTS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:there(?: will be|'ll be|’ll be| are)|we(?:'re|’re| are) cooking for|i(?:'m|’m| am) cooking for)\s+([a-z]+|\d+)(?: people| guests| of us)?\s+(?:on |for )?([a-z][a-z -]{1,35})$").unwrap()
    });
    let size = SIZE
        .captures(q)
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .or_else(|| {
            GUESTS
                .captures(q)
                .map(|c| (c[2].to_owned(), c[1].to_owned()))
        });
    if pantry.is_none() && size.is_none() {
        return Ok(None);
    }
    if let Some(text) = pantry {
        let words: HashSet<_> = text
            .split(|c: char| !c.is_alphabetic())
            .map(singular)
            .collect();
        if !plan.meals.iter().flat_map(|m| &m.items).any(|i| {
            i.name
                .split_whitespace()
                .all(|w| words.contains(singular(w)))
        }) {
            return Ok(None);
        }
    }
    // A saved capture can remain in the journal after a newer version of the
    // same note was observed. Rebuild instead of silently buying from the old one.
    let stale = || {
        Some(
            json!({"answer":"The meal plan changed or is no longer available. Ask for a fresh list so I can use the current quantities.","sources":[],"epoch":epoch}),
        )
    };
    for origin in task["sources"].as_array().into_iter().flatten() {
        let Some(id) = origin["id"].as_str() else {
            return Ok(stale());
        };
        let row = if id.starts_with("apple-note:") {
            imported::source(s, id).ok()
        } else {
            let current:bool = s.db.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM activity a WHERE a.id=? AND NOT EXISTS(SELECT 1 FROM activity b WHERE b.source=a.source AND b.title=a.title AND b.rowid>a.rowid))",[id],|r|r.get(0)).msg()?;
            if !current {
                return Ok(stale());
            }
            source(s, id)
                .ok()
                .filter(|r| allowed(&cfg, r["source"].as_str().unwrap_or("")))
        };
        if row
            .and_then(|r| parse(r["body"].as_str()?))
            .and_then(|p| serde_json::to_value(p).ok())
            .as_ref()
            != Some(&state["plan"])
        {
            return Ok(stale());
        }
    }
    let clarification = |message: &str| {
        Some(
            json!({"answer":message,"workKind":"meal-plan","mealTask":state,"sources":task["sources"],"epoch":epoch}),
        )
    };
    if let Some(text) = pantry {
        let Some(new) = items(text) else {
            return Ok(clarification("What quantity do you have of each item? Use the unit from the list so I can subtract it."));
        };
        let keys: HashSet<_> = plan
            .meals
            .iter()
            .flat_map(|m| m.items.iter().map(key))
            .collect();
        let mut updated = HashSet::new();
        for item in new {
            let k = key(&item);
            if !keys.contains(&k) || !updated.insert(k.clone()) {
                return Ok(clarification(
                    "Which item and unit from the shopping list should I subtract?",
                ));
            }
            stock.insert(k, item.amount);
        }
    } else {
        let Some((name, amount)) = size else {
            return Ok(None);
        };
        let Some(n) = count(&amount).filter(|n| (1..=24).contains(n)) else {
            return Ok(None);
        };
        let terms: Vec<_> = name
            .split_whitespace()
            .filter(|t| !matches!(*t, "dinner" | "meal" | "the"))
            .collect();
        let matches: Vec<_> = plan
            .meals
            .iter()
            .filter(|m| {
                !terms.is_empty()
                    && terms
                        .iter()
                        .all(|t| m.name.split_whitespace().any(|w| w == *t))
            })
            .collect();
        let [meal] = matches.as_slice() else {
            return Ok(clarification(
                "Which dinner is changing, and how many people will it serve?",
            ));
        };
        sizes.insert(meal.name.clone(), n);
    }
    if &s.get("activityEpoch")? != epoch || config(s)?["generation"] != cfg["generation"] {
        return Err("Your sources changed. Ask again.".into());
    }
    Ok(Some(
        json!({"answer":list(&plan,&sizes,&stock),"workKind":"meal-plan","sources":task["sources"],"epoch":epoch,"mealTask":{"plan":plan,"sizes":sizes,"stock":stock,"generation":cfg["generation"]}}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bulleted_recipes_and_abstains_from_unmeasured_ingredients() {
        let recipe="Weeknight dinners\nMonday tacos for two\n• 1 can of beans\n• 2 limes\nPantry:\n- 1 lime";
        let plan = parse(recipe).unwrap();
        assert_eq!(plan.meals[0].serves, 2);
        assert_eq!(plan.meals[0].items.len(), 2);
        assert_eq!(plan.pantry[0].amount, 1);
        assert!(parse(&recipe.replace("2 limes", "some limes")).is_none());
        assert!(parse("Monday tacos for two:\nPantry: 1 lime").is_none());
    }

    #[test]
    fn edits_only_one_meal_subtracts_absolute_stock_and_rechecks_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::with_key(dir.path().join("book"), &[7; 32]).unwrap();
        super::super::init(&store).unwrap();
        let cfg = json!({"enabled":true,"scope":"displays","generation":"test"});
        store.set("activitySettings", &cfg).unwrap();
        let title = "Notes · Weeknight dinners";
        let menu="Weeknight dinners\nTuesday tacos (serves 2): 2 cans black beans, 4 tortillas.\nThursday bowls (serves 2): 1 can black beans, 1 lime.\nOn hand: 1 can black beans.";
        let record = |body: &str| {
            ingest(
                &store,
                &cfg,
                &json!({"source":"display","title":title,"body":body}),
                Utc::now().timestamp(),
            )
            .unwrap()
        };
        record(menu);
        let source: String = store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM activity WHERE title=?", [title], |r| {
                r.get(0)
            })
            .unwrap();
        let epoch = store.get("activityEpoch").unwrap();
        let mut turn = json!({"id":"a","question":"Make my shopping list.","sources":[{"id":source}],"epoch":epoch});
        assert!(start(
            parse(menu).unwrap(),
            "Make my shopping list.",
            &mut turn,
            &cfg["generation"]
        ));
        assert!(turn["answer"]
            .as_str()
            .unwrap()
            .contains("black beans — 2 cans"));
        let chat = "44392ae7-019d-483a-8841-21971511ee62";
        let args = json!({"chatId":chat,"previousTurn":"a","epoch":epoch});
        let save = |turn: &Value| {
            crate::floating_chats::save(
                &store,
                &json!({"chats":[{"id":chat,"epoch":epoch,"turns":[turn]}]}),
                false,
            )
            .unwrap()
        };
        save(&turn);
        assert!(revise(
            &store,
            &json!({"chatId":id(),"previousTurn":"a","epoch":epoch}),
            "Make Thursday dinner for four.",
            &epoch
        )
        .unwrap()
        .is_none());
        turn = revise(&store, &args, "There’ll be four of us on Thursday.", &epoch)
            .unwrap()
            .unwrap();
        let list = turn["answer"].as_str().unwrap();
        assert!(
            list.contains("black beans — 3 cans")
                && list.contains("tortillas — 4")
                && list.contains("lime — 2")
        );
        turn["id"] = json!("a");
        turn["question"] = json!("Make Thursday dinner for four.");
        save(&turn);
        let stock = "Actually, I have three cans of black beans.";
        turn = revise(&store, &args, stock, &epoch).unwrap().unwrap();
        assert!(turn["answer"]
            .as_str()
            .unwrap()
            .contains("black beans — 1 can"));
        turn["id"] = json!("a");
        turn["question"] = json!(stock);
        save(&turn);
        assert_eq!(
            revise(&store, &args, stock, &epoch).unwrap().unwrap()["answer"],
            turn["answer"]
        );
        assert_eq!(super::super::source(&store, &source).unwrap()["body"], menu);
        record(&menu.replace("4 tortillas", "8 tortillas"));
        assert!(
            revise(&store, &args, stock, &epoch).unwrap().unwrap()["answer"]
                .as_str()
                .unwrap()
                .contains("meal plan changed")
        );
    }
}
