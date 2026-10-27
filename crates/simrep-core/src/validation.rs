// Registry concept adapted from using76/Iteration-CFD rust/src/capture/registry.rs.
// Source: https://github.com/using76/Iteration-CFD/blob/main/rust/src/capture/registry.rs
// Copyright (c) 2026 Iterations Co., Ltd.
// Prosperity Public License 3.0.0; see licenses/THIRD_PARTY.md.
//! Runtime verdicts, recorded where a comparison is actually computed.
//!
//! `PASS` means `|S-D| <= u_val`, not proof that a model is correct. `MISS`
//! means the discrepancy exceeds the stated uncertainty. Missing uncertainty
//! or reference information remains `OPEN`, even if an existing test passes.
use std::{fs, io};

use crate::uncertainty::validation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Miss,
    Open,
}

pub struct Gate<'a> {
    pub name: &'a str,
    pub reference: &'a str,
    pub metric: &'a str,
    pub value: f64,
    pub datum: f64,
    /// None means not estimated, not zero.
    pub u_num: Option<f64>,
    pub u_input: f64,
    pub u_datum: f64,
    /// Description of the physical or order band; never substituted for u_num.
    pub band: &'a str,
}

impl Gate<'_> {
    pub fn verdict(&self) -> Verdict {
        match self.u_num {
            Some(u) => {
                let v = validation(self.value, self.datum, u, self.u_input, self.u_datum);
                if v.e.abs() <= v.u_val {
                    Verdict::Pass
                } else {
                    Verdict::Miss
                }
            }
            None => Verdict::Open,
        }
    }

    /// Record only when explicitly requested. Each gate owns one file, so
    /// parallel test binaries cannot mix output. A reused name is an error.
    pub fn record(&self) -> io::Result<()> {
        for value in [self.value, self.datum, self.u_input, self.u_datum] {
            assert!(value.is_finite());
        }
        assert!(self.u_input >= 0.0 && self.u_datum >= 0.0);
        assert!(self.u_num.is_none_or(|u| u.is_finite() && u >= 0.0));
        assert!(
            !self.name.is_empty()
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        );
        let Some(directory) = std::env::var_os("SIMREP_GATE_DIR") else {
            return Ok(());
        };
        fs::create_dir_all(&directory)?;
        let path = std::path::Path::new(&directory).join(format!("{}.tsv", self.name));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        use io::Write;
        let u_num = self
            .u_num
            .map_or("not estimated".into(), |u| format!("{u:.3e}"));
        let u_val = self.u_num.map_or("not estimated".into(), |u| {
            format!(
                "{:.3e}",
                validation(self.value, self.datum, u, self.u_input, self.u_datum).u_val
            )
        });
        let verdict = match self.verdict() {
            Verdict::Pass => "PASS",
            Verdict::Miss => "MISS",
            Verdict::Open => "OPEN",
        };
        for text in [self.reference, self.metric, self.band] {
            assert!(!text.contains(['\t', '\n', '|']));
        }
        writeln!(
            file,
            "{}\t{}\t{}\t{:.3e}\t{}\t{:.3e}\t{:.3e}\t{}\t{}\t{}",
            self.name,
            self.reference,
            self.metric,
            self.value,
            u_num,
            self.datum,
            self.value - self.datum,
            u_val,
            self.band,
            verdict
        )
    }
}

/// An r=2 error ladder is a code-verification order measurement, not an
/// uncertainty on the measured order. Leave its ASME verdict OPEN. The band
/// records the documented expectation and the minimum adjacent order.
pub fn record_order(name: &str, reference: &str, errors: [f64; 3], expected: f64) {
    assert!(errors.into_iter().all(|e| e.is_finite() && e > 0.0));
    let orders = [
        (errors[0] / errors[1]).log2(),
        (errors[1] / errors[2]).log2(),
    ];
    let order = (errors[0] / errors[2]).log2() / 2.0;
    println!(
        "{name}: errors={errors:?}, orders={orders:.6?}, endpoint={order:.6}, expected={expected}"
    );
    Gate {
        name,
        reference,
        metric: "endpoint order (r=2, 3 levels)",
        value: order,
        datum: expected,
        u_num: None,
        u_input: 0.0,
        u_datum: 0.0,
        band: &format!(
            "expected {expected:.1}; min adjacent {:.3}; {}",
            orders[0].min(orders[1]),
            if order >= expected - 0.1 {
                if orders.into_iter().all(|p| p >= expected - 0.1) {
                    "endpoint and adjacent orders met"
                } else {
                    "endpoint met; adjacent below documented order"
                }
            } else {
                "endpoint below documented order"
            }
        ),
    }
    .record()
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_use_uncertainty_not_the_test_tolerance() {
        let mut gate = Gate {
            name: "example",
            reference: "analytic",
            metric: "V",
            value: 1.3,
            datum: 1.0,
            u_num: None,
            u_input: 0.0,
            u_datum: 0.0,
            band: "not an uncertainty",
        };
        assert_eq!(gate.verdict(), Verdict::Open);
        gate.u_num = Some(0.2);
        assert_eq!(gate.verdict(), Verdict::Miss);
        gate.u_datum = 0.4;
        assert_eq!(gate.verdict(), Verdict::Pass);
    }
}
