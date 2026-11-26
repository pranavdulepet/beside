// A checkout receipt is assembled only from public fields on the pointed control's
// accessibility ancestors and the itinerary visible in the same browser window.
use super::{grams, kg, trip_date, LazyLock, NaiveDate, Regex, Value};
use std::collections::BTreeSet;
use url::Url;

pub(super) const MARKER: &str = "Checked baggage checkout";

static ROUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"^(?:Outbound flight:\s*|Flight:\s*)?([\p{L}][\p{L}\p{M} .’'/-]{0,55})\s*\(([A-Z]{3})\)\s*(?:→|->| to | – | — | - )\s*([\p{L}][\p{L}\p{M} .’'/-]{0,55})\s*\(([A-Z]{3})\)$"
).unwrap()
});
static AIRPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([\p{L}][\p{L}\p{M} .’'/-]{0,55})\s*\(([A-Z]{3})\)$").unwrap());
static TRAVELERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:passengers?:\s*|travell?ers?:\s*)?(\d+)\s*(?:adult|passenger|travell?er)s?$",
    )
    .unwrap()
});
static ADD_CONTROL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:\+|add|increase|increase quantity|(?:(?:add|increase|select) (?:a |an |one |another |extra |additional )?)?(?:[1-9][0-9]?(?:\.[0-9])?\s*(?:kg|lb)s? )?(?:(?:check[- ]in|checked|hold) )?(?:bags?|baggage|luggage)(?: quantity)?)$").unwrap()
});
static CHECKED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:checked|check[- ]in|hold) (?:bags?|baggage|luggage)\b").unwrap()
});
static LIMIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([1-9][0-9]?(?:\.[0-9])?)\s*(kg|kilograms?|lbs?|pounds?)\b").unwrap()
});
static INCLUDES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:(?:(?:your |this )?fare includes|fare allowance:|includes|included:)\s+(?:\d+|no|one|two|three)\s+(?:(?:checked|check[- ]in|hold)\s+)?bags?|(?:\d+|no|one|two|three)\s+(?:(?:checked|check[- ]in|hold)\s+)?bags?\s+included(?: (?:in (?:your|this) fare|per passenger))?)[.]?$"
).unwrap()
});
static INCLUDED_COUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(\d+|no|one|two|three)\s+(?:(?:checked|check[- ]in|hold)\s+)?bags?\b")
        .unwrap()
});
static SELECTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:(?:quantity|selected|bags? selected|selected (?:checked )?bags?)\s*:\s*([0-9])|([0-9])\s+(?:(?:checked|check[- ]in|hold)\s+)?bags? selected)$"
).unwrap()
});
static MONEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)(?:[$€£]\s*[0-9]{1,4}(?:\s*[.,]\s*[0-9]{2})?|\b(?:USD|EUR|GBP|CAD|AUD|NZD|CHF|DKK|NOK|SEK)\s+[0-9]{1,4}(?:[.,][0-9]{2})?|\b[0-9]{1,4}(?:[.,][0-9]{2})?\s+(?:USD|EUR|GBP|CAD|AUD|NZD|CHF|DKK|NOK|SEK)\b)"
).unwrap()
});

static PRICE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:(?:price|fee|per bag|each (?:bag|way)|(?:next|extra|additional|first|second|checked) (?:checked )?bag(?: (?:fee|price))?)\s*[:·–-]?\s*)?(?P<price>[$€£]\s*[0-9]{1,4}(?:\s*[.,]\s*[0-9]{2})?|(?:USD|EUR|GBP|CAD|AUD|NZD|CHF|DKK|NOK|SEK)\s+[0-9]{1,4}(?:[.,][0-9]{2})?|[0-9]{1,4}(?:[.,][0-9]{2})?\s+(?:USD|EUR|GBP|CAD|AUD|NZD|CHF|DKK|NOK|SEK))(?:\s+(?:per bag|each|per checked bag))?$").unwrap()
});
static WEIGHT_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:(?:add|select) (?:a |an |another )?)?(?:[1-9][0-9]?(?:\.[0-9])?\s*(?:kg|kilograms?|lbs?|pounds?)(?: (?:checked|check[- ]in|hold) (?:bag(?:gage)?|luggage))?|(?:(?:checked|check[- ]in|hold) (?:bag(?:gage)?|luggage)(?: (?:limit|allowance|weight limit))?|(?:maximum |weight )?limit(?: per bag)?|(?:maximum |maximum allowed )?weight(?: per (?:checked )?bag)?)\s*[:·–-]?\s*(?:up to |max(?:imum)? )?[1-9][0-9]?(?:\.[0-9])?\s*(?:kg|kilograms?|lbs?|pounds?))(?: (?:per bag|max(?:imum)?))?$").unwrap()
});

