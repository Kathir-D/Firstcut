//! `firstcut-cli run` — the headless leg of the command registry (docs/v2-plan.md §4.3):
//! chains `<command> key=value …` groups, dispatches each through the same entry point the UI
//! and the control channel use, and prints one JSON line per command. A failure stops the
//! chain: a later command depends on the state the earlier ones left.

use serde_json::{Map, Value, json};

use firstcut_engine::model::Model;
use firstcut_engine::registry::dispatch;

/// One command's outcome, in chain order.
pub struct Outcome {
    pub id: String,
    pub result: Result<Value, String>,
}

/// The CLI's `--sessions` isolation: `file.open` gets the sessionDir parameter when the
/// caller did not pass one (a test or a script points the run at a scratch directory the
/// way the engine's own tests do).
fn run_chain(sessions_dir: Option<&str>, chain: &[Step], model: &mut Model) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for (id, params) in chain {
        let params = match (id.as_str(), sessions_dir) {
            ("file.open", Some(dir)) if !params.contains_key("sessionDir") => {
                let mut params = params.clone();
                params.insert("sessionDir".to_string(), json!(dir));
                params
            }
            _ => params.clone(),
        };
        let result = dispatch(model, id, &Value::Object(params));
        let done = result.is_ok();
        outcomes.push(Outcome { id: id.clone(), result });
        if !done {
            break; // the chain's later commands stand on the earlier ones' state
        }
    }
    outcomes
}

/// One chain step: the command's id and its parameters.
type Step = (String, Map<String, Value>);

/// The token stream is groups of `<id> key=value …`: a token that carries an `=` is the
/// previous command's parameter. Values coerce the way a human types them — digits are
/// numbers, `true`/`false` is a bool, objects are raw JSON, the rest is a string.
fn parse_chain(tokens: &[String]) -> Result<Vec<Step>, String> {
    let mut chain: Vec<(String, Map<String, Value>)> = Vec::new();
    for token in tokens {
        match token.split_once('=') {
            Some((key, value)) => {
                let Some((_, params)) = chain.last_mut() else {
                    return Err(format!("run: `{key}=…` arrives before any command"));
                };
                params.insert(key.to_string(), coerce(key, value)?);
            }
            None => chain.push((token.clone(), Map::new())),
        }
    }
    if chain.is_empty() {
        return Err("run: no commands — try `firstcut run help`".to_string());
    }
    Ok(chain)
}

fn coerce(key: &str, value: &str) -> Result<Value, String> {
    match value {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        digits if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => match digits.parse::<u64>() {
            Ok(number) => Ok(json!(number)),
            Err(_) => Err(format!("run: `{key}={value}` is too large a number")),
        },
        json_like if json_like.starts_with('{') || json_like.starts_with('[') => {
            serde_json::from_str(json_like).map_err(|e| format!("run: `{key}={value}` is not JSON: {e}"))
        }
        text => Ok(json!(text)),
    }
}

pub fn cmd_run(args: &[String]) -> Result<(), String> {
    let mut index = 0;
    let mut sessions: Option<&str> = None;
    while let Some(arg) = args.get(index) {
        if arg == "--sessions" {
            sessions = Some(args.get(index + 1).ok_or("run: `--sessions` needs a directory")?);
            index += 2;
        } else if arg.as_str() == "help" {
            print_registry();
            return Ok(());
        } else if arg.starts_with("--") {
            return Err(format!("run: unknown option `{arg}`"));
        } else {
            break;
        }
    }
    let tokens = args.get(index..).ok_or("run: nothing to do")?;
    let chain = parse_chain(tokens)?;

    let mut model = Model::new();
    let outcomes = run_chain(sessions, &chain, &mut model);
    for outcome in &outcomes {
        let line = match &outcome.result {
            Ok(result) => json!({ "id": outcome.id, "result": result }),
            Err(error) => json!({ "id": outcome.id, "error": error }),
        };
        println!("{line}");
        if outcome.result.is_err() {
            return Err(format!("run: stopped after `{}` failed", outcome.id));
        }
    }
    Ok(())
}

