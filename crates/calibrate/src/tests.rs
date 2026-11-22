use super::*;

/// xorshift64* with Box–Muller: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn normal(&mut self) -> f64 {
        let u = self.uniform().max(1e-300);
        let v = self.uniform();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// A judge that is overconfident by 2× and leans toward "yes" by 1 nat: it reports
/// `σ(2z + 1)` when the truth is `σ(z)`. The honest map is `a = 0.5, b = -0.5`.
fn distorted(n: usize, seed: u64) -> Vec<Sample> {
    let mut rng = Rng(seed);
    (0..n)
        .map(|_| {
            let z = 1.5 * rng.normal();
            let y = rng.uniform() < sigmoid(z);
            Sample {
                p: sigmoid(2.0 * z + 1.0),
                y,
                w: 1.0,
            }
        })
        .collect()
}

#[test]
fn worked_example_from_the_board() {
    // 0.997 → logit 5.81 → 0.6·5.81 − 0.4 → 0.956
    let q = Map::Platt { a: 0.6, b: -0.4 }.apply(0.997);
    assert!((logit(0.997) - 5.806).abs() < 1e-3);
    assert!((q - 0.956).abs() < 1e-3, "{q}");
    assert!((cutoff(1.0, 4.0) - 0.2).abs() < 1e-12);
}

#[test]
fn an_honest_judge_fits_to_the_identity() {
    let mut rng = Rng(7);
    let s: Vec<Sample> = (0..20_000)
        .map(|_| {
            let z = 1.5 * rng.normal();
            Sample {
                p: sigmoid(z),
                y: rng.uniform() < sigmoid(z),
                w: 1.0,
            }
        })
        .collect();
    let (a, b) = fit(MapKind::Platt, &s).slope_intercept();
    assert!((a - 1.0).abs() < 0.05 && b.abs() < 0.05, "a={a} b={b}");
}

#[test]
fn platt_recovers_temperature_and_chemical_potential() {
    let s = distorted(20_000, 11);
    let map = fit(MapKind::Platt, &s);
    let (a, b) = map.slope_intercept();
    assert!((a - 0.5).abs() < 0.03, "a={a}");
    assert!((b + 0.5).abs() < 0.05, "b={b}");
    let before = ece(&Map::Identity, &s, 10);
    let after = ece(&map, &s, 10);
    assert!(before > 0.05 && after < 0.015, "ece {before} → {after}");
    assert!(log_loss(&map, &s) < log_loss(&Map::Identity, &s));
}

#[test]
fn intercept_only_tracks_a_base_rate_shift() {
    // An honest judge whose traffic got 1 nat less positive (a storm passed).
    let mut rng = Rng(3);
    let s: Vec<Sample> = (0..120)
        .map(|_| {
            let z = 1.5 * rng.normal();
            Sample {
                p: sigmoid(z),
                y: rng.uniform() < sigmoid(z - 1.0),
                w: 1.0,
            }
        })
        .collect();
    assert_eq!(MapKind::for_labels(s.len()), MapKind::Intercept);
    let Map::Intercept { b } = fit(MapKind::Intercept, &s) else {
        panic!()
    };
    assert!((b + 1.0).abs() < 0.4, "b={b}");
}

#[test]
fn certainty_and_separable_labels_stay_finite() {
    let s: Vec<Sample> = [
        (1.0, true),
        (0.0, false),
        (f64::NAN, true),
        (0.9999999, true),
    ]
    .into_iter()
    .map(|(p, y)| Sample { p, y, w: 1.0 })
    .collect();
    for kind in [MapKind::Intercept, MapKind::Temperature, MapKind::Platt] {
        let (a, b) = fit(kind, &s).slope_intercept();
        assert!(
            a.is_finite() && b.is_finite() && a.abs() < 10.0,
            "{kind:?}: a={a} b={b}"
        );
    }
    assert!(log_loss(&Map::Identity, &s).is_finite());
}

#[test]
fn the_audit_coin_is_stable_and_fair() {
    assert_eq!(coin("review:acme/app#12"), coin("review:acme/app#12"));
    let hits = (0..100_000)
        .filter(|i| coin(&format!("item-{i}")) < 0.03)
        .count();
    assert!((2_700..3_300).contains(&hits), "{hits}");
}

/// The router hides what it thinks is "no", so the labels it sees are mostly "yes". The
/// unweighted base rate is badly off; the 1/observe weights put it back.
#[test]
fn audit_weights_undo_the_routers_selection() {
    let mut rng = Rng(19);
    let mut log = Vec::new();
    let mut truth = 0.0;
    let n = 40_000;
    for i in 0..n {
        let z = 1.5 * rng.normal();
        let y = rng.uniform() < sigmoid(z);
        truth += if y { 1.0 } else { 0.0 };
        let p = sigmoid(2.0 * z + 1.0);
        let id = format!("item-{i}");
        let r = route(&Map::Identity, p, 0.5, 0.05, &id);
        log.push(Judgment {
            id,
            at: i as i64,
            p,
            observe: r.observe,
            label: r.show.then_some(y),
        });
    }
    let truth = truth / n as f64;
    let s = samples(&log);
    let naive = s.iter().filter(|x| x.y).count() as f64 / s.len() as f64;
    let w: f64 = s.iter().map(|x| x.w).sum();
    let weighted = s.iter().filter(|x| x.y).map(|x| x.w).sum::<f64>() / w;
    assert!(
        (weighted - truth).abs() < 0.02,
        "weighted {weighted} vs {truth}"
    );
    assert!((naive - truth).abs() > 0.1, "naive {naive} vs {truth}");
}

#[test]
fn refit_ships_only_a_better_map() {
    let log: Vec<Judgment> = distorted(2_000, 5)
        .into_iter()
        .enumerate()
        .map(|(i, s)| Judgment {
            id: i.to_string(),
            at: i as i64,
            p: s.p,
            observe: 1.0,
            label: Some(s.y),
        })
        .collect();

    let r = refit(&log, Map::Identity, 0.2, 100);
    assert!(r.promoted, "{r:?}");
    assert_eq!(r.challenger.kind(), MapKind::Platt);
    assert!(r.challenger_loss < r.champion_loss);

    // Too few held-back labels: keep serving the champion, whatever the numbers say.
    let r = refit(&log[..60], Map::Identity, 0.2, 100);
    assert!(!r.promoted);
    assert_eq!(r.serving(), Map::Identity);
}

#[test]
fn maps_round_trip_as_json() {
    let m = Map::Platt { a: 0.6, b: -0.4 };
    let s = serde_json::to_string(&m).unwrap();
    assert_eq!(s, r#"{"kind":"platt","a":0.6,"b":-0.4}"#);
    assert_eq!(serde_json::from_str::<Map>(&s).unwrap(), m);
}