pub(super) fn allowed_price(price: &str) -> bool {
    PRICE_LINE
        .captures(price)
        .and_then(|c| c.name("price"))
        .is_some_and(|m| m.as_str() == price)
}

fn strings<'a>(value: &'a Value, max: usize) -> Option<Vec<&'a str>> {
    let lines = value.as_array()?;
    if lines.is_empty() || lines.len() > max {
        return None;
    }
    lines
        .iter()
        .map(|line| {
            line.as_str().filter(|s| {
                !s.is_empty() && s.chars().count() <= 165 && !s.contains(['\r', '\n', '\t', '@'])
            })
        })
        .collect()
}

fn unique<T: Ord>(values: BTreeSet<T>) -> Option<T> {
    (values.len() == 1)
        .then(|| values.into_iter().next())
        .flatten()
}

fn itinerary(scope: &[&str]) -> Option<(String, String, NaiveDate, String)> {
    let (mut origins, mut arrivals, mut dates, mut people) = (
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
    );
    let mut one_way = false;
    let mut proof = Vec::new();
    for &raw in scope {
        let line = raw.trim();
        let low = line.to_lowercase();
        if [
            "round trip",
            "round-trip",
            "return flight",
            "return date",
            "multi-city",
            "multi city",
        ]
        .iter()
        .any(|term| low.contains(term))
        {
            return None;
        }
        if low.contains("child") || low.contains("infant") || low.contains("teen") {
            if !["0 children", "0 infants", "0 teens"].contains(&low.as_str()) {
                return None;
            }
            continue;
        }
        if [
            "one way",
            "one-way",
            "one-way flight",
            "one way flight",
            "one-way journey",
            "one way journey",
        ]
        .contains(&low.as_str())
        {
            one_way = true;
            proof.push(line);
            continue;
        }
        if let Some(c) = TRAVELERS.captures(line) {
            people.insert(c[1].parse::<u8>().ok()?);
        } else if let Some(c) = ROUTE.captures(line) {
            origins.insert(c[2].to_owned());
            arrivals.insert((c[3].trim().to_owned(), c[4].to_owned()));
        } else if let Some((label, value)) = line.split_once(':') {
            let value = value.trim();
            match label.trim().to_lowercase().as_str() {
                "passengers" | "travelers" | "travellers" => {
                    people.insert(value.parse::<u8>().ok()?);
                }
                "from" | "departure airport" | "origin" => {
                    origins.insert(AIRPORT.captures(value)?[2].to_owned());
                }
                "to" | "arrival airport" | "destination" => {
                    let c = AIRPORT.captures(value)?;
                    arrivals.insert((c[1].trim().to_owned(), c[2].to_owned()));
                }
                "outbound" | "departure" | "departing" | "departure date" | "travel date" => {
                    dates.insert(trip_date(value)?);
                }
                _ => continue,
            }
        } else {
            continue;
        }
        proof.push(line);
    }
    let origin = unique(origins)?;
    let (name, code) = unique(arrivals)?;
    if !one_way || unique(people)? != 1 || origin == code {
        return None;
    }
    let proof = proof.join(" · ");
    (proof.len() <= 500).then_some((name, code, unique(dates)?, proof))
}

fn count(value: &str) -> Option<u32> {
    match value.to_lowercase().as_str() {
        "no" => Some(0),
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        n => n.parse().ok().filter(|n| *n <= 9),
    }
}

fn weight(value: &str, unit: &str) -> Option<i32> {
    if unit.to_ascii_lowercase().starts_with('k') {
        return grams(value);
    }
    let lb = value.parse::<f64>().ok()?;
    // Display a conservative tenth of a kilogram for a pound-only allowance.
    Some(((lb * 453.59237 / 100.).floor() as i32) * 100)
}

