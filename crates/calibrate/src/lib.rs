//! The honesty layer: the map you put after a model you can't retrain.
//!
//! A judge (Jev, an LLM behind an API, a vendor's score) answers a yes/no question with a
//! probability. Its probabilities are only as honest as its training made them, for its
//! traffic, not yours. This crate owns the part you control: a small map from the judge's
//! probability to an honest one, fitted on labels the judge never saw, and the loop that
//! keeps it honest as traffic drifts.
//!
//! The loop, one question at a time, on one pinned judge version:
//!
//! 1. **Serve and log.** Every judgment is a [`Judgment`]: the judge's raw `p`, and the
//!    chance its label would ever be observed ([`Judgment::observe`]).
//! 2. **Label two ways.** Items the router shows get labelled by what the operator does.
//!    Items it would hide are still shown at a small audit rate ([`route`]), so the hidden
//!    region keeps producing labels. Each label is weighted by `1 / observe`.
//! 3. **Fit per question.** [`fit`] a map from the ladder ([`MapKind`]) by weighted log
//!    loss, with a weak prior at the identity: the fit starts from trusting the judge.
//! 4. **Ship only if better.** [`refit`] holds back the newest labels and promotes the
//!    challenger only if it beats the current map on them.
//! 5. **Watch.** [`ece`] and [`log_loss`] on the newest labels say when the map has gone
//!    stale.
//!
//! The physics reading of [`Map::Platt`]: `a` is an inverse temperature (how loud the
//! judge is), `b` a chemical potential (where its boundary sits for this traffic).
//!
//! Why the weights: selection on the judge's own score doesn't bias a correctly specified
//! fit (logistic regression is conditional on its input), but it does bias every
//! population number: the base rate, the ECE, the log loss the ship gate compares, and
//! the fit itself whenever the map family can't represent the true curve. The weights are
//! what let a 3% audit stand for everything you didn't review.

use serde::{Deserialize, Serialize};

/// Probabilities are clipped to `[CLIP, 1 - CLIP]` before the logit. A judge that says
/// 0.99999 would otherwise put an effectively infinite input into the fit.
pub const CLIP: f64 = 1e-4;

/// Strength of the prior pulling the map toward the identity, in pseudo-observations.
const PRIOR: f64 = 1.0;

pub fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let e = z.exp();
        e / (1.0 + e)
    }
}

/// `ln(p / (1 - p))` of `p` clipped to `[CLIP, 1 - CLIP]`. NaN reads as 0.5.
pub fn logit(p: f64) -> f64 {
    let p = if p.is_nan() {
        0.5
    } else {
        p.clamp(CLIP, 1.0 - CLIP)
    };
    (p / (1.0 - p)).ln()
}

/// A calibration map, applied to the logit of the judge's probability.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Map {
    /// Trust the judge as is.
    Identity,
    /// Only the base rate moved: `σ(logit p + b)`.
    Intercept { b: f64 },
    /// Uniformly over- or underconfident: `σ(a · logit p)`.
    Temperature { a: f64 },
    /// The default: `σ(a · logit p + b)`.
    Platt { a: f64, b: f64 },
}

impl Map {
    pub fn slope_intercept(&self) -> (f64, f64) {
        match *self {
            Map::Identity => (1.0, 0.0),
            Map::Intercept { b } => (1.0, b),
            Map::Temperature { a } => (a, 0.0),
            Map::Platt { a, b } => (a, b),
        }
    }

    pub fn kind(&self) -> MapKind {
        match self {
            Map::Identity => MapKind::Identity,
            Map::Intercept { .. } => MapKind::Intercept,
            Map::Temperature { .. } => MapKind::Temperature,
            Map::Platt { .. } => MapKind::Platt,
        }
    }

    /// The honest probability for the judge's `p`.
    pub fn apply(&self, p: f64) -> f64 {
        let (a, b) = self.slope_intercept();
        sigmoid(a * logit(p) + b)
    }
}

/// The ladder of maps, by how many parameters the labels can afford.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapKind {
    Identity,
    Intercept,
    Temperature,
    Platt,
}

impl MapKind {
    /// The default rung for `n` labels: nothing below a few dozen, the intercept until a
    /// few hundred, Platt after. Temperature is never picked by count; choose it when the
    /// reliability curve says "uniformly overconfident".
    pub fn for_labels(n: usize) -> MapKind {
        match n {
            0..30 => MapKind::Identity,
            30..300 => MapKind::Intercept,
            _ => MapKind::Platt,
        }
    }
}

