//! Which model should check Gramly's text: every route in
//! `bench/routes.json` against every case in `bench/cases.json`, through
//! the same prompt, diff and protection the app uses.
//!
//!     cargo run --release -p gramly-core --example bench -- [--runs 3]
//!         [--only haiku,gemini | --app] [--jobs 16] [--show haiku]
//!
//! Spends real money on the configured OpenRouter key: a few cents a run.
//!
//! What matters, in order: a wrong suggestion costs the user's trust, so
//! precision is weighed over recall (F0.5, as in grammar-correction
//! research); a correction the user waits seconds for is not seen; and the
//! same text should get the same answer every time.

use gramly_core::router::Route;
use gramly_core::{Config, Fix, ai, apply, diff, protect};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
struct Case {
    kind: String,
    text: String,
    /// Every acceptable result; the original text itself when nothing is
    /// wrong.
    want: Vec<String>,
}

/// One fix as a comparable value: where, and what it becomes.
type Edit = (usize, usize, String);

struct Outcome {
    route: usize,
    case: usize,
    result: Result<(BTreeSet<Edit>, Option<f64>), String>,
    elapsed: Duration,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let runs: usize = flag("--runs").map_or(Ok(3), |v| v.parse())?;
    let jobs: usize = flag("--jobs").map_or(Ok(16), |v| v.parse())?;
    let only = flag("--only");
    let show = flag("--show");

    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("bench");
    let cases: Vec<Case> =
        serde_json::from_str(&std::fs::read_to_string(here.join("cases.json"))?)?;
    // `--app`: only the route the app uses, whatever routes.json lists.
    let routes: Vec<Route> = if args.iter().any(|a| a == "--app") {
        vec![Route::default()]
    } else {
        serde_json::from_str(&std::fs::read_to_string(here.join("routes.json"))?)?
    };
    let routes: Vec<Route> = routes
        .into_iter()
        .filter(|r| {
            only.as_ref()
                .is_none_or(|o| o.split(',').any(|o| label(r).contains(o.trim())))
        })
        .collect();
    let key = Config::load()?
        .openrouter_key
        .ok_or_else(|| anyhow::anyhow!("no OpenRouter key in the Gramly config"))?;

    // Every call, shuffled across routes so one slow host does not hold a
    // block of the others back.
    let mut queue: Vec<(usize, usize)> = (0..routes.len())
        .flat_map(|r| (0..cases.len()).flat_map(move |c| std::iter::repeat_n((r, c), runs)))
        .collect();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for i in (1..queue.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        queue.swap(i, (seed % (i as u64 + 1)) as usize);
    }
    let total = queue.len();
    eprintln!(
        "{} routes x {} cases x {runs} runs = {total} calls",
        routes.len(),
        cases.len()
    );

    let queue = Arc::new(Mutex::new(queue));
    let (tx, rx) = mpsc::channel();
    let routes = Arc::new(routes);
    let cases = Arc::new(cases);
    for _ in 0..jobs {
        let (queue, tx, routes, cases, key) = (
            queue.clone(),
            tx.clone(),
            routes.clone(),
            cases.clone(),
            key.clone(),
        );
        std::thread::spawn(move || {
            loop {
                let Some((route, case)) = queue.lock().unwrap().pop() else {
                    return;
                };
                let text = &cases[case].text;
                let started = Instant::now();
                let result = ai::check_on(&routes[route], &key, text, "american")
                    .map(|(fixes, cost)| (edits(protect::keep(text, fixes)), cost))
                    .map_err(|e| format!("{e:#}"));
                let elapsed = started.elapsed();
                let _ = tx.send(Outcome {
                    route,
                    case,
                    result,
                    elapsed,
                });
            }
        });
    }
    drop(tx);
    let mut outcomes = Vec::with_capacity(total);
    for outcome in rx {
        outcomes.push(outcome);
        if outcomes.len() % 50 == 0 {
            eprintln!("{}/{total}", outcomes.len());
        }
    }

    let mut rows: Vec<Row> = (0..routes.len())
        .map(|r| score(r, &routes[r], &cases, &outcomes, show.as_deref()))
        .collect();
    rows.sort_by(|a, b| b.f05.total_cmp(&a.f05).then(a.p50.cmp(&b.p50)));
    println!(
        "\n{:<46} {:>6} {:>5} {:>5} {:>5} {:>6} {:>6} {:>6} {:>6} {:>4} {:>8}",
        "route", "exact", "prec", "rec", "F0.5", "wrong", "same", "p50", "p95", "fail", "$/1k"
    );
    for row in &rows {
        println!(
            "{:<46} {:>5.0}% {:>5.2} {:>5.2} {:>5.2} {:>6} {:>5.0}% {:>5.1}s {:>5.1}s {:>4} {:>8}",
            row.label,
            row.exact * 100.0,
            row.precision,
            row.recall,
            row.f05,
            row.wrong,
            row.same * 100.0,
            row.p50.as_secs_f64(),
            row.p95.as_secs_f64(),
            row.failed,
            row.cost
                .map_or("?".into(), |c| format!("{:.3}", c * 1000.0)),
        );
    }
    // For `./sandbox`, which compares runs.
    let rows_json: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "route": r.label, "exact": r.exact, "precision": r.precision,
                "recall": r.recall, "f05": r.f05, "wrong": r.wrong, "same": r.same,
                "p50": r.p50.as_secs_f64(), "p95": r.p95.as_secs_f64(),
                "failed": r.failed, "cost_per_1k": r.cost.map(|c| c * 1000.0),
                "cases": cases.len(), "runs": runs,
            })
        })
        .collect();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bench");
    std::fs::create_dir_all(&out)?;
    std::fs::write(
        out.join("last.json"),
        serde_json::to_string_pretty(&rows_json)?,
    )?;
    println!(
        "\nexact: runs whose result is one of the wanted texts. wrong: suggestions no wanted\n\
         text makes. same: cases answered identically every run. p50/p95: seconds per\n\
         paragraph. $/1k: dollars per thousand checks."
    );
    Ok(())
}

