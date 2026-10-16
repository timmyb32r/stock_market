use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Kind {
    Sma,
    Ema,
    Wma,
    Rma,
    Dema,
    Tema,
    Hma,
    Zlema,
}
pub const KINDS: [Kind; 8] = [
    Kind::Sma,
    Kind::Ema,
    Kind::Wma,
    Kind::Rma,
    Kind::Dema,
    Kind::Tema,
    Kind::Hma,
    Kind::Zlema,
];
impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", format!("{self:?}").to_uppercase())
    }
}
fn sma(x: &[Option<f64>], n: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; x.len()];
    let mut sum = 0.0;
    let mut count = 0;
    for i in 0..x.len() {
        if let Some(v) = x[i] {
            sum += v;
            count += 1;
        }
        if i >= n {
            if let Some(v) = x[i - n] {
                sum -= v;
                count -= 1;
            }
        }
        if i + 1 >= n && count == n {
            out[i] = Some(sum / n as f64);
        }
    }
    out
}
fn ema(x: &[Option<f64>], n: usize, alpha: f64) -> Vec<Option<f64>> {
    let seed = sma(x, n);
    let mut prev = None;
    x.iter()
        .enumerate()
        .map(|(i, v)| {
            prev = match (prev, v) {
                (Some(p), Some(v)) => Some(alpha * v + (1.0 - alpha) * p),
                _ => seed[i],
            };
            prev
        })
        .collect()
}
fn wma(x: &[Option<f64>], n: usize) -> Vec<Option<f64>> {
    // Rolling weighted sum: newest observation gets weight n.
    let mut out = vec![None; x.len()];
    let (mut sum, mut weighted, mut count) = (0.0, 0.0, 0usize);
    for i in 0..x.len() {
        weighted -= sum;
        if let Some(v) = x[i] {
            sum += v;
            weighted += n as f64 * v;
            count += 1;
        }
        if i >= n {
            if let Some(v) = x[i - n] {
                sum -= v;
                count -= 1;
            }
        }
        if i + 1 >= n && count == n {
            out[i] = Some(weighted / (n * (n + 1) / 2) as f64);
        }
    }
    out
}
pub fn average(values: &[f64], kind: Kind, n: usize) -> Vec<Option<f64>> {
    let x: Vec<_> = values.iter().map(|v| Some(*v)).collect();
    let alpha = 2.0 / (n + 1) as f64;
    match kind {
        Kind::Sma => sma(&x, n),
        Kind::Ema => ema(&x, n, alpha),
        Kind::Rma => ema(&x, n, 1.0 / n as f64),
        Kind::Wma => wma(&x, n),
        Kind::Dema | Kind::Tema => {
            let a = ema(&x, n, alpha);
            let b = ema(&a, n, alpha);
            let c = ema(&b, n, alpha);
            (0..x.len())
                .map(|i| match kind {
                    Kind::Dema => Some(2.0 * a[i]? - b[i]?),
                    _ => Some(3.0 * a[i]? - 3.0 * b[i]? + c[i]?),
                })
                .collect()
        }
        Kind::Hma => {
            let a = wma(&x, (n / 2).max(1));
            let b = wma(&x, n);
            let d: Vec<_> = (0..x.len()).map(|i| Some(2.0 * a[i]? - b[i]?)).collect();
            wma(&d, (n as f64).sqrt().round().max(1.0) as usize)
        }
        Kind::Zlema => {
            let lag = (n - 1) / 2;
            let z: Vec<_> = (0..x.len())
                .map(|i| {
                    if i >= lag {
                        Some(2.0 * values[i] - values[i - lag])
                    } else {
                        None
                    }
                })
                .collect();
            ema(&z, n, alpha)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_averages_and_prefix_invariance() {
        let v = vec![1., 2., 3., 4., 5., 6., 7., 8., 9., 10., 11., 12.];
        assert_eq!(average(&v, Kind::Sma, 3)[2], Some(2.));
        assert!((average(&v, Kind::Wma, 3)[2].unwrap() - 14.0 / 6.0).abs() < 1e-10);
        for k in KINDS {
            assert_eq!(&average(&v, k, 3)[..9], average(&v[..9], k, 3).as_slice());
        }
    }
}