/// One labelled judgment, as the fit sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// The judge's raw probability.
    pub p: f64,
    pub y: bool,
    /// `1 / P(this label was observed)`.
    pub w: f64,
}

/// Fit `kind` to `samples` by weighted log loss (Newton's method, with a weak prior at the
/// identity so separable labels can't drive the temperature to zero).
pub fn fit(kind: MapKind, samples: &[Sample]) -> Map {
    let (free_a, free_b) = match kind {
        MapKind::Identity => return Map::Identity,
        MapKind::Intercept => (false, true),
        MapKind::Temperature => (true, false),
        MapKind::Platt => (true, true),
    };
    let xs: Vec<(f64, f64, f64)> = samples
        .iter()
        .filter(|s| s.w.is_finite() && s.w > 0.0)
        .map(|s| (logit(s.p), if s.y { 1.0 } else { 0.0 }, s.w))
        .collect();
    let objective = |a: f64, b: f64| -> f64 {
        let data: f64 = xs.iter().map(|&(x, y, w)| w * nll(a * x + b, y)).sum();
        data + 0.5 * PRIOR * ((a - 1.0).powi(2) + b.powi(2))
    };

    let (mut a, mut b) = (1.0, 0.0);
    let mut f = objective(a, b);
    for _ in 0..100 {
        let (mut ga, mut gb) = (PRIOR * (a - 1.0), PRIOR * b);
        let (mut haa, mut hab, mut hbb) = (PRIOR, 0.0, PRIOR);
        for &(x, y, w) in &xs {
            let q = sigmoid(a * x + b);
            let r = w * (q - y);
            let s = w * q * (1.0 - q);
            ga += r * x;
            gb += r;
            haa += s * x * x;
            hab += s * x;
            hbb += s;
        }
        let (da, db) = match (free_a, free_b) {
            (true, true) => {
                let det = haa * hbb - hab * hab;
                ((hbb * ga - hab * gb) / det, (haa * gb - hab * ga) / det)
            }
            (true, false) => (ga / haa, 0.0),
            (false, true) => (0.0, gb / hbb),
            (false, false) => unreachable!(),
        };
        // Backtracking: Newton on log loss can overshoot from far away.
        let mut t = 1.0;
        let (na, nb, nf) = loop {
            let (na, nb) = (a - t * da, b - t * db);
            let nf = objective(na, nb);
            if nf <= f || t < 1e-8 {
                break (na, nb, nf);
            }
            t *= 0.5;
        };
        let moved = (na - a).abs().max((nb - b).abs());
        (a, b, f) = (na, nb, nf);
        if moved < 1e-10 {
            break;
        }
    }
    match kind {
        MapKind::Intercept => Map::Intercept { b },
        MapKind::Temperature => Map::Temperature { a },
        MapKind::Platt => Map::Platt { a, b },
        MapKind::Identity => Map::Identity,
    }
}

/// `-ln P(y)` for a logit `z`, without overflow.
fn nll(z: f64, y: f64) -> f64 {
    // ln(1 + e^z) - y·z
    let softplus = if z > 0.0 {
        z + (-z).exp().ln_1p()
    } else {
        z.exp().ln_1p()
    };
    softplus - y * z
}

/// Weighted mean log loss of `map` on `samples`.
pub fn log_loss(map: &Map, samples: &[Sample]) -> f64 {
    let (a, b) = map.slope_intercept();
    let (mut num, mut den) = (0.0, 0.0);
    for s in samples {
        num += s.w * nll(a * logit(s.p) + b, if s.y { 1.0 } else { 0.0 });
        den += s.w;
    }
    if den > 0.0 {
        num / den
    } else {
        f64::NAN
    }
}

/// Weighted expected calibration error of `map` on `samples`, over `bins` equal-width bins.
pub fn ece(map: &Map, samples: &[Sample], bins: usize) -> f64 {
    let bins = bins.max(1);
    let mut acc = vec![(0.0f64, 0.0f64, 0.0f64); bins]; // (Σw, Σw·q, Σw·y)
    let mut total = 0.0;
    for s in samples {
        let q = map.apply(s.p);
        let i = ((q * bins as f64) as usize).min(bins - 1);
        acc[i].0 += s.w;
        acc[i].1 += s.w * q;
        acc[i].2 += s.w * if s.y { 1.0 } else { 0.0 };
        total += s.w;
    }
    if total == 0.0 {
        return f64::NAN;
    }
    acc.iter()
        .filter(|(w, _, _)| *w > 0.0)
        .map(|(w, q, y)| (w / total) * (q / w - y / w).abs())
        .sum()
}