fn label(route: &Route) -> String {
    format!(
        "{} @{} {}",
        route.model,
        route.provider,
        route.reasoning.as_deref().unwrap_or("default")
    )
}

fn edits(fixes: Vec<Fix>) -> BTreeSet<Edit> {
    fixes
        .into_iter()
        .map(|f| (f.start, f.end, f.replacement))
        .collect()
}

struct Row {
    label: String,
    exact: f64,
    precision: f64,
    recall: f64,
    f05: f64,
    wrong: usize,
    same: f64,
    p50: Duration,
    p95: Duration,
    failed: usize,
    cost: Option<f64>,
}

fn score(
    index: usize,
    route: &Route,
    cases: &[Case],
    outcomes: &[Outcome],
    show: Option<&str>,
) -> Row {
    let label = label(route);
    let showing = show.is_some_and(|s| label.contains(s));
    let (mut tp, mut fp, mut fan) = (0usize, 0usize, 0usize);
    let (mut exact, mut runs, mut failed) = (0usize, 0usize, 0usize);
    let mut times = Vec::new();
    let mut costs = Vec::new();
    let mut same = 0usize;
    for (c, case) in cases.iter().enumerate() {
        let mut answers: Vec<BTreeSet<Edit>> = Vec::new();
        for outcome in outcomes.iter().filter(|o| o.route == index && o.case == c) {
            runs += 1;
            let (produced, cost) = match &outcome.result {
                Ok(ok) => ok,
                Err(why) => {
                    // No answer is no underline: every mistake in the case
                    // is missed, and the run is not exact.
                    failed += 1;
                    fan += case
                        .want
                        .iter()
                        .map(|want| diff::fixes(&case.text, want).len())
                        .min()
                        .unwrap_or(0);
                    if showing {
                        println!("[{label}] FAILED {:?}: {why}", case.text);
                    }
                    continue;
                }
            };
            times.push(outcome.elapsed);
            costs.extend(*cost);
            answers.push(produced.clone());
            // The wanted text this answer comes closest to.
            let fixes: Vec<Fix> = produced
                .iter()
                .map(|(start, end, replacement)| Fix {
                    start: *start,
                    end: *end,
                    replacement: replacement.clone(),
                    kind: gramly_core::Kind::Correctness,
                    message: String::new(),
                    source: gramly_core::Source::Ai,
                })
                .collect();
            let result = apply(&case.text, &fixes);
            let best = case
                .want
                .iter()
                .map(|want| {
                    let expected = edits(diff::fixes(&case.text, want));
                    let hit = produced.intersection(&expected).count();
                    (hit, produced.len() - hit, expected.len() - hit, want)
                })
                .max_by_key(|&(hit, extra, missed, _)| {
                    hit as isize * 2 - extra as isize * 2 - missed as isize
                })
                .expect("every case wants something");
            if case.want.contains(&result) {
                exact += 1;
            } else if showing {
                println!(
                    "[{label}] {} {:?}\n    got  {:?}\n    want {:?}",
                    case.kind, case.text, result, best.3
                );
            }
            tp += best.0;
            fp += best.1;
            fan += best.2;
        }
        // One answer (or none) is trivially consistent.
        if answers.windows(2).all(|w| w[0] == w[1]) {
            same += 1;
        }
    }
    times.sort();
    let at = |q: f64| {
        times
            .get(((times.len() as f64 * q).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or_default()
    };
    let precision = if tp + fp == 0 {
        1.0
    } else {
        tp as f64 / (tp + fp) as f64
    };
    let recall = if tp + fan == 0 {
        1.0
    } else {
        tp as f64 / (tp + fan) as f64
    };
    let f05 = if precision + recall == 0.0 {
        0.0
    } else {
        1.25 * precision * recall / (0.25 * precision + recall)
    };
    Row {
        label,
        exact: exact as f64 / runs.max(1) as f64,
        precision,
        recall,
        f05,
        wrong: fp,
        same: same as f64 / cases.len() as f64,
        p50: at(0.5),
        p95: at(0.95),
        failed,
        cost: (!costs.is_empty()).then(|| costs.iter().sum::<f64>() / costs.len() as f64),
    }
}