/// `run help` prints the registry — the M1 rear of the generated consumers (§4.3).
fn print_registry() {
    for def in firstcut_engine::registry::REGISTRY {
        let params = def.params.iter().map(|(name, kind)| format!("{name}: {kind}")).collect::<Vec<_>>().join(", ");
        if params.is_empty() {
            println!("{} — {}", def.id, def.description);
        } else {
            println!("{} ({}) — {}", def.id, params, def.description);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn a_chain_parses_into_id_and_coerced_params() {
        let chain = parse_chain(&tokens(&["file.open", "path=/tmp/shoot", "rate.stars", "stars=4", "engine.state"])).unwrap();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].0, "file.open");
        assert_eq!(chain[0].1.get("path"), Some(&json!("/tmp/shoot")));
        assert_eq!(chain[1].1.get("stars"), Some(&json!(4)), "digits parse as numbers");
    }

    #[test]
    fn a_parameter_before_any_command_is_reported() {
        let error = parse_chain(&tokens(&["path=/tmp/shoot"])).unwrap_err();
        assert!(error.contains("before any command"), "{error}");
    }

    #[test]
    fn values_coerce_bool_numbers_and_strings() {
        let chain = parse_chain(&tokens(&["x", "on=true", "off=false", "count=7", "name=_Not kept"])).unwrap();
        let params = &chain[0].1;
        assert_eq!(params.get("on"), Some(&Value::Bool(true)));
        assert_eq!(params.get("off"), Some(&Value::Bool(false)));
        assert_eq!(params.get("count"), Some(&json!(7)));
        assert_eq!(params.get("name"), Some(&json!("_Not kept")));
    }

    /// The full slice end to end on a real (synthetic) shoot: open, rate, see the state
    /// through the same registry the other surfaces use.
    #[test]
    fn a_chain_opens_rates_and_reports_state() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let bytes = firstcut_meta::meta::cr3::SyntheticCr3::r8().at("2026:08:27 19:54:01").build();
        std::fs::write(folder.path().join("IMG_0001.CR3"), bytes).unwrap();

        let chain =
            parse_chain(&tokens(&["file.open", &format!("path={}", folder.path().display()), "rate.stars", "stars=4", "engine.state"])).unwrap();

        let mut model = Model::new();
        let outcomes = run_chain(sessions.path().to_str(), &chain, &mut model);
        assert_eq!(
            outcomes.len(),
            chain.len(),
            "the whole chain runs without an error: {:?}",
            outcomes.iter().filter_map(|o| o.result.as_ref().err()).collect::<Vec<_>>()
        );
        for outcome in &outcomes {
            assert!(outcome.result.is_ok(), "`{}` failed: {:?}", outcome.id, outcome.result.as_ref().err());
        }

        // The state the run left behind: one photo, rated, in the culling phase.
        let state = outcomes[2].result.as_ref().unwrap();
        assert_eq!(state.get("photoCount").and_then(Value::as_u64), Some(1));
        assert_eq!(state.get("ratedCount").and_then(Value::as_u64), Some(1), "the rating landed");
        assert_eq!(state.get("phase").and_then(Value::as_str), Some("culling"));
        let keep = state.pointer("/countsByTier/keep").and_then(Value::as_u64).unwrap_or_default();
        assert_eq!(keep, 1, "a 4-star photo under the default keep threshold is a keep");
    }

    #[test]
    fn a_failure_stops_the_chain_and_reports_it() {
        let sessions = tempfile::tempdir().unwrap();
        let chain = parse_chain(&tokens(&["file.open", &format!("path={}", "/nowhere/at/all"), "rate.stars", "stars=4"])).unwrap();

        let mut model = Model::new();
        let outcomes = run_chain(sessions.path().to_str(), &chain, &mut model);
        assert_eq!(outcomes.len(), 1, "a failed open stops the chain");
        assert!(outcomes[0].result.is_err(), "the miss is reported");
        assert_eq!(model.phase(), firstcut_engine::model::Phase::Welcome);
    }
}
