//! Fear and greed readings: parsing; storing and serving are tested with store and server.
use bagholder_market::fear;
use serde_json::{json, Value};

fn cnn() -> Value {
    json!({
        "fear_and_greed": {"score": 28.6571428571429, "rating": "fear", "timestamp": "2026-09-15T23:59:51+00:00",
                           "previous_close": 31.0571428571429, "previous_1_week": 39.142857142857146,
                           "previous_1_month": 64.31428571428572, "previous_1_year": 64.45714285714287},
        "fear_and_greed_historical": {"data": [{"x": 1789516791000.0, "y": 28.6571428571429, "rating": "fear"},
                                               {"x": 1757980800000.0, "y": 64.37142857142858, "rating": "greed"}]},
        "market_momentum_sp125": {"score": 22.8, "rating": "extreme fear", "data": []},
        "market_momentum_sp500": {"score": 99.0, "rating": "extreme greed", "data": []},
        "stock_price_strength": {"score": 1, "rating": "extreme fear", "data": []},
        "stock_price_breadth": {"score": 5, "rating": "extreme fear", "data": []},
        "put_call_options": {"score": 32.2, "rating": "fear", "data": []},
        "market_volatility_vix_50": {"score": 50, "rating": "neutral", "data": []},
        "market_volatility_vix": {"score": 12, "rating": "extreme fear", "data": []},
        "junk_bond_demand": {"score": 58.6, "rating": "greed", "data": []},
        "safe_haven_demand": {"score": 31, "rating": "fear", "data": []},
    })
}
fn crypto() -> Value {
    json!({"data": [{"value": "51", "value_classification": "Neutral", "timestamp": "1789516800"},
                    {"value": "69", "value_classification": "Greed", "timestamp": "1789430400"}]})
}

#[test]
fn test_a_score_is_named_on_the_publishers_own_scale() {
    let got: Vec<String> = [0.0, 24.9, 25.0, 44.9, 45.0, 55.0, 56.0, 75.9, 76.0, 100.0].iter().map(|v| fear::band(Some(*v))).collect();
    assert_eq!(got, ["Extreme fear", "Extreme fear", "Fear", "Fear", "Neutral", "Neutral", "Greed", "Greed", "Extreme greed", "Extreme greed"]);
}

#[test]
fn test_the_publishers_own_word_is_kept_where_it_gives_one() {
    assert_eq!(fear::rating("extreme fear", Some(90.0)), "Extreme fear");
    assert_eq!(fear::rating("", Some(90.0)), "Extreme greed");
}

#[test]
fn test_the_reading_its_comparisons_its_seven_indicators_and_its_history() {
    let rec = fear::parse_stocks(&cnn()).unwrap();
    assert_eq!((rec.index.as_str(), rec.source.as_str(), rec.score, rec.rating.as_str()), ("stocks", "CNN", 28.7, "Fear"));
    assert_eq!(rec.as_of, "2026-09-15T23:59:51Z");
    let prev: Vec<(&str, f64, &str)> = rec.previous.iter().map(|r| (r.label.as_str(), r.score, r.rating.as_str())).collect();
    assert_eq!(prev, vec![("Previous close", 31.1, "Fear"), ("A week ago", 39.1, "Fear"), ("A month ago", 64.3, "Greed"), ("A year ago", 64.5, "Greed")]);
    let names: Vec<&str> = rec.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Market momentum", "Stock price strength", "Stock price breadth", "Put and call options", "Market volatility", "Junk bond demand", "Safe haven demand"]);
    assert_eq!(rec.parts[0].score, 22.8, "the 125-day momentum CNN's own page names");
    assert_eq!(rec.parts[4].score, 50.0, "and the VIX's 50-day average");
    let dates: Vec<&str> = rec.series.iter().map(|p| p.date.as_str()).collect();
    assert_eq!(dates, ["2025-09-16", "2026-09-15"], "oldest first");
}

#[test]
fn test_an_answer_with_no_score_is_no_reading() {
    assert_eq!(fear::parse_stocks(&json!({"fear_and_greed": {}})), None);
    assert_eq!(fear::parse_stocks(&Value::Null), None);
}

#[test]
fn test_the_days_own_reading_and_the_days_behind_it() {
    let rec = fear::parse_crypto(&crypto()).unwrap();
    assert_eq!((rec.index.as_str(), rec.source.as_str(), rec.score, rec.rating.as_str()), ("crypto", "Alternative.me", 51.0, "Neutral"));
    assert_eq!(rec.as_of, "2026-09-16T00:00:00Z");
    let prev: Vec<(&str, f64)> = rec.previous.iter().map(|r| (r.label.as_str(), r.score)).collect();
    assert_eq!(prev, vec![("Yesterday", 69.0)], "only the days the publisher gave");
    assert!(rec.parts.is_empty());
    let dates: Vec<&str> = rec.series.iter().map(|p| p.date.as_str()).collect();
    assert_eq!(dates, ["2026-09-15", "2026-09-16"]);
}

#[test]
fn test_an_empty_answer_is_no_reading() {
    assert_eq!(fear::parse_crypto(&json!({"data": []})), None);
}
