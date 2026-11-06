//! How the improvement pass behaves: every case in `bench/improve.json`,
//! through the app's own prompt and filters, on the app's route (or every
//! route in `bench/routes.json` with `--all`).
//!
//!     cargo run --release -p squiggle-core --example bench_improve -- [--runs 3]
//!         [--all | --only gemini] [--show]
//!
//! Spends real money on the configured OpenRouter key: about a cent a run.
//!
//! A rewrite has no single right answer, so this does not score wording;
//! `--show` prints every proposal to read. What it counts: proposals on
//! text that reads fine ("keep" cases), which cost trust and should be
//! none; how often awkward text ("improve" cases) gets one; whether the
//! same text gets the same answer every run; latency and cost.

use serde::Deserialize;
use squiggle_core::router::Route;
use squiggle_core::{Config, apply, improve, protect};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// (route, case, the text with every proposal accepted and its cost, time).
type Outcome = (
    usize,
    usize,
    Result<(String, Option<f64>), String>,
    Duration,
);

#[derive(Debug, Deserialize)]
struct Case {
    kind: String,
    text: String,
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
    let only = flag("--only");
    let show = args.iter().any(|a| a == "--show");

    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("bench");
    let cases: Vec<Case> =
        serde_json::from_str(&std::fs::read_to_string(here.join("improve.json"))?)?;
    let routes: Vec<Route> = if args.iter().any(|a| a == "--all") || only.is_some() {
        let all: Vec<Route> =
            serde_json::from_str(&std::fs::read_to_string(here.join("routes.json"))?)?;
        all.into_iter()
            .filter(|r| only.as_ref().is_none_or(|o| r.model.contains(o.as_str())))
            .collect()
    } else {
        vec![Route::default()]
    };
    let key = Config::load()?
        .openrouter_key
        .ok_or_else(|| anyhow::anyhow!("no OpenRouter key in the Squiggle config"))?;

    let queue: Vec<(usize, usize)> = (0..routes.len())
        .flat_map(|r| (0..cases.len()).flat_map(move |c| std::iter::repeat_n((r, c), runs)))
        .collect();
    let queue = Arc::new(Mutex::new(queue));
    let (routes, cases) = (Arc::new(routes), Arc::new(cases));
    let (tx, rx) = mpsc::channel();
    for _ in 0..16 {
        let (queue, tx, routes, cases, key) = (
            queue.clone(),
            tx.clone(),
            routes.clone(),
            cases.clone(),
            key.clone(),
        );
        std::thread::spawn(move || {
            while let Some((route, case)) = queue.lock().unwrap().pop() {
                let text = &cases[case].text;
                let started = Instant::now();
                let result = improve::check_on(&routes[route], &key, text)
                    .map(|(fixes, cost)| (apply(text, &protect::keep(text, fixes)), cost))
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send((route, case, result, started.elapsed()));
            }
        });
    }
    drop(tx);
    let outcomes: Vec<Outcome> = rx.into_iter().collect();

    println!(
        "\n{:<46} {:>12} {:>9} {:>6} {:>6} {:>6} {:>4} {:>8}",
        "route", "false alarm", "offered", "same", "p50", "p95", "fail", "$/1k"
    );
    for (r, route) in routes.iter().enumerate() {
        let mine: Vec<_> = outcomes.iter().filter(|o| o.0 == r).collect();
        let (mut alarms, mut keeps, mut offered, mut improves, mut fails) = (0, 0, 0, 0, 0);
        let mut cost = 0.0;
        let mut costed = 0;
        let mut times: Vec<Duration> = mine.iter().map(|o| o.3).collect();
        times.sort();
        let mut same = 0;
        for (c, case) in cases.iter().enumerate() {
            let answers: Vec<&String> = mine
                .iter()
                .filter(|o| o.1 == c)
                .filter_map(|o| o.2.as_ref().ok().map(|(t, _)| t))
                .collect();
            if answers.windows(2).all(|w| w[0] == w[1]) && !answers.is_empty() {
                same += 1;
            }
            for answer in &answers {
                let changed = *answer != &case.text;
                if case.kind == "keep" {
                    keeps += 1;
                    alarms += changed as usize;
                } else {
                    improves += 1;
                    offered += changed as usize;
                }
                if show && changed {
                    println!(
                        "[{}] {}\n    was {:?}\n    now {:?}",
                        case.kind,
                        label(route),
                        case.text,
                        answer
                    );
                }
            }
        }
        for o in &mine {
            match &o.2 {
                Ok((_, Some(c))) => {
                    cost += c;
                    costed += 1;
                }
                Ok(_) => {}
                Err(why) => {
                    fails += 1;
                    if show {
                        println!("[{}] FAILED: {why}", label(route));
                    }
                }
            }
        }
        let pct = |n: usize, of: usize| format!("{}/{} ", n, of);
        let at = |q: f64| {
            times
                .get(((times.len() as f64 - 1.0) * q).round() as usize)
                .map_or(0.0, Duration::as_secs_f64)
        };
        println!(
            "{:<46} {:>12} {:>9} {:>5.0}% {:>5.1}s {:>5.1}s {:>4} {:>8.3}",
            label(route),
            pct(alarms, keeps),
            pct(offered, improves),
            same as f64 * 100.0 / cases.len() as f64,
            at(0.5),
            at(0.95),
            fails,
            if costed > 0 {
                cost / costed as f64 * 1000.0
            } else {
                0.0
            },
        );
    }
    println!(
        "\nfalse alarm: proposals on text that reads fine, of runs. offered: proposals on\n\
         awkward text, of runs. same: cases answered identically every run."
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