pub(super) fn receipt(v: &Value) -> Option<(String, String)> {
    let url = Url::parse(v["origin"].as_str()?).ok()?;
    let host = url.domain()?;
    if url.scheme() != "https"
        || !host.contains('.')
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let (name, code, day, route) = itinerary(&strings(&v["itinerary"], 48)?)?;
    let control = v["control"].as_str()?;
    if control.len() > 100 || !ADD_CONTROL.is_match(control) {
        return None;
    }
    for group in v["groups"].as_array()?.iter().take(6) {
        let lines = strings(group, 32)?;
        if !lines.iter().any(|s| CHECKED.is_match(s)) && !CHECKED.is_match(control) {
            continue;
        }
        let mut prices = BTreeSet::new();
        let mut price_positions = BTreeSet::new();
        let mut limits = BTreeSet::new();
        let mut selected = BTreeSet::new();
        let mut included = BTreeSet::new();
        let mut proof = Vec::new();
        if !v["quantity"].is_null() {
            selected.insert(u32::try_from(v["quantity"].as_u64()?).ok()?);
        }
        for &line in std::iter::once(&control).chain(lines.iter()) {
            let lower = line.to_lowercase();
            if [
                "carry-on",
                "carry on",
                "cabin bag",
                "personal item",
                "overweight",
                "oversize",
                "starting at",
                "from $",
                "from €",
                "from £",
                "per kg",
                "per pound",
                "each way",
                "round trip",
                "up to $",
                "up to €",
                "up to £",
                "per lb",
                "discount",
                "total",
                "non-refundable",
                " if ",
                "subject to",
                "membership",
                "members only",
            ]
            .iter()
            .any(|s| lower.contains(s))
            {
                return None;
            }
            let mut relevant = CHECKED.is_match(line);
            if let Some(c) = SELECTED.captures(line) {
                selected.insert(count(c.get(1).or(c.get(2))?.as_str())?);
                relevant = true;
            }
            if (lower.contains("included")
                || lower.contains("includes")
                || lower.contains("fare allowance"))
                && !INCLUDES.is_match(line)
            {
                return None;
            }
            if INCLUDES.is_match(line) {
                let c = INCLUDED_COUNT.captures(line)?;
                included.insert(count(&c[1])?);
                relevant = true;
            }
            if let Some(c) = LIMIT.captures(line) {
                if !WEIGHT_LINE.is_match(line) {
                    return None;
                }
                limits.insert(weight(&c[1], &c[2])?);
                relevant = true;
            }
            if MONEY.is_match(line) {
                let position = lower.split_whitespace().next()?;
                if ["first", "second", "extra", "additional", "next"].contains(&position) {
                    price_positions.insert(position.to_string());
                }
                let price = PRICE_LINE
                    .captures(line)?
                    .name("price")?
                    .as_str()
                    .replace(',', ".");
                let price = if price.starts_with(['£', '€', '$']) {
                    price.replace(' ', "")
                } else {
                    price
                };
                if lower.contains("from ") || lower.contains("change fee") {
                    return None;
                }
                prices.insert(price);
                relevant = true;
            }
            if relevant && !proof.contains(&line) {
                proof.push(line);
            }
        }
        if prices.len() > 1 || limits.len() > 1 || selected.len() > 1 || included.len() > 1 {
            return None;
        }
        if prices.is_empty() || limits.is_empty() || selected.is_empty() || included.is_empty() {
            continue;
        }
        let (price, limit, selected, included) = (
            unique(prices)?,
            unique(limits)?,
            unique(selected)?,
            unique(included)?,
        );
        if !(1_000..=65_000).contains(&limit) || selected > 9 {
            return None;
        }
        // "First bag" can be a different price from the next item on the
        // stepper. A bare price cannot be assigned to an extra bag when the
        // person can still select a bag within their free allowance.
        if price_positions.len() > 1
            || (price_positions.contains("first") && selected.max(included) != 0)
            || (price_positions.contains("second") && selected.max(included) != 1)
            || (included > selected
                && !["extra", "additional", "second"]
                    .iter()
                    .any(|word| price_positions.contains(*word)))
        {
            continue;
        }
        let raw = if v["quantity"].is_number() && !proof.iter().any(|line| SELECTED.is_match(line))
        {
            format!("{} · Quantity: {selected}", proof.join(" · "))
        } else {
            proof.join(" · ")
        };
        if raw.len() > 520 {
            return None;
        }
        let body=format!("{MARKER}\nDestination: {name} ({code})\nOutbound: {code} {day}\nPer bag: {} kg\nIncluded: {included}\nSelected: {selected}\nPrice: {price}\nSource: {host}\nOn screen: {raw}\nItinerary on screen: {route}",kg(limit));
        return Some((format!("Checked bags · {host}"), body));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example() -> Value {
        json!({
            "origin": "https://tickets.example.org",
            "itinerary": ["One way", "Passengers: 1", "From: Dublin (DUB)", "To: Porto (OPO)", "Outbound: 2026-12-06"],
            "control": "Add a checked bag", "quantity": 0,
            "groups": [["Checked bag up to 15 kg", "No checked bags included", "€ 29,50"]]
        })
    }

    #[test]
    fn public_price_weight_and_count_stay_on_the_same_control() {
        let (title, proof) = receipt(&example()).unwrap();
        assert_eq!(title, "Checked bags · tickets.example.org");
        assert!(proof.contains("Destination: Porto (OPO)\nOutbound: OPO 2026-12-06\nPer bag: 15 kg\nIncluded: 0\nSelected: 0\nPrice: €29.50"));
        assert!(proof.contains("No checked bags included · € 29,50 · Quantity: 0"));
        assert!(proof.contains("From: Dublin (DUB) · To: Porto (OPO) · Outbound: 2026-12-06"));
        let mut different = example();
        different["origin"] = json!("https://booking.example.net");
        different["itinerary"] = json!([
            "One-way",
            "1 adult",
            "0 children",
            "Oslo (OSL) → Toronto (YYZ)",
            "Departure date: 15 March 2027"
        ]);
        different["groups"] = json!([[
            "50 lb checked bag",
            "2 checked bags included in your fare",
            "Extra checked bag: CAD 63.40 per bag",
            "Selected: 1"
        ]]);
        different["quantity"] = Value::Null;
        let (title, proof) = receipt(&different).unwrap();
        assert_eq!(title, "Checked bags · booking.example.net");
        assert!(proof.contains("Outbound: YYZ 2027-03-15\nPer bag: 22.6 kg\nIncluded: 2\nSelected: 1\nPrice: CAD 63.40"));
        for line in [
            "€31.00",
            "1 bag selected",
            "1 bag included",
            "20 kg checked bag",
            "Carry-on included",
            "Overweight fee",
            "Only available if you change fare",
            "Bag total: €29.50",
        ] {
            let mut ambiguous = example();
            ambiguous["groups"][0]
                .as_array_mut()
                .unwrap()
                .push(json!(line));
            assert!(receipt(&ambiguous).is_none(), "{line}");
        }
        for line in [
            "from €29.50",
            "€10,000",
            "-€5.00",
            "Price €29.50–34.50",
            "Seat fee €29.50",
            "€29.50 per kg",
            "0 checked bags included",
        ] {
            let mut ambiguous = example();
            ambiguous["groups"][0][2] = json!(line);
            assert!(receipt(&ambiguous).is_none(), "{line}");
        }
        for line in [
            "Weighs 15 kg unloaded",
            "Weight of extra sports equipment: 15 kg",
            "Bag dimensions",
            "Checked bag 15 kg or 20 kg",
        ] {
            let mut other = example();
            other["groups"][0][0] = json!(line);
            assert!(receipt(&other).is_none(), "{line}");
        }
        let mut other = example();
        other["groups"] = json!([
            ["15 kg checked bag", "0 bags included"],
            ["€29.50", "0 bags included"]
        ]);
        assert!(receipt(&other).is_none());
        other = example();
        other["groups"][0][1] = json!("1 checked bag included");
        assert!(receipt(&other).is_none());
        other["groups"][0][2] = json!("Second checked bag: €34.50");
        assert!(receipt(&other).is_some());
        other["groups"][0][2] = json!("First checked bag: €14.50");
        assert!(receipt(&other).is_none());
        other = example();
        other["control"] = json!("Remove a checked bag");
        assert!(receipt(&other).is_none());
    }

    #[test]
    fn conflicting_or_incomplete_itineraries_abstain() {
        for line in [
            "2 adults",
            "1 child",
            "Destination: Rome (FCO)",
            "Date",
            "Return flight: 9 December 2026",
            "Travel date: 2026-12-12",
            "Round trip",
        ] {
            let mut v = example();
            if line == "Date" {
                v["itinerary"][4] = json!(line);
            } else {
                v["itinerary"].as_array_mut().unwrap().push(json!(line));
            }
            assert!(receipt(&v).is_none(), "{line}");
        }
        for index in 0..5 {
            let mut v = example();
            v["itinerary"].as_array_mut().unwrap().remove(index);
            assert!(receipt(&v).is_none(), "{index}");
        }
        for origin in [
            "http://example.org",
            "https://localhost",
            "https://example.org/secret",
            "https://example.org?customer=abc",
            "https://one:two@example.org",
            "https://example.org:1234",
        ] {
            let mut v = example();
            v["origin"] = json!(origin);
            assert!(receipt(&v).is_none(), "{origin}");
        }
        let mut v = example();
        v["quantity"] = Value::Null;
        assert!(receipt(&v).is_none());
    }
}
