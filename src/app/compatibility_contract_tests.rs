//! The contract comes from a captured Mac registry, independently of Linux's schemas.
use super::*;
use crate::app::{actions, compatibility};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn baseline() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/parity/mac-preview14.json"
    ))
    .unwrap()
}

fn matrix() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/parity/compatibility.json"
    ))
    .unwrap()
}

#[test]
fn every_captured_mac_action_has_an_honest_registration_and_permission_decision() {
    let baseline = baseline();
    assert_eq!(baseline["app"]["version"], "0.1.0 (1925)");
    assert_eq!(baseline["server"]["api_revision"], 12);
    let names: BTreeSet<_> = baseline["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 117, "capture a new version explicitly");
    let matrix = matrix();
    let rows = matrix["actions"].as_array().unwrap();
    let listed: BTreeSet<_> = rows.iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(rows.len(), listed.len(), "no duplicate classifications");
    assert_eq!(
        names, listed,
        "new Mac names must receive an explicit decision"
    );
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let spec = actions::find(name);
        assert_eq!(
            spec.is_some(),
            row["registered"].as_bool().unwrap(),
            "{name}"
        );
        let remote = match spec.map(actions::Spec::remote) {
            Some(actions::Remote::View { .. }) => "view",
            _ => "denied",
        };
        assert_eq!(row["remote"], remote, "{name}: remote policy changed");
        assert!(!row["notes"].as_str().unwrap().is_empty(), "{name}");
        match row["status"].as_str().unwrap() {
            "supported" | "partial" => assert!(spec.is_some(), "{name}"),
            "alias" => {
                let (target, args) = compatibility::alias(name).expect("shared handler");
                assert_eq!(row["target"], target);
                assert!(actions::find(target).unwrap().accepts(&args), "{name}");
            }
            "unsupported" => assert!(compatibility::unsupported(name).is_some(), "{name}"),
            "binding" => assert_eq!(name, "client.key.forward"),
            "missing" | "platform_specific" => assert!(spec.is_none(), "{name}"),
            status => panic!("unknown contract status {status}"),
        }
    }
}

fn sample(key: &str, schema: &Value) -> Value {
    if let Some(values) = schema["enum"].as_array() {
        return values[0].clone();
    }
    match schema["type"].as_str().unwrap() {
        "string" => json!(match key {
            "key" => "ctrl+u",
            "name" => "resize",
            _ => "fixture-id",
        }),
        "boolean" => json!(false),
        "integer" | "number" => json!(1),
        other => panic!("add an independent sample for {key}: {other}"),
    }
}

#[test]
fn supported_actions_accept_arguments_from_the_captured_mac_schemas() {
    let baseline = baseline();
    let matrix = matrix();
    let decisions: BTreeMap<_, _> = matrix["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["name"].as_str().unwrap(), a["status"].as_str().unwrap()))
        .collect();
    for action in baseline["actions"].as_array().unwrap() {
        let name = action["name"].as_str().unwrap();
        if !matches!(decisions[name], "supported" | "alias") {
            continue;
        }
        let spec = actions::find(name).unwrap();
        let Some(properties) = action["args"]["properties"].as_object() else {
            assert!(spec.accepts(&Value::Null), "{name}");
            assert!(spec.accepts(&json!({})), "{name}");
            continue;
        };
        let mut args = json!({});
        for key in action["args"]["required"].as_array().into_iter().flatten() {
            let key = key.as_str().unwrap();
            args[key] = sample(key, &properties[key]);
        }
        assert!(spec.accepts(&args), "{name}: {args}");
        // Check each captured enum value and optional property, not just one happy path.
        for (key, schema) in properties {
            let values = schema["enum"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![sample(key, schema)]);
            for value in values {
                let mut with = args.clone();
                with[key] = value;
                assert!(spec.accepts(&with), "{name}: {with}");
            }
        }
    }
}

#[test]
fn mac_auto_split_and_explicit_wrap_reach_the_existing_handlers_without_bad_args() {
    for direction in ["auto", "best_fit"] {
        let (mut ws, rx) = tabbed_workspace("split");
        assert!(actions::run(
            &mut ws,
            "pane.split",
            &json!({"direction":direction}),
            &mut rune::Tx::default()
        ));
        assert_eq!(requests(&rx)[0].0, "session.new_split");
    }
    for wrap in [false, true] {
        let (mut ws, rx) = tabbed_workspace("split");
        assert!(actions::run(
            &mut ws,
            "pane.focus",
            &json!({"direction":"left", "wrap":wrap}),
            &mut rune::Tx::default()
        ));
        let sent = requests(&rx);
        assert_eq!(sent[0].0, "session.focus_direction");
        assert_eq!(sent[0].2["wrap"], wrap);
    }
    let (mut ws, rx) = tabbed_workspace("split");
    for args in [
        json!({"direction":"left","wrap":1}),
        json!({"direction":"left","typo":true}),
    ] {
        assert!(!actions::run(
            &mut ws,
            "pane.focus",
            &args,
            &mut rune::Tx::default()
        ));
    }
    assert!(requests(&rx).is_empty());
    assert!(
        !actions::find("client.theme.change")
            .unwrap()
            .accepts(&json!({"name":"Midnight"})),
        "keep the documented partial contract explicit until theme-name selection is implemented"
    );
}
