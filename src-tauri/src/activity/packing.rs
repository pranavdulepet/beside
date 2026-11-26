// Deliberately broad per-item ranges in grams. The bag itself counts. An
// unrecognized line, ambiguous count or missing container invalidates the total.
use regex::Regex;
use std::sync::LazyLock;

struct Item {
    name: Regex,
    low: i32,
    high: i32,
    container: bool,
}

static ITEMS: LazyLock<Vec<Item>> = LazyLock::new(|| {
    [
        ("(?:lightweight )?daypack|small backpack", 300, 1200, true),
        ("(?:cabin |carry-on )?(?:roller|suitcase)", 1800, 4500, true),
        ("large suitcase|checked suitcase", 3000, 6000, true),
        ("t[- ]?shirts?|tees?", 100, 280, false),
        ("trousers|pants|jeans", 300, 950, false),
        ("socks", 35, 140, false),
        ("underwear|briefs|boxers", 35, 130, false),
        ("(?:light(?:weight)? )?scarf", 70, 300, false),
        ("rain (?:shell|jacket)", 150, 650, false),
        ("sweatshirts?|hoodies?", 350, 950, false),
        ("phone charger|charging cable", 30, 220, false),
        ("travel[- ]size(?:d)? toiletries", 150, 750, false),
        ("paperbacks?", 150, 600, false),
        ("laptops?", 900, 3300, false),
        ("trainers|sneakers", 450, 1200, false),
    ]
    .into_iter()
    .map(|(name, low, high, container)| Item {
        name: Regex::new(&format!("^(?:{name})$")).unwrap(),
        low,
        high,
        container,
    })
    .collect()
});
static QUANTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(\d{1,2})(?:\s*[x×]\s*|\s+))?(?:(pairs? of)\s+)?(.+)$").unwrap()
});

pub(super) fn looks_like_inventory(body: &str) -> bool {
    let mut container = false;
    let mut articles = 0;
    for raw in body.lines().take(150) {
        let line = raw
            .trim()
            .trim_start_matches(['-', '*', '•', '☐', '☑'])
            .trim()
            .to_lowercase();
        if let Some(item) = QUANTITY
            .captures(&line)
            .and_then(|c| ITEMS.iter().find(|item| item.name.is_match(&c[3])))
        {
            if item.container {
                container = true;
            } else {
                articles += 1;
            }
        }
    }
    container && articles > 0 || articles >= 3
}

pub(super) fn estimate(body: &str) -> Option<(i32, i32)> {
    let (mut low, mut high, mut bags, mut articles) = (0, 0, 0, 0);
    for (i, line) in body.trim().lines().enumerate() {
        let line = line
            .trim()
            .trim_start_matches(['-', '*', '•', '☐', '☑'])
            .trim()
            .to_lowercase();
        if line.is_empty()
            || (i == 0 && (line.ends_with("packing list") || line.ends_with("packing checklist")))
        {
            continue;
        }
        let caps = QUANTITY.captures(&line)?;
        let count = caps
            .get(1)
            .map_or(Some(1), |n| n.as_str().parse::<i32>().ok())?;
        if !(1..=50).contains(&count) {
            return None;
        }
        let name = caps.get(3)?.as_str();
        let item = ITEMS.iter().find(|item| item.name.is_match(name))?;
        if caps.get(2).is_some()
            && !matches!(
                name,
                "socks"
                    | "underwear"
                    | "briefs"
                    | "boxers"
                    | "trousers"
                    | "pants"
                    | "jeans"
                    | "trainers"
                    | "sneakers"
            )
        {
            return None;
        }
        low += count * item.low;
        high += count * item.high;
        if item.container {
            bags += count
        } else {
            articles += count
        }
    }
    (bags == 1 && articles > 0).then_some((low, high))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_every_line_and_the_container() {
        assert_eq!(
            estimate("Peru packing list\nDaypack\n• 2 x T-shirts\n4 pairs of socks"),
            Some((640, 2320))
        );
        assert_eq!(estimate("Morocco packing list\n\nDaypack\n3 T-shirts\n1 pair of trousers\n4 pairs of socks\n4 pairs of underwear\nLight scarf\nPhone charger\nTravel-size toiletries"),Some((1430,5340)));
        for s in [
            "Daypack\n3 T-shirts and a camera",
            "Daypack\n2 pairs of laptops",
            "T-shirt",
            "Daypack",
            "2 daypacks\nT-shirt",
            "Daypack\n0 socks",
            "Daypack\n55 socks",
            "Daypack\n2 T-shirts\ncamera",
        ] {
            assert_eq!(estimate(s), None, "{s}");
        }
    }
}