/// The decision threshold on an honest probability when a false positive costs `c_fp` and
/// a false negative `c_fn`. A new cost ratio is a new cutoff; it needs no new labels.
pub fn cutoff(c_fp: f64, c_fn: f64) -> f64 {
    c_fp / (c_fp + c_fn)
}

/// One judgment in the log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Judgment {
    /// The item judged, stable across restarts.
    pub id: String,
    /// When the judgment was served, epoch milliseconds.
    pub at: i64,
    /// The judge's raw probability.
    pub p: f64,
    /// The chance, fixed when the judgment was served, that its label would be observed.
    pub observe: f64,
    /// What happened, once known.
    pub label: Option<bool>,
}

/// What the router does with one item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    /// The honest probability.
    pub q: f64,
    pub show: bool,
    /// Shown only because the audit coin came up.
    pub audited: bool,
    /// `P(label observed)`, to store on the [`Judgment`].
    pub observe: f64,
}

/// Route an item: show it when the honest probability clears `cutoff`; otherwise show it
/// anyway with probability `audit`, decided by a coin seeded by the item's id so a restart
/// doesn't re-roll it.
pub fn route(map: &Map, p: f64, cutoff: f64, audit: f64, id: &str) -> Route {
    let q = map.apply(p);
    if q >= cutoff {
        return Route {
            q,
            show: true,
            audited: false,
            observe: 1.0,
        };
    }
    let audit = audit.clamp(0.0, 1.0);
    let audited = coin(id) < audit;
    Route {
        q,
        show: audited,
        audited,
        observe: audit,
    }
}

/// A uniform draw in `[0, 1)` from an id (FNV-1a, then a splitmix finalizer).
pub fn coin(id: &str) -> f64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// The labelled judgments as weighted samples. An `observe` of zero can't have produced a
/// label and is skipped.
pub fn samples(log: &[Judgment]) -> Vec<Sample> {
    log.iter()
        .filter_map(|j| {
            let y = j.label?;
            (j.observe > 0.0).then(|| Sample {
                p: j.p,
                y,
                w: 1.0 / j.observe,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Refit {
    pub champion: Map,
    pub challenger: Map,
    pub promoted: bool,
    pub n_train: usize,
    pub n_holdout: usize,
    pub champion_loss: f64,
    pub challenger_loss: f64,
    pub champion_ece: f64,
    pub challenger_ece: f64,
}

impl Refit {
    /// The map to serve after this refit.
    pub fn serving(&self) -> Map {
        if self.promoted {
            self.challenger
        } else {
            self.champion
        }
    }
}

/// Fit a challenger on all but the newest `holdout` fraction of labels, and promote it
/// only if it beats `champion` on those newest labels. Fewer than `min_holdout` held-back
/// labels never promotes.
pub fn refit(log: &[Judgment], champion: Map, holdout: f64, min_holdout: usize) -> Refit {
    let mut labelled: Vec<&Judgment> = log.iter().filter(|j| j.label.is_some()).collect();
    labelled.sort_by_key(|j| j.at);
    let n_hold = ((labelled.len() as f64) * holdout.clamp(0.0, 1.0)).round() as usize;
    let split = labelled.len() - n_hold;
    let owned: Vec<Judgment> = labelled.iter().map(|j| (*j).clone()).collect();
    let train = samples(&owned[..split]);
    let hold = samples(&owned[split..]);

    let challenger = fit(MapKind::for_labels(train.len()), &train);
    let champion_loss = log_loss(&champion, &hold);
    let challenger_loss = log_loss(&challenger, &hold);
    let promoted =
        hold.len() >= min_holdout && challenger != champion && challenger_loss < champion_loss;
    Refit {
        champion,
        challenger,
        promoted,
        n_train: train.len(),
        n_holdout: hold.len(),
        champion_loss,
        challenger_loss,
        champion_ece: ece(&champion, &hold, 10),
        challenger_ece: ece(&challenger, &hold, 10),
    }
}

#[cfg(test)]
mod tests;
