use std::time::Instant;

/// Twenty independent batch means after three warmup batches. The interval is
/// the two-sided Student-t interval for the mean of these 20 batch means.
pub fn measure(
    mut action: impl FnMut() -> Result<(), String>,
    batch: usize,
) -> Result<String, String> {
    for _ in 0..3 {
        for _ in 0..batch {
            action()?;
        }
    }
    let mut samples = Vec::with_capacity(20);
    for _ in 0..20 {
        let start = Instant::now();
        for _ in 0..batch {
            action()?;
        }
        samples.push(start.elapsed().as_secs_f64() * 1_000_000.0 / batch as f64);
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / 19.0;
    let margin = 2.093 * (variance / 20.0).sqrt();
    samples.sort_by(f64::total_cmp);
    let median = (samples[9] + samples[10]) / 2.0;
    Ok(format!(
        "mean {mean:.3} us/op; median {median:.3}; 95% CI [{:.3}, {:.3}]; 20 samples x {batch} iterations",
        (mean - margin).max(0.0),
        mean + margin
    ))
}
