// Only public labels recognized in the current browser window reach this matcher.
// Geometry is normalized to the captured window, with its origin at top left.
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Label {
    kind: String,
    text: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn only<T>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

pub(super) fn receipt(layout: &Value) -> Option<String> {
    let (x, y) = (layout["x"].as_f64()?, layout["y"].as_f64()?);
    let trip = &layout["trip"];
    if trip["tpAdults"] != "1"
        || trip["tpTeens"] != "0"
        || trip["tpChildren"] != "0"
        || trip["tpInfants"] != "0"
        || trip["tpEndDate"] != ""
    {
        return None;
    }
    let labels: Vec<Label> = serde_json::from_value(layout["labels"].clone()).ok()?;
    if labels.len() > 120
        || ![x, y].into_iter().all(|v| (0.0..=1.0).contains(&v))
        || !labels.iter().all(|l| {
            [l.x, l.y, l.w, l.h]
                .into_iter()
                .all(|v| (0.0..=1.0).contains(&v))
        })
    {
        return None;
    }
    let mut headings: Vec<_> = labels
        .iter()
        .filter(|l| l.kind == "heading" && (0.015..0.55).contains(&(y - l.y)))
        .collect();
    if headings.len() < 2 || headings.len() > 5 {
        return None;
    }
    headings.sort_by(|a, b| a.x.total_cmp(&b.x));
    let (i, head) = headings
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (a.x - x).abs().total_cmp(&(b.x - x).abs()))?;
    if headings
        .iter()
        .any(|h| (h.y - head.y).abs() > 2. * head.h.max(h.h))
    {
        return None;
    }
    // Neighbouring headings bound the actual column, even when three different
    // prices sit on the same baseline. Reject a pointer on a column boundary.
    let near = headings.get(if i == 0 { 1 } else { i - 1 })?;
    let far = headings.get(i + 1).unwrap_or(near);
    let left = head.x - (head.x - near.x).abs() / 2.;
    let right = head.x + (head.x - far.x).abs() / 2.;
    let in_column = |p: f64| p > left + 0.004 && p < right - 0.004;
    if !in_column(x) {
        return None;
    }
    let price = only(
        labels
            .iter()
            .filter(|l| l.kind == "price" && in_column(l.x) && (l.y - y).abs() < 0.08),
    )?;
    let currency = price.text.chars().next()?;
    if !['£', '$', '€'].contains(&currency) {
        return None;
    }
    let adjacent: Vec<_> = labels
        .iter()
        .filter(|l| {
            l.kind == "price" && !in_column(l.x) && (l.y - price.y).abs() < 2.0 * price.h.max(l.h)
        })
        .collect();
    if adjacent.is_empty() || adjacent.iter().any(|l| !l.text.contains(currency)) {
        return None;
    }
    let included = only(labels.iter().filter(|l| {
        l.kind == "included"
            && in_column(l.x)
            && l.y > price.y
            && l.y - price.y < 4.0 * price.h.max(l.h)
    }))?;
    let mut quantities = labels.iter().filter(|l| {
        l.kind == "quantity"
            && in_column(l.x)
            && l.x < price.x - price.w / 2.
            && price.x - l.x < 0.13
            && (l.y - price.y).abs() < 1.3 * price.h.max(l.h)
    });
    let qty = quantities.next()?;
    if !["1", "2", "3"].contains(&qty.text.as_str())
        || quantities.any(|l| l.text != qty.text || (l.x - qty.x).abs() > l.w.max(qty.w))
    {
        return None;
    }
    let route = only(
        labels
            .iter()
            .filter(|l| l.kind == "route" && l.y > head.y && l.y < price.y),
    )?;
    let (_, destination) = route.text.split_once(" to ")?;
    if destination.len() > 90 || destination.contains(['\r', '\n']) {
        return None;
    }
    let code = trip["tpDestinationIata"].as_str();
    let date = trip["tpStartDate"].as_str();
    let outbound = match (code, date) {
        (Some(code), Some(date))
            if code.len() == 3
                && code.bytes().all(|c| c.is_ascii_uppercase())
                && NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() =>
        {
            format!("{code} {date}")
        }
        (None, _) | (_, None) => "unverified".into(),
        _ => return None,
    };
    let body=format!("Check-in bag selection\nDestination: {destination}\nOutbound: {outbound}\nAdd {}\n{} {} {}",head.text,qty.text,price.text,included.text);
    super::checked_offer(&body)?;
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn label(kind: &str, text: &str, x: f64, y: f64) -> Value {
        json!({"kind":kind,"text":text,"x":x,"y":y,"h":0.018,"w":if kind=="price"{0.04}else{0.012}})
    }
    fn layout() -> Value {
        json!({"x":0.575,"y":0.519,"trip":{"tpAdults":"1","tpTeens":"0","tpChildren":"0","tpInfants":"0","tpEndDate":"","tpStartDate":"2026-12-02","tpDestinationIata":"ATH"},"labels":[
            label("heading","10kg Check-in Bag",0.416,0.337),label("heading","20kg Check-in Bag",0.579,0.335),
            label("heading","23kg Check-in Bag",0.742,0.339),label("route","London Stansted to Athens",0.26,0.465),
            label("price","Add £24.99",0.416,0.524),label("quantity","1",0.557,0.518),
            label("price","£41.49",0.606,0.52),label("included","1 bag included with Plus",0.578,0.544),
            label("price","Add £55.99",0.741,0.527),label("price","£15.00",0.78,0.678)]})
    }
    #[test]
    fn pins_the_visual_booking_and_never_reuses_it_for_an_ambiguous_pointer() {
        use crate::{activity, store::Store};
        let temp = tempfile::tempdir().unwrap();
        let store = Store::with_key(temp.path().join("book"), &[11; 32]).unwrap();
        activity::init(&store).unwrap();
        let cfg =
            json!({"enabled":true,"scope":"displays","generation":"vision","tripWeather":true});
        store.set("activitySettings", &cfg).unwrap();
        let seen = |title: &str, body: &str| {
            activity::ingest(
                &store,
                &cfg,
                &json!({"source":"display","title":title,"body":body}),
                chrono::Utc::now().timestamp(),
            )
            .unwrap()
        };
        seen(
            "Notes · Athens packing list",
            "Destination: Athens\nOutbound: 2 December 2026\nLarge suitcase\n2 hoodies",
        );
        seen(
            "Notes · January packing list",
            "Destination: Athens\nOutbound: 5 January 2027\nLarge suitcase\n6 laptops",
        );
        let mut point = json!({"source":"display","app":"Chrome","bookingTitle":"Check-in bags · Ryanair","bagLayout":layout(),"bookingControlsUnavailable":true});
        let pinned = super::super::pin(&store, &point, &cfg["generation"])
            .unwrap()
            .unwrap();
        let ask = |id: &str| {
            super::super::answer(
                &store,
                &json!({"anchor":id}),
                "Should I add another checked bag?",
                &store.get("activityEpoch").unwrap(),
            )
            .unwrap()
            .unwrap()
        };
        let answer = ask(&pinned);
        assert_eq!(answer["sources"][1]["title"], "Notes · Athens packing list");
        assert_eq!(answer["forecastTrip"]["airport"], "ATH");
        assert_eq!(answer["forecastTrip"]["date"], "2026-12-02");
        assert!(answer["answer"].as_str().unwrap().contains("£41.49"));
        point["bagLayout"]["x"] = json!(0.74);
        assert!(super::super::pin(&store, &point, &cfg["generation"])
            .unwrap()
            .is_none());
        point["bagLayout"]["x"] = json!(0.575);
        point["bagLayout"]["labels"][6]["text"] = json!("£48.99");
        let changed = super::super::pin(&store, &point, &cfg["generation"])
            .unwrap()
            .unwrap();
        assert_ne!(changed, pinned);
        assert!(ask(&changed)["answer"].as_str().unwrap().contains("£48.99"));
        assert!(!ask(&pinned)["answer"].as_str().unwrap().contains("£41.49"));
    }

    #[test]
    fn uses_only_the_pointed_control_and_its_own_checked_price() {
        let v = layout();
        let body = receipt(&v).unwrap();
        let f = super::super::checked_offer(&body).unwrap();
        assert_eq!(
            (f.destination, f.limit, f.selected, f.second.as_str()),
            ("Athens", 20000, Some(1), "£41.49")
        );
        for (index, key, value) in [
            (5, "text", json!("0")),
            (5, "x", json!(0.42)),
            (6, "text", json!("€41.49")),
            (7, "x", json!(0.74)),
            (7, "y", json!(0.76)),
            (3, "kind", json!("other")),
            (1, "y", json!(0.21)),
        ] {
            let mut wrong = v.clone();
            wrong["labels"][index][key] = value;
            assert!(receipt(&wrong).is_none(), "{index} {key}");
        }
        for extra in [
            label("price", "£5.99", 0.57, 0.53),
            label("quantity", "2", 0.558, 0.52),
            label("route", "Athens to Dublin", 0.26, 0.44),
        ] {
            let mut mixed = v.clone();
            mixed["labels"].as_array_mut().unwrap().push(extra);
            assert!(receipt(&mixed).is_none());
        }
        let mut neighbour = v.clone();
        neighbour["x"] = json!(0.74);
        assert!(receipt(&neighbour).is_none());
        let mut missing = v;
        missing["trip"]
            .as_object_mut()
            .unwrap()
            .remove("tpStartDate");
        let undated = receipt(&missing).unwrap();
        assert!(super::super::weather_trip(
            &undated,
            &super::super::checked_offer(&undated).unwrap()
        )
        .is_none());
        missing["trip"]["tpAdults"] = json!("2");
        assert!(receipt(&missing).is_none());
    }
}
